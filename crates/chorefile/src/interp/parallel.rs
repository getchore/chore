//! `parallel <task>...`, the one builtin that runs tasks.
//!
//! Its arguments are task names, so it cannot be a [`Builtin`](crate::exec::Builtin):
//! those take a `Ctx` and have no way to call a task. Like `cd`, it is
//! resolved by the interpreter itself, and like every other builtin its name
//! is reserved so no task can take it.
//!
//! # What is shared and what is not
//!
//! Each sibling runs on its own thread with its own [`Interpreter`], because
//! frames, `cd`, locals and `$TASK` are per-call state that concurrent tasks
//! must not share. Three things *are* shared, and each for a reason:
//!
//! - The run-once record, so the documented "a task runs once per invocation"
//!   still holds across siblings: `parallel build test` where both call
//!   `deps` runs `deps` once and the second sibling waits for the first
//!   rather than repeating its effects. This is the whole difficulty, and it
//!   lives in [`memo`](super::memo).
//! - `$ROOT`, the globals, `$NOW` and the builtin table, which are facts about
//!   the invocation. A sibling that computed its own `$NOW` would disagree
//!   with its parent about what time the run started.
//! - The environment overlay *as it stands at the call*, copied into each
//!   child the way the current directory is. A sibling therefore sees every
//!   `env NAME value` its callers made, and none that another sibling makes
//!   while it runs: the copies are separate, so two siblings setting the same
//!   name cannot race, and nothing they set outlives the `parallel`. This is
//!   the reason `env` never touches the process environment, which they
//!   really would share.
//! - The current directory *at the call*, copied into each child the way
//!   `call_task` copies it into a frame, so a task called from a `parallel`
//!   starts exactly where it would have started had it been called directly.
//!   A `cd` inside a sibling dies with that sibling, as it does with a frame.
//!
//! # Output
//!
//! Each sibling writes into its own pair of buffers, and the blocks are
//! printed when everything has finished, in the order the tasks were named.
//! `parallel lint test` therefore prints exactly what `lint` then `test`
//! would have printed: concurrency changes the timing, not the transcript.
//! Streaming instead would interleave two builds line by line, which is the
//! usual reason parallel task runners are unpleasant to read. A sibling's own
//! stdout and stderr keep their own blocks, so the interleaving *between* the
//! two streams of one task is the one thing not preserved.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::ast::File;
use crate::error::{Error, Result};
use crate::exec::{EnvOverlay, Output};

use super::memo::{CtxId, Memo};
use super::run::{Shared, write_file};
use super::{BuiltinTable, Called, Dest, Errs, Interpreter, Mode, Repeat};

/// The `--fail-fast` flag, and the tasks to run.
struct Plan {
    names: Vec<String>,
    fail_fast: bool,
}

impl Plan {
    fn parse(args: &[String]) -> Result<Self> {
        let mut plan = Plan {
            names: Vec::new(),
            fail_fast: false,
        };
        for arg in args {
            match arg.as_str() {
                "--fail-fast" => plan.fail_fast = true,
                other if other.starts_with("--") => {
                    return Err(Error::Run {
                        message: format!("parallel: unknown option {other}"),
                    });
                }
                other => plan.names.push(other.to_string()),
            }
        }
        if plan.names.is_empty() {
            return Err(Error::Run {
                message: "usage: parallel [--fail-fast] <task>...".to_string(),
            });
        }
        Ok(plan)
    }
}

/// How one sibling ended.
enum Ended {
    /// It finished. Nonzero is a failure, as for any other command.
    Code(i32),
    /// It ran `exit`, which unwinds the whole run and not just this task.
    Exited(i32),
    /// A run error: the message is the diagnostic the task would have shown.
    Failed(String),
    /// `--fail-fast` stopped it between statements. Not a failure of its own:
    /// something else already failed, and this task never got to finish.
    Stopped,
    /// An I/O fault, which is ours and not the task's verdict, so it keeps
    /// unwinding the way it would outside a `parallel`.
    Fault(Error),
}

impl Ended {
    fn failed(&self) -> bool {
        !matches!(self, Ended::Code(0) | Ended::Stopped)
    }
}

/// One finished sibling, with the output it produced.
struct Ran {
    name: String,
    out: Vec<u8>,
    err: Vec<u8>,
    ended: Ended,
}

impl<'a> Interpreter<'a> {
    /// `parallel [--fail-fast] <task>...`: run the named tasks concurrently,
    /// wait for all of them, and fail if any failed.
    pub(super) fn parallel(&mut self, argv: &[String], dest: Dest, errs: Errs) -> Result<Output> {
        let plan = Plan::parse(&argv[1..])?;
        // Every name is checked before any of them runs: half a `parallel`
        // followed by "unknown task" is a worse answer than none of it.
        for name in &plan.names {
            if self.task(name).is_none() {
                return Err(Error::Run {
                    message: format!("parallel: `{name}` is not a task"),
                });
            }
        }

        let ran = if self.mode == Mode::Dry {
            self.preview_siblings(&plan)
        } else {
            self.run_siblings(&plan)
        };
        self.deliver(ran, dest, errs)
    }

    /// `--dry` runs the siblings one after another, in the order they were
    /// named.
    ///
    /// A preview is a description of the work, and the description does not
    /// change with the number of threads: the same tasks are previewed, in
    /// the same blocks, in the same order. What running them concurrently
    /// *would* add is a preview whose captures and conditions raced each
    /// other, which is a worse preview and a much larger promise to keep.
    /// Sequential is also the honest reading of run-once here: under `--dry`
    /// the second caller of a shared `deps` sees it already recorded, exactly
    /// as it would in a real run.
    fn preview_siblings(&mut self, plan: &Plan) -> Vec<Ran> {
        let mut ran = Vec::new();
        for name in &plan.names {
            let (out, err) = (Shared::new(), Shared::new());
            let outer = self.swap_output(out.writer(), err.writer());
            let ended = self.sibling(name);
            self.restore_output(outer);
            ran.push(Ran {
                name: name.clone(),
                out: out.take(),
                err: err.take(),
                ended,
            });
            if plan.fail_fast && ran.last().is_some_and(|r| r.ended.failed()) {
                break;
            }
        }
        ran
    }

    /// One thread per task, joined in the order the tasks were named.
    fn run_siblings(&mut self, plan: &Plan) -> Vec<Ran> {
        // An outer `--fail-fast` already reaches us, and its flag reaches our
        // children too: a failure anywhere under it stops everything under
        // it. Only a `parallel` that has no flag above it makes a new one.
        let abort = match (&self.abort, plan.fail_fast) {
            (Some(outer), _) => Some(Arc::clone(outer)),
            (None, true) => Some(Arc::new(AtomicBool::new(false))),
            (None, false) => None,
        };

        // The children are registered as ours before they start, so a
        // grandchild that asks for a task we are still running can see that
        // waiting for us would be waiting for itself. See [`memo`].
        let ids: Vec<CtxId> = plan.names.iter().map(|_| self.memo.context()).collect();
        self.memo.joining(self.ctx, ids.clone());

        let ran = thread::scope(|scope| {
            let mut running = Vec::new();
            for (name, id) in plan.names.iter().zip(ids) {
                let fork = self.fork(id, abort.clone());
                let (fail_fast, abort) = (plan.fail_fast, abort.clone());
                let task = name.clone();
                let handle = scope.spawn(move || {
                    let (out, err) = (Sink::default(), Sink::default());
                    let mut child = fork.interpreter(Box::new(out.clone()), Box::new(err.clone()));
                    let ended = child.sibling(&task);
                    // Set only once the claim this task held has been
                    // released, so a sibling blocked on it wakes to an answer
                    // rather than to a stopped run.
                    drop(child);
                    if fail_fast && ended.failed() {
                        if let Some(flag) = &abort {
                            flag.store(true, Ordering::Relaxed);
                        }
                    }
                    (out.take(), err.take(), ended)
                });
                running.push((name.clone(), handle));
            }
            running
                .into_iter()
                .map(|(name, handle)| match handle.join() {
                    Ok((out, err, ended)) => Ran {
                        name,
                        out,
                        err,
                        ended,
                    },
                    // A panic has already printed its own message; what
                    // matters here is that the call fails rather than
                    // resuming as though the task had run.
                    Err(_) => Ran {
                        name,
                        out: Vec::new(),
                        err: Vec::new(),
                        ended: Ended::Failed("panicked".to_string()),
                    },
                })
                .collect::<Vec<_>>()
        });

        self.memo.joined(self.ctx);
        ran
    }

    /// Run one named task as a sibling, turning every way it can end into an
    /// [`Ended`]. Nothing unwinds out of here: a `parallel` reports what all
    /// of its tasks did, so a failure is a value rather than a `?`.
    fn sibling(&mut self, name: &str) -> Ended {
        match self.call_task(name, &[], false) {
            Ok(call) => match call.called {
                Called::Exited(code) => Ended::Exited(code),
                // A task that stopped between statements did not fail; it
                // never reached its end. Its own nonzero code, if it has one,
                // is the better answer and wins.
                Called::Done(out) if out.success() && self.aborted => Ended::Stopped,
                Called::Done(out) => Ended::Code(out.code),
            },
            Err(Error::Run { message }) => Ended::Failed(message),
            Err(e) => Ended::Fault(e),
        }
    }

    /// Print the blocks in the order the tasks were named, then answer for
    /// the call as a whole.
    fn deliver(&mut self, ran: Vec<Ran>, dest: Dest, errs: Errs) -> Result<Output> {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut fault = None;
        let mut exited = None;
        let mut failure = None;
        for sibling in ran {
            out.extend(sibling.out);
            err.extend(sibling.err);
            let name = &sibling.name;
            // Every failure is reported, not just the first: the point of
            // letting the siblings finish is being told about all of them in
            // one run rather than one per run.
            match sibling.ended {
                Ended::Code(0) => {}
                Ended::Code(code) => {
                    let _ = writeln!(err, "parallel: task `{name}` failed with code {code}");
                    failure.get_or_insert(code);
                }
                Ended::Failed(message) => {
                    let _ = writeln!(err, "parallel: task `{name}` failed: {message}");
                    failure.get_or_insert(1);
                }
                Ended::Exited(code) => {
                    let _ = writeln!(
                        err,
                        "parallel: task `{name}` ended the run with code {code}"
                    );
                    exited.get_or_insert(code);
                }
                Ended::Stopped => {
                    let _ = writeln!(err, "parallel: task `{name}` stopped early: --fail-fast");
                }
                Ended::Fault(e) => {
                    let _ = writeln!(err, "parallel: task `{name}` failed: {e}");
                    fault.get_or_insert(e);
                }
            }
        }

        // `2>&1` on a `parallel` is settled here rather than by the caller:
        // the two streams are already whole blocks by this point, so joining
        // them is appending one to the other. Every sibling's stdout comes
        // first and every sibling's stderr after it, which is what the blocks
        // mean — the interleaving between the two streams is the one thing
        // `parallel` never preserved anyway.
        if matches!(errs, Errs::ToStdout) {
            out.append(&mut err);
        }

        // The output goes out before any `?`: the blocks are what the run
        // produced, and a fault in one sibling must not swallow what the
        // others printed.
        match &errs {
            // Under `--dry` nothing is written, the same rule a redirect on
            // any other command follows.
            Errs::File(path) if self.mode == Mode::Run => write_file(path, &err, false)?,
            Errs::File(_) | Errs::ToStdout => {}
            Errs::Inherit => {
                self.err.write_all(&err)?;
                self.err.flush()?;
            }
        }
        let mut result = Output::ok();
        match dest {
            Dest::Stream => {
                self.out.write_all(&out)?;
                self.out.flush()?;
            }
            Dest::Capture => result.stdout = out,
            Dest::File { path, append } => {
                if self.mode == Mode::Run {
                    write_file(&path, &out, append)?;
                }
            }
        }

        if let Some(e) = fault {
            return Err(e);
        }
        // `exit` in a sibling means what it means anywhere else: it unwinds
        // the whole run, not just the task that wrote it. It cannot stop the
        // other siblings, which were already running, so it takes effect once
        // they are all done and their output is printed. Two siblings that
        // both `exit` are answered by the first in the order they were named,
        // so the code does not depend on which thread got there first.
        if let Some(code) = exited {
            self.pending_exit = Some(code);
            return Ok(Output::failed(code));
        }
        // The failing sibling's own code, so `parallel test` fails the way
        // `test` did. The first in call order, again for determinism.
        Ok(Output::failed(failure.unwrap_or(0)))
    }

    /// Everything a sibling needs to build its own interpreter on its own
    /// thread.
    fn fork(&self, ctx: CtxId, abort: Option<Arc<AtomicBool>>) -> Fork<'a> {
        Fork {
            file: self.file,
            root: self.root.clone(),
            cwd: self.cwd().to_path_buf(),
            mode: self.mode,
            repeat: self.repeat,
            globals: self.globals.clone(),
            globals_invented: self.globals_invented.clone(),
            envs: self.envs.clone(),
            now: self.now.clone(),
            builtins: self.builtins,
            memo: Arc::clone(&self.memo),
            ctx,
            quiet: self.quiet,
            abort,
        }
    }
}

/// An interpreter in transit.
///
/// [`Interpreter`] holds `Box<dyn Write>` and is not `Send`, and it should
/// not be: it is one thread's state. This is the part of it that *is* shared
/// or copied, and it is what crosses to the new thread, which then builds its
/// own interpreter around its own buffers.
struct Fork<'a> {
    file: &'a File,
    root: PathBuf,
    cwd: PathBuf,
    mode: Mode,
    repeat: Repeat,
    globals: HashMap<String, String>,
    /// The environment `env NAME value` had built by the time of the call.
    /// Copied, not shared: see the module docs.
    envs: EnvOverlay,
    /// The `--dry` marks on those globals: a sibling reading one must be told
    /// the same thing the parent would have been.
    globals_invented: HashMap<String, String>,
    now: String,
    builtins: BuiltinTable,
    memo: Arc<Memo>,
    ctx: CtxId,
    quiet: bool,
    abort: Option<Arc<AtomicBool>>,
}

impl<'a> Fork<'a> {
    fn interpreter(self, out: Box<dyn Write>, err: Box<dyn Write>) -> Interpreter<'a> {
        // Through `new` rather than by hand, so a field added to the
        // interpreter is initialised once and cannot be forgotten here.
        let mut child = Interpreter::new(self.file, self.root, self.mode, self.repeat)
            .with_cwd(self.cwd)
            .with_builtins(self.builtins)
            .with_output(out)
            .with_error_output(err);
        child.globals = self.globals;
        child.globals_invented = self.globals_invented;
        child.envs = self.envs;
        // The globals were evaluated once, before the first task, and their
        // values came with us. Running them again would repeat every `$(...)`
        // in them, once per sibling.
        child.globals_done = true;
        child.now = self.now;
        child.memo = self.memo;
        child.ctx = self.ctx;
        child.quiet = self.quiet;
        child.abort = self.abort;
        child.captive = true;
        child
    }
}

/// A buffer a thread fills and its parent reads once the thread is joined.
///
/// The mutex is never contended in practice: only the child writes, and the
/// parent reads after the join. It is there because the handle has to cross
/// the thread boundary at all.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Sink {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.lock())
    }

    /// A panicking task can poison this, and the bytes it wrote before it
    /// panicked are still the bytes it wrote.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<u8>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
