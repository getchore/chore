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

use crate::ast::{Chain, Command, Redirect, RedirectKind, Word};
use crate::error::{Error, Result};
use crate::exec::{Builtin, Ctx, Output};
use crate::{builtins, vars};

use super::{Called, Dest, Flags, Flow, Frame, Interpreter, MAX_DEPTH, Mode, Repeat};

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
        let dry = self.mode == Mode::Dry;
        // Only a streamed command writes to the terminal; a capture or a `>`
        // hands the builtin a buffer, and progress redraws must stop there.
        let interactive = matches!(dest, Dest::Stream) && io::stdout().is_terminal();
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
                stdin,
                dry,
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

        command.stdin(match stdin {
            Some(_) => Stdio::piped(),
            None => Stdio::inherit(),
        });
        let capture = matches!(dest, Dest::Capture);
        match &dest {
            Dest::Stream => command.stdout(Stdio::inherit()),
            Dest::Capture => command.stdout(Stdio::piped()),
            Dest::File { path, append } => command.stdout(open_file(path, *append)?),
        };
        match &stderr {
            Some(path) => command.stderr(open_file(path, false)?),
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
        if capture {
            let finished = child.wait_with_output()?;
            out.code = finished.status.code().unwrap_or(1);
            out.stdout = finished.stdout;
            out.stderr = finished.stderr;
        } else {
            out.code = child.wait()?.code().unwrap_or(1);
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
        let called = called?;
        written?;

        let mut out = match called {
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
            // and the next caller may be a `$(...)` that wants it.
            self.remember(name, args, &out.stdout);
            if let Dest::File { path, append } = dest {
                if self.mode == Mode::Run {
                    write_file(&path, &out.stdout, append)?;
                }
                out.stdout.clear();
            }
        }
        Ok(out)
    }

    /// Record a run-once task's captured output, keyed the way `call_task`
    /// keys the run itself.
    fn remember(&mut self, name: &str, args: &[String], stdout: &[u8]) {
        if self.repeat == Repeat::Once {
            self.ran
                .insert((name.to_string(), args.to_vec()), Some(stdout.to_vec()));
        }
    }

    /// Call a task. `wants_value` is true when the caller is a `$(...)`, a
    /// pipe or a `>` — something that will read what the task printed.
    pub(super) fn call_task(
        &mut self,
        name: &str,
        args: &[String],
        wants_value: bool,
    ) -> Result<Called> {
        let task = self.task(name).ok_or_else(|| Error::Run {
            message: format!("unknown task `{name}`"),
        })?;
        if args.len() < task.params.len() {
            return Err(Error::Run {
                message: format!(
                    "task `{name}` takes {} argument(s) ({}), got {}",
                    task.params.len(),
                    task.params.join(", "),
                    args.len()
                ),
            });
        }

        // Keyed on name *and* arguments: a parameterised task called with
        // different arguments has different work to do.
        let key = (name.to_string(), args.to_vec());
        if self.repeat == Repeat::Once {
            match self.ran.get(&key) {
                // Run-once exists to keep a task's *effects* from happening
                // twice. A capture asks for a value, and the second asking is
                // not a second request for the work: replay what the first
                // call printed. Suppressing it and answering `Output::ok()` —
                // an empty, successful output — is what used to blank
                // `platform=$(platform-id)` on its second use, silently.
                Some(Some(recorded)) if wants_value => {
                    return Ok(Called::Done(Output {
                        stdout: recorded.clone(),
                        ..Output::ok()
                    }));
                }
                // The work is done and nobody wants a value: skip it.
                Some(_) if !wants_value => return Ok(Called::Done(Output::ok())),
                // Ran, but streamed to the terminal, so there is no value to
                // replay. Running it again is the only honest way to answer,
                // and it beats handing back an empty string that would be
                // interpolated into a path. A task used as a function is
                // called in `$(...)` every time and never reaches this.
                Some(_) => {}
                None => {
                    self.ran.insert(key, None);
                }
            }
        }

        if self.frames.len() > MAX_DEPTH {
            return Err(Error::Run {
                message: format!("task `{name}` recursed more than {MAX_DEPTH} levels deep"),
            });
        }

        self.frames.push(Frame {
            task: name.to_string(),
            args: args.to_vec(),
            // The callee starts where the caller stands, and its own `cd`
            // dies with the frame.
            cwd: self.cwd().to_path_buf(),
            vars: HashMap::new(),
        });
        let flow = self.block(&task.body);
        self.frames.pop();

        match flow? {
            Flow::Normal => Ok(Called::Done(Output::ok())),
            // `return` stops here, at the frame that raised it: the call is
            // over, the caller is not. The code is the task's status, so a
            // `return 1` reads to the caller exactly like a command that
            // exited 1 — `&&` skips, `||` takes over, `try` swallows it, and
            // outside those the caller stops fail-fast. `exit` is the other
            // half of this match precisely because it does *not* stop here.
            Flow::Return(code) => Ok(Called::Done(Output::failed(code))),
            Flow::Exit(code) => Ok(Called::Exited(code)),
        }
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

fn write_file(path: &Path, bytes: &[u8], append: bool) -> Result<()> {
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
struct Shared(Rc<RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
