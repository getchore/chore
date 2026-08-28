//! Carrying one command out.
//!
//! Chains, redirection and the three ways a command can resolve — a task, a
//! builtin, a program on `PATH`. Everything funnels into one [`Output`], which
//! is what lets `|`, `>` and `$(...)` treat all three the same.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::process::{self, Stdio};
use std::rc::Rc;

use crate::ast::{Chain, Command, Redirect, RedirectKind, Task, Word};
use crate::error::{Error, Result};
use crate::exec::{Builtin, Ctx, Output};
use crate::{builtins, vars};

use super::memo::Claimed;
use super::{Call, Called, Dest, Flags, Flow, Frame, Interpreter, MAX_DEPTH, Mode, Repeat};

impl Interpreter<'_> {
    pub(super) fn chain(
        &mut self,
        chain: &Chain,
        dest: Dest,
        stdin: Option<&[u8]>,
        flags: Flags,
    ) -> Result<Output> {
        match chain {
            Chain::Single(cmd) => self.command(cmd, dest, stdin, flags),
            Chain::And(a, b) => {
                let left = self.chain(a, borrow(&dest), stdin, flags)?;
                if left.success() {
                    let mut right = self.chain(b, dest, None, flags)?;
                    right.stdout = concat(left.stdout, right.stdout);
                    right.stderr = concat(left.stderr, right.stderr);
                    Ok(right)
                } else {
                    Ok(left)
                }
            }
            Chain::Or(a, b) => {
                let left = self.chain(a, borrow(&dest), stdin, flags)?;
                if left.success() {
                    Ok(left)
                } else {
                    let mut right = self.chain(b, dest, None, flags)?;
                    right.stdout = concat(left.stdout, right.stdout);
                    right.stderr = concat(left.stderr, right.stderr);
                    Ok(right)
                }
            }
            Chain::Pipe(a, b) => {
                // The left side is captured whole and handed to the right as
                // stdin, rather than the two running concurrently on real OS
                // pipes. A chorefile pipe joins short, finite commands, and a
                // buffer keeps one code path for tasks, builtins and programs
                // — which is what makes `task | grep` work at all.
                let left = self.chain(a, Dest::Capture, stdin, flags)?;
                let right = self.chain(b, dest, Some(&left.stdout), flags)?;
                // As in sh, the pipeline's status is the last command's.
                Ok(right)
            }
        }
    }

    // -- one command -------------------------------------------------------

    fn command(
        &mut self,
        cmd: &Command,
        dest: Dest,
        stdin: Option<&[u8]>,
        flags: Flags,
    ) -> Result<Output> {
        let mut argv = self.expand(&cmd.name)?;
        for word in &cmd.args {
            argv.extend(self.expand(word)?);
        }
        let Some((name, rest)) = argv.split_first() else {
            return Err(Error::Run {
                message: "empty command".into(),
            });
        };
        let (name, rest) = (name.clone(), rest.to_vec());

        let (dest, stderr) = self.redirects(&cmd.redirects, dest)?;

        if flags.echo && !self.quiet {
            self.echo(&name, &rest, cmd.force_path, &cmd.redirects)?;
        }

        let mut argv = Vec::with_capacity(rest.len() + 1);
        argv.push(name.clone());
        argv.extend(rest);

        if !cmd.force_path {
            if name == "cd" {
                return self.cd(&argv);
            }
            // Like `cd`, and unlike every other builtin: its arguments are
            // task names and it has to call them, which needs the
            // interpreter itself rather than a `Ctx`. So it is resolved here
            // instead of through the table. It is still reserved in
            // `builtins::NAMES`, so `check` stops a task from taking the
            // name.
            if name == "parallel" {
                return self.parallel(&argv, dest, stderr);
            }
            if self.task(&name).is_some() {
                return self.run_task_command(&name, &argv[1..], dest, stderr);
            }
            if let Some(f) = (self.builtins)(&name) {
                return self.run_builtin(f, &argv, dest, stderr, stdin);
            }
            if builtins::is_builtin(&name) {
                return Err(Error::Run {
                    message: format!("builtin `{name}` is not available in this build"),
                });
            }
        }
        self.run_program(&argv, dest, stderr, stdin, flags)
    }

    /// Apply `>`, `>>` and `2>`. A `>` wins over the destination the caller
    /// asked for, exactly as in sh.
    fn redirects(&mut self, redirects: &[Redirect], dest: Dest) -> Result<(Dest, Option<PathBuf>)> {
        let mut dest = dest;
        let mut stderr = None;
        for r in redirects {
            let target = self.expand_to_string(&r.target)?;
            let path = self.resolve(&target);
            match r.kind {
                RedirectKind::Stdout => {
                    dest = Dest::File {
                        path,
                        append: false,
                    }
                }
                RedirectKind::StdoutAppend => dest = Dest::File { path, append: true },
                RedirectKind::Stderr => stderr = Some(path),
            }
        }
        Ok((dest, stderr))
    }

    fn resolve(&self, path: &str) -> PathBuf {
        let p = vars::to_native(path);
        if p.is_absolute() {
            p
        } else {
            self.cwd().join(p)
        }
    }

    fn echo(
        &mut self,
        name: &str,
        args: &[String],
        force_path: bool,
        redirects: &[Redirect],
    ) -> Result<()> {
        let mut line = String::from("$ ");
        if force_path {
            line.push('^');
        }
        line.push_str(&quote(name));
        for arg in args {
            line.push(' ');
            line.push_str(&quote(arg));
        }
        for r in redirects {
            let target = self.expand_to_string(&r.target)?;
            line.push_str(match r.kind {
                RedirectKind::Stdout => " > ",
                RedirectKind::StdoutAppend => " >> ",
                RedirectKind::Stderr => " 2> ",
            });
            line.push_str(&quote(&target));
        }
        writeln!(self.out, "{line}")?;
        self.out.flush()?;
        Ok(())
    }

    /// `cd` moves the interpreter, never the process, so one task's directory
    /// cannot leak into the next.
    fn cd(&mut self, argv: &[String]) -> Result<Output> {
        let target = match argv.get(1) {
            Some(arg) => self.resolve(arg),
            None => self.root.clone(),
        };
        if !target.is_dir() && self.mode == Mode::Run {
            return Err(Error::Run {
                message: format!("cd: no such directory `{}`", vars::display(&target)),
            });
        }
        self.frames.last_mut().unwrap().cwd = normalize(&target);
        Ok(Output::ok())
    }

    fn run_builtin(
        &mut self,
        f: Builtin,
        argv: &[String],
        dest: Dest,
        stderr: Option<PathBuf>,
        stdin: Option<&[u8]>,
    ) -> Result<Output> {
        let cwd = self.cwd().to_path_buf();
        let root = self.root.clone();
        let task = self.frame().task.clone();
        let dry = self.mode == Mode::Dry;
        let force = self.repeat == Repeat::Always;
        // Only a streamed command writes to the terminal; a capture or a `>`
        // hands the builtin a buffer, and progress redraws must stop there.
        // A parallel child streams into a buffer its parent prints later, so
        // there is no terminal on the other end of it either.
        let interactive =
            matches!(dest, Dest::Stream) && !self.captive && io::stdout().is_terminal();
        let mut sink = Vec::new();
        let mut diag = Vec::new();

        // A builtin writes to `ctx.out` and does not know whether it is being
        // captured; when it also fills `Output::stdout` that wins, so both
        // conventions produce the same result.
        let result = {
            let out_writer: &mut dyn Write = match dest {
                Dest::Stream => &mut *self.out,
                _ => &mut sink,
            };
            // Without `2>`, diagnostics stream even when stdout is captured —
            // sh does the same, and a silent `$(...)` would hide the reason it
            // came back empty.
            let err_writer: &mut dyn Write = match stderr {
                Some(_) => &mut diag,
                None => &mut *self.err,
            };
            let mut ctx = Ctx {
                args: argv,
                cwd: &cwd,
                root: &root,
                task: &task,
                stdin,
                dry,
                force,
                out: out_writer,
                err: err_writer,
                interactive,
            };
            f(&mut ctx)
        };

        // A hard failure *is* the builtin's diagnostic, and it must reach the
        // redirect before it unwinds: a program on `PATH` opens its `2>` file
        // before it even spawns, so the file exists whatever happens, and a
        // builtin that skipped it would catch nothing at exactly the moment
        // there was something to catch.
        let mut out = match result {
            Ok(out) => out,
            // A preview looks at the world *before* the run, so a read-only
            // builtin that fails under `--dry` is usually complaining about
            // something a skipped step would have made: `read` before the
            // `download`, `find` before the `mkdir`. Reporting it and carrying
            // on previews the rest of the recipe; unwinding would end the
            // preview at exactly the step whose successors matter. `fail` is
            // the author's own hard stop and keeps unwinding. An `Error::Io`
            // is a fault of ours rather than the command's verdict, so it
            // keeps unwinding too — the same line `try` draws.
            Err(Error::Run { message }) if self.mode == Mode::Dry && argv[0] != "fail" => {
                return Ok(self.dry_failed(&message));
            }
            Err(e) => {
                if let (Some(path), Mode::Run) = (&stderr, self.mode) {
                    if let Error::Run { message } = &e {
                        let _ = writeln!(diag, "{message}");
                    }
                    // The builtin's own error is the task's failure; a fault
                    // writing this file must not stand in for it.
                    let _ = write_file(path, &diag, false);
                }
                return Err(e);
            }
        };

        match dest {
            Dest::Stream => {}
            Dest::Capture => {
                if out.stdout.is_empty() {
                    out.stdout = sink;
                }
            }
            Dest::File { path, append } => {
                let bytes = if out.stdout.is_empty() {
                    &sink
                } else {
                    &out.stdout
                };
                if !dry {
                    write_file(&path, bytes, append)?;
                }
                // The bytes went to the file and nowhere else: leaving them in
                // `Output` would let `&&` splice them back into the caller's
                // stdout.
                out.stdout.clear();
            }
        }
        if let Some(path) = stderr {
            // Whatever the builtin wrote to `ctx.err` landed in `diag`, so the
            // file holds what a shell's `2>` would have caught.
            if self.mode == Mode::Run {
                write_file(&path, &diag, false)?;
            }
        }
        Ok(out)
    }

    fn run_program(
        &mut self,
        argv: &[String],
        dest: Dest,
        stderr: Option<PathBuf>,
        stdin: Option<&[u8]>,
        flags: Flags,
    ) -> Result<Output> {
        // `--dry` skips effects, but a command whose output or exit status is
        // consumed still runs: the preview would otherwise describe a run that
        // could never happen.
        if self.mode == Mode::Dry && !flags.needed {
            return Ok(Output::ok());
        }

        let mut command = process::Command::new(vars::to_native(&argv[0]));
        command.args(&argv[1..]).current_dir(self.cwd());

        command.stdin(match (stdin, self.captive) {
            (Some(_), _) => Stdio::piped(),
            // A parallel sibling has no console: several of them reading the
            // terminal at once would race for the same keystrokes, and a
            // prompt nobody can see is worse than an immediate EOF.
            (None, true) => Stdio::null(),
            (None, false) => Stdio::inherit(),
        });
        // A parallel child's streamed output belongs in its own buffer, in
        // one block, rather than on the terminal interleaved with its
        // siblings'. Piping it here is what makes that true of a program on
        // `PATH` as well as of a builtin.
        let streamed = matches!(dest, Dest::Stream) && self.captive;
        let capture = matches!(dest, Dest::Capture);
        match &dest {
            Dest::Stream if streamed => command.stdout(Stdio::piped()),
            Dest::Stream => command.stdout(Stdio::inherit()),
            Dest::Capture => command.stdout(Stdio::piped()),
            Dest::File { path, append } => command.stdout(open_file(path, *append)?),
        };
        match &stderr {
            Some(path) => command.stderr(open_file(path, false)?),
            None if streamed => command.stderr(Stdio::piped()),
            None => command.stderr(Stdio::inherit()),
        };

        let child = command.spawn().map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Error::Run {
                    message: format!("unknown command `{}`", argv[0]),
                }
            } else {
                Error::Io(e)
            }
        });
        let mut child = match child {
            Ok(child) => child,
            // Only a needed command reaches this line under `--dry`, and the
            // program it names may be the one a skipped `download` or
            // `extract` installs. Reporting it as a failed command keeps the
            // preview going, the same as for a builtin that cannot run.
            Err(e @ Error::Run { .. }) if self.mode == Mode::Dry => {
                let message = e.to_string();
                return Ok(self.dry_failed(&message));
            }
            Err(e) => return Err(e),
        };
        if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // A short write here means the child exited early; that is its
            // verdict to give through the exit code, not an error of ours.
            let _ = pipe.write_all(bytes);
        }
        drop(child.stdin.take());

        let mut out = Output::default();
        if capture || streamed {
            let finished = child.wait_with_output()?;
            out.code = finished.status.code().unwrap_or(1);
            out.stdout = finished.stdout;
            out.stderr = finished.stderr;
        } else {
            out.code = child.wait()?.code().unwrap_or(1);
        }
        if streamed {
            // The command was streaming: its bytes are the run's output, not
            // a value, so they go where the interpreter's output goes and
            // must not reach `&&` or a `$(...)` further out.
            self.out.write_all(&out.stdout)?;
            self.err.write_all(&out.stderr)?;
            out.stdout.clear();
            out.stderr.clear();
        }
        Ok(out)
    }

    /// A task used as a command. Its output goes wherever the caller asked,
    /// so `$(task)`, `task | grep`, `task > f` and `task 2> f` behave like any
    /// other command.
    fn run_task_command(
        &mut self,
        name: &str,
        args: &[String],
        dest: Dest,
        stderr: Option<PathBuf>,
    ) -> Result<Output> {
        // `2>` on a task diverts everything its commands report, for as long
        // as the call lasts — the same reach `>` has over what they print.
        let diag = stderr.as_ref().map(|_| {
            let buf = Rc::new(RefCell::new(Vec::new()));
            let prev = mem::replace(&mut self.err, Box::new(Shared(Rc::clone(&buf))));
            (buf, prev)
        });

        // A capture, a `>` or a pipe wants the task's *value*, not just its
        // work; `call_task` has to know, because a run-once call that already
        // happened can only answer with what it printed the first time.
        let (called, buffered) = match dest {
            Dest::Stream => (self.call_task(name, args, false), None),
            _ => {
                let buf = Rc::new(RefCell::new(Vec::new()));
                let prev = mem::replace(&mut self.out, Box::new(Shared(Rc::clone(&buf))));
                let quiet = mem::replace(&mut self.quiet, true);
                let called = self.call_task(name, args, true);
                self.out = prev;
                self.quiet = quiet;
                (called, Some(buf))
            }
        };

        // Restore stderr before any `?`: an error must not leave the
        // interpreter writing into a buffer nobody reads. The file is written
        // even when the task failed — that is when its diagnostics matter —
        // but the task's own error is the one worth reporting.
        let mut written = Ok(());
        if let (Some((buf, prev)), Some(path)) = (diag, stderr) {
            self.err = prev;
            if self.mode == Mode::Run {
                let bytes = Rc::try_unwrap(buf)
                    .map(RefCell::into_inner)
                    .unwrap_or_default();
                written = write_file(&path, &bytes, false);
            }
        }
        let call = called?;
        written?;

        let mut out = match call.called {
            Called::Done(out) => out,
            Called::Exited(code) => {
                self.pending_exit = Some(code);
                Output::failed(code)
            }
        };
        if let Some(buf) = buffered {
            let printed = Rc::try_unwrap(buf)
                .map(RefCell::into_inner)
                .unwrap_or_default();
            // A replayed run-once call brought its value with it and printed
            // nothing this time; the same convention `run_builtin` uses for a
            // builtin that fills `Output::stdout` instead of writing.
            if out.stdout.is_empty() {
                out.stdout = printed;
            }
            // Remembered before a `>` clears the bytes: the value existed,
            // and the next caller may be a `$(...)` that wants it. Keyed on
            // the arguments the task ran with — defaults filled in — because
            // that is the key `call_task` will look it up under.
            self.remember(name, &call.args, &out.stdout);
            if let Dest::File { path, append } = dest {
                if self.mode == Mode::Run {
                    write_file(&path, &out.stdout, append)?;
                }
                out.stdout.clear();
            }
        }
        // Released only now, after `remember` has had its chance to record
        // what the task printed: a parallel sibling blocked on this same task
        // then wakes to the value rather than to a bare "it ran", and reruns
        // nothing.
        drop(call.claim);
        Ok(out)
    }

    /// Record a run-once task's captured output, keyed the way `call_task`
    /// keys the run itself.
    fn remember(&mut self, name: &str, args: &[String], stdout: &[u8]) {
        if self.repeat == Repeat::Once {
            self.memo.record(&(name.to_string(), args.to_vec()), stdout);
        }
    }

    /// Call a task. `wants_value` is true when the caller is a `$(...)`, a
    /// pipe or a `>` — something that will read what the task printed.
    pub(super) fn call_task(
        &mut self,
        name: &str,
        args: &[String],
        wants_value: bool,
    ) -> Result<Call> {
        let task = self.task(name).ok_or_else(|| Error::Run {
            message: format!("unknown task `{name}`"),
        })?;
        // A parameter with a default is optional, so the arity a call must
        // meet is not the number of parameters but the number of *required*
        // ones. Anything past the last declared parameter is still accepted
        // and reaches the body through `$@`.
        let missing: Vec<&str> = task
            .params
            .iter()
            .skip(args.len())
            .filter(|p| p.required())
            .map(|p| p.name.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(Error::Run {
                message: self.arity_error(name, task, &missing),
            });
        }

        if self.frames.len() > MAX_DEPTH {
            return Err(Error::Run {
                message: format!("task `{name}` recursed more than {MAX_DEPTH} levels deep"),
            });
        }

        // The frame goes up before the defaults are evaluated, because that
        // is where a default is meant to be evaluated: in the callee's scope,
        // at the moment of the call. `$TRIPLE` in a default is the callee's
        // `$TRIPLE`, `$(...)` runs in the callee's directory, and the caller's
        // locals are not in scope — a default belongs to the task that
        // declared it, not to whoever happened to call it.
        self.frames.push(Frame {
            task: name.to_string(),
            args: args.to_vec(),
            // The callee starts where the caller stands, and its own `cd`
            // dies with the frame.
            cwd: self.cwd().to_path_buf(),
            vars: HashMap::new(),
        });
        let bound = match self.bind_defaults(task, args.len()) {
            Ok(()) => self.frame().args.clone(),
            // The frame must not outlive the failure: a default that could
            // not be evaluated leaves no half-bound call behind.
            Err(e) => {
                self.frames.pop();
                return Err(e);
            }
        };

        // Keyed on name *and* arguments: a parameterised task called with
        // different arguments has different work to do.
        //
        // The arguments are the bound ones. `deploy` and `deploy staging`,
        // where `staging` is the default, ask for exactly the same work, and
        // keying them apart would run the body — and its effects — twice for
        // one job. The price is that a repeat call evaluates the defaults it
        // is about to discard, so a `$( )` default is paid for once per call
        // rather than once per run; that is a cost, where running the body
        // twice would be a wrong answer.
        //
        // The record is shared with every `parallel` sibling, and the claim
        // is taken *before* the body runs rather than after: two siblings
        // that ask for the same task at the same instant must not both run
        // it, so the second finds the first's claim and waits for it. See
        // [`memo`](super::memo).
        let key = (name.to_string(), bound.clone());
        // Skipping the call still has to unwind the frame that was pushed
        // to bind the defaults.
        let mut skipped = None;
        let mut claim = None;
        if self.repeat == Repeat::Once {
            match self.memo.claim(&key, self.ctx, wants_value) {
                Claimed::Run(held) => claim = Some(held),
                // Run-once exists to keep a task's *effects* from happening
                // twice. A capture asks for a value, and the second asking is
                // not a second request for the work: replay what the first
                // call printed. Suppressing it and answering `Output::ok()` —
                // an empty, successful output — is what used to blank
                // `platform=$(platform-id)` on its second use, silently.
                Claimed::Replay(recorded) => {
                    skipped = Some(Called::Done(Output {
                        stdout: recorded,
                        ..Output::ok()
                    }));
                }
                // The work is done and nobody wants a value: skip it.
                Claimed::Skip => skipped = Some(Called::Done(Output::ok())),
                // Ran, but streamed to the terminal, so there is no value to
                // replay. Running it again is the only honest way to answer,
                // and it beats handing back an empty string that would be
                // interpolated into a path. A task used as a function is
                // called in `$(...)` every time and never reaches this.
                Claimed::Rerun => {}
            }
        }
        if let Some(called) = skipped {
            self.frames.pop();
            return Ok(Call {
                called,
                args: bound,
                claim: None,
            });
        }

        let flow = self.block(&task.body);
        self.frames.pop();
        // A task `--fail-fast` cut short between statements has not run, so
        // the key goes back: a later call must do the work rather than
        // believe it is already done.
        if self.aborted {
            if let Some(claim) = &mut claim {
                claim.abandon();
            }
        }

        let called = match flow? {
            Flow::Normal => Called::Done(Output::ok()),
            // `return` stops here, at the frame that raised it: the call is
            // over, the caller is not. The code is the task's status, so a
            // `return 1` reads to the caller exactly like a command that
            // exited 1 — `&&` skips, `||` takes over, `try` swallows it, and
            // outside those the caller stops fail-fast. `exit` is the other
            // half of this match precisely because it does *not* stop here.
            Flow::Return(code) => Called::Done(Output::failed(code)),
            Flow::Exit(code) => Called::Exited(code),
        };
        Ok(Call {
            called,
            args: bound,
            claim,
        })
    }

    /// Fill in the parameters the caller left off, in declaration order.
    ///
    /// Two rules meet here, and both are deliberate.
    ///
    /// A default is evaluated **only when it is used**: the loop starts at the
    /// first parameter the caller did not supply, so `task fetch url=$(read
    /// .env)` called with an explicit url never reads the file. A default that
    /// costs something — a capture, a network probe — is then a fallback and
    /// not a toll every call pays.
    ///
    /// A default is evaluated **left to right, into the frame it is filling**,
    /// so an earlier parameter is already bound and visible to a later one's
    /// default: `task t a b=$1` binds `b` to `a`. Parameters live in the frame
    /// as `$1`, `$2`, ..., and by the time the default for `$2` is evaluated,
    /// `$1` is there — there is no order in which the reverse could work, and
    /// forbidding it would only forbid the useful direction.
    ///
    /// The value is expanded to exactly one string, the way an assignment's
    /// right-hand side is. A default fills one parameter slot; letting a
    /// default that expands to two words spill into the next slot would make
    /// `$2` mean something different depending on what `$1` happened to
    /// contain. A caller's argument still splits at the call site, because
    /// there the word is an argument and splitting is what argument words do.
    fn bind_defaults(&mut self, task: &Task, given: usize) -> Result<()> {
        for param in task.params.iter().skip(given) {
            // The arity check above rejected any required parameter this loop
            // would reach, so every one of these has a default.
            let Some(default) = &param.default else {
                continue;
            };
            let value = self.expand_to_string(default)?;
            self.frames.last_mut().unwrap().args.push(value);
        }
        Ok(())
    }

    /// The message for a call that left a required argument off.
    ///
    /// A task with an optional parameter has no single number of arguments it
    /// "takes", so the message names what is missing and shows the shape of a
    /// complete call — `<required>` and `[optional=default]` — rather than
    /// asking the caller to work the count out.
    fn arity_error(&self, name: &str, task: &Task, missing: &[&str]) -> String {
        let missing = missing
            .iter()
            .map(|m| format!("`{m}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let usage = task
            .params
            .iter()
            .map(|p| match &p.default {
                Some(d) => format!("[{}={}]", p.name, self.preview(d)),
                None => format!("<{}>", p.name),
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!("task `{name}` is missing required argument(s) {missing} (usage: {name} {usage})")
    }
}

fn concat(mut a: Vec<u8>, b: Vec<u8>) -> Vec<u8> {
    a.extend(b);
    a
}

/// Both sides of `&&` and `||` write to the same place, so the destination is
/// shared rather than moved.
fn borrow(dest: &Dest) -> Dest {
    match dest {
        Dest::Stream => Dest::Stream,
        Dest::Capture => Dest::Capture,
        // The second write appends: the first already truncated the file.
        Dest::File { path, .. } => Dest::File {
            path: path.clone(),
            append: true,
        },
    }
}

fn open_file(path: &Path, append: bool) -> Result<fs::File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .open(path)?;
    Ok(file)
}

pub(super) fn write_file(path: &Path, bytes: &[u8], append: bool) -> Result<()> {
    let mut file = open_file(path, append)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Echo an argument the way it would have to be written to mean the same
/// thing again — argv itself is never re-quoted, only this preview.
fn quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains(char::is_whitespace) || arg.contains('"') {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

/// Render a chain for an error message.
pub(super) fn describe(chain: &Chain, word: &mut dyn FnMut(&Word) -> String) -> String {
    match chain {
        Chain::Single(cmd) => {
            let mut s = word(&cmd.name);
            for arg in &cmd.args {
                s.push(' ');
                s.push_str(&word(arg));
            }
            s
        }
        Chain::And(a, b) => format!("{} && {}", describe(a, word), describe(b, word)),
        Chain::Or(a, b) => format!("{} || {}", describe(a, word), describe(b, word)),
        Chain::Pipe(a, b) => format!("{} | {}", describe(a, word), describe(b, word)),
    }
}

/// Resolve `.` and `..` without touching the filesystem: `cd` is bookkeeping,
/// and `canonicalize` would resolve symlinks the user wrote deliberately.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// A writer that hands its bytes back, for capturing a task's own output.
pub(super) struct Shared(Rc<RefCell<Vec<u8>>>);

impl Shared {
    pub(super) fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    /// A second handle on the same bytes, to hand the interpreter as its
    /// `out` or `err` while this one stays behind to read them.
    pub(super) fn writer(&self) -> Box<dyn Write> {
        Box::new(Self(Rc::clone(&self.0)))
    }

    pub(super) fn take(&self) -> Vec<u8> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
