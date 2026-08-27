//! Running a parsed chorefile.
//!
//! Commands resolve task, then builtin, then `PATH`; a leading `^` skips
//! straight to `PATH`. Arguments are passed to the OS as argv, never through a
//! shell, so nothing is re-quoted or re-expanded on the way out.
//!
//! This module holds the run's state and its statements; [`expand`] turns
//! words into argv, and [`run`] carries a command out.
//!
//! # `include`
//!
//! By the time a tree reaches here, [`resolve`](crate::resolve) has followed
//! every `include` and merged the result into one [`File`], so the
//! interpreter never learns that more than one file existed. A task pulled in
//! under `include libs/chorefile as libs` is simply a task *named*
//! `libs::build`: it is looked up, called, captured, piped, redirected and
//! keyed for run-once by that whole string, which is what keeps a `build` in
//! one namespace from standing in for a `build` in another. `$TASK` reports
//! the same name — the one `chore list` prints and the one that has to be
//! typed to run it.
//!
//! The one thing merging must not move is `$ROOT`. It is one directory per
//! invocation, the top-level chorefile's, so that a relative path written in
//! an included file lands where the project's author expects rather than next
//! to the file that happened to contain it. The interpreter holds it in a
//! field, answers `$ROOT` from that field, and hands the same path to every
//! builtin as `Ctx::root`; [`Interpreter::merged`] takes it from
//! [`Merged::root`] so a caller cannot supply a different one by accident.
//!
//! # What `--dry` does with a command that cannot run
//!
//! A preview skips effects but still runs captures and conditions, so a
//! read-only command under `--dry` looks at a world the recipe has not built
//! yet: `read dist/manifest.json` before the `download` that fetches it,
//! `find build/` before the `mkdir` that creates it. Letting those failures
//! unwind ends the preview at the first such step, which is exactly the step
//! whose *successors* the author wanted to see. So under [`Mode::Dry`]:
//!
//! - A builtin that fails is reported on stderr and becomes a command that
//!   exited nonzero, rather than an error that stops the run — see
//!   `run_builtin`. `fail` is the one exception: it is a hard stop the author
//!   wrote, and a preview that swallowed it would describe a run that cannot
//!   happen.
//! - A program on `PATH` that a capture or condition needs and cannot spawn
//!   is treated the same way: it may be the tool a skipped step installs.
//! - A nonzero command does not end the preview. A statement reports it and
//!   moves on, `&&` and `||` branch as they always do, and a `$(...)` yields
//!   the empty string with a note on stderr.
//! - **A condition is believed only when its command actually answered.** Any
//!   command that *fails* inside an `if` condition leaves the condition
//!   undecided, and an undecided condition takes the `then` branch:
//!   previewing the work the author wrote beats previewing nothing. A command
//!   that ran and answered — even by exiting nonzero — is believed, because a
//!   verdict is a verdict whichever way it goes.
//!
//!   The rule is positional, not per-builtin: it is about where the command
//!   sits, not which command it is. `exists`, `which` and `env <NAME>` look
//!   like exceptions only because they are the three builtins that *cannot*
//!   fail — a miss is their answer, reported as a nonzero exit, so
//!   `if exists build/version.txt` previews the `else` branch while
//!   `if read build/version.txt` previews the `then` branch. Every other
//!   builtin reports a miss as a failure. A program on `PATH` inside a
//!   condition really does run under `--dry`, so its nonzero exit is an
//!   answer and is believed; only a program that cannot be spawned at all
//!   leaves the condition undecided.
//!
//!   The choice is made above the whole condition, once, so it composes: a
//!   failure anywhere inside `&&`, `||` or `not` leaves the condition
//!   undecided, and a `not` around it does not flip the branch — there is no
//!   truth value to negate. Short-circuiting is respected: in
//!   `if exists x && read x` the left side answered "no" and decided the
//!   condition before the right side ever ran.
//!
//! None of this applies to [`Mode::Run`], where a failing command is a failing
//! run. A preview is a preview of the commands, not a claim that the run would
//! succeed.

mod expand;
mod run;

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ast::{Block, CompareOp, Cond, File, Stmt, Task, Word};
use crate::error::{Error, Result};
use crate::exec::{Builtin, Output};
use crate::resolve::Merged;
use crate::{builtins, vars};

/// How much of a run actually happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Run everything.
    Run,
    /// `--dry`: echo each command, skip the ones with effects.
    ///
    /// Captures and conditions still run. A `$(...)` that did not execute
    /// would leave every interpolated path downstream empty, which makes the
    /// preview describe a run that could never happen.
    ///
    /// Because the effects *are* skipped, the world a read-only command looks
    /// at is the world before the run, not the one the recipe builds as it
    /// goes: `find build/` runs before the `mkdir build` that would have
    /// created it. See the module docs for what a preview does with a command
    /// that therefore cannot answer.
    Dry,
}

/// Whether a task that already ran this invocation runs again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    /// Default: a task runs once per invocation, keyed on its name *and* its
    /// arguments, so a parameterised task called twice with different
    /// arguments is not silently skipped.
    Once,
    /// `--force`.
    Always,
}

/// Task calls nest, so a runaway recursion would otherwise blow the stack.
const MAX_DEPTH: usize = 128;

// ---------------------------------------------------------------------------
// Builtin dispatch
// ---------------------------------------------------------------------------

/// Look up a builtin by name.
///
/// WIRE BUILTINS HERE — this is the single place the real table is attached.
/// A reserved name that no module claims resolves to `None`, and the
/// interpreter says so rather than falling through to `PATH` — which would
/// silently run `rm` when the chorefile asked for `remove`.
pub fn builtin(name: &str) -> Option<Builtin> {
    builtins::fs::lookup(name)
        .or_else(|| builtins::net::lookup(name))
        .or_else(|| builtins::pack::lookup(name))
}

/// Resolves a builtin name to its implementation. Swappable so tests can run
/// the whole pipeline — pipes, redirects, captures — without shelling out.
pub type BuiltinTable = fn(&str) -> Option<Builtin>;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One task invocation: its locals, its positional arguments, and its
/// directory. Dropping the frame is what stops a `cd` leaking to the caller.
struct Frame {
    task: String,
    args: Vec<String>,
    vars: HashMap<String, String>,
    cwd: PathBuf,
}

/// Where a command's output goes. Owns its
/// path and remembers whether `>` or `>>` asked for it.
enum Dest {
    Stream,
    Capture,
    File { path: PathBuf, append: bool },
}

/// Per-call execution flags, threaded down a chain.
#[derive(Clone, Copy)]
struct Flags {
    /// Print `$ cmd` before running. Off inside captures and conditions, which
    /// are machinery rather than steps of the recipe.
    echo: bool,
    /// Something consumes this command's output or exit status, so `--dry`
    /// must run it anyway.
    needed: bool,
}

/// The two ways a task call ends.
enum Called {
    Done(Output),
    Exited(i32),
}

/// What a block did when it stopped.
///
/// The two abnormal exits differ only in where they stop, and that is the
/// whole of the distinction between the statements that raise them:
/// [`Flow::Exit`] unwinds every frame to the top of the run, while
/// [`Flow::Return`] is caught by `call_task` at the end of the task that
/// raised it, so the caller carries on with its next statement.
enum Flow {
    Normal,
    /// `exit [code]`, unwinding to the top of the run.
    Exit(i32),
    /// `return [code]`, unwinding to the end of the enclosing task only. The
    /// code becomes that task's exit status, which is what `&&`, `||`, `try`,
    /// a condition and a capture read. In the task the command line named
    /// there is no caller left to return to, so it ends the run with that
    /// code — zero, and so a success, unless one was written.
    Return(i32),
}

/// A parsed chorefile, plus everything a run needs to keep track of.
pub struct Interpreter<'a> {
    file: &'a File,
    root: PathBuf,
    mode: Mode,
    repeat: Repeat,
    globals: HashMap<String, String>,
    /// Run-once records, keyed on `(task, args)`. The value is the task's
    /// captured stdout the first time something asked for it, or `None` while
    /// nothing has: replaying it is what lets a task be used as a function
    /// without running its body twice. See `call_task`.
    ran: HashMap<(String, Vec<String>), Option<Vec<u8>>>,
    frames: Vec<Frame>,
    globals_done: bool,
    /// Set when a called task ran `exit`, so the caller unwinds too.
    pending_exit: Option<i32>,
    /// True while a task's own output is being captured: its steps must not
    /// echo into the buffer the caller is about to read.
    quiet: bool,
    /// Set when `--dry` had to paper over a command it could not carry out,
    /// so the caller can tell "the command answered no" from "the command
    /// could not answer". Scoped with `tracking`.
    unevaluated: bool,
    /// One timestamp per run, so two lines of the same recipe cannot disagree
    /// about what `$NOW` is.
    now: String,
    builtins: BuiltinTable,
    out: Box<dyn Write>,
    /// Where builtins' diagnostics go. Held here rather than reached for as
    /// `io::stderr()` at each call so a `2>` on a *task* can divert everything
    /// the task's own commands report, the way `>` already diverts what they
    /// print.
    err: Box<dyn Write>,
}

impl<'a> Interpreter<'a> {
    /// `root` is the directory holding the top-level chorefile: `$ROOT`, and
    /// the directory commands start in.
    pub fn new(file: &'a File, root: impl Into<PathBuf>, mode: Mode, repeat: Repeat) -> Self {
        let root = root.into();
        let globals = vars::statics(&root)
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let frame = Frame {
            task: String::new(),
            args: Vec::new(),
            vars: HashMap::new(),
            cwd: root.clone(),
        };
        Self {
            file,
            root,
            mode,
            repeat,
            globals,
            ran: HashMap::new(),
            frames: vec![frame],
            globals_done: false,
            pending_exit: None,
            quiet: false,
            unevaluated: false,
            now: now_iso8601(),
            builtins: builtin,
            out: Box::new(io::stdout()),
            err: Box::new(io::stderr()),
        }
    }

    /// Run a chorefile and everything its `include`s pulled in.
    ///
    /// The merged tree names an included task the way `include ... as libs`
    /// asked — `libs::build` — and the interpreter treats that as a task name
    /// like any other. The reason to prefer this over [`new`](Self::new) is
    /// the root: it comes from [`Merged::root`], the top-level chorefile's
    /// directory, so a caller cannot pair a merged tree with an included
    /// file's directory and send its `download ... third_party/` somewhere
    /// its author never chose.
    pub fn merged(merged: &'a Merged, mode: Mode, repeat: Repeat) -> Self {
        Self::new(&merged.file, &merged.root, mode, repeat)
    }

    /// Start somewhere other than `$ROOT`.
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.frames[0].cwd = cwd.into();
        self
    }

    /// Send echo lines and builtin output somewhere other than stdout.
    /// Programs on `PATH` still inherit the real stdout when they stream.
    pub fn with_output(mut self, out: Box<dyn Write>) -> Self {
        self.out = out;
        self
    }

    /// Send builtin diagnostics somewhere other than stderr. Programs on
    /// `PATH` still inherit the real stderr when they stream.
    pub fn with_error_output(mut self, err: Box<dyn Write>) -> Self {
        self.err = err;
        self
    }

    /// Override builtin resolution.
    pub fn with_builtins(mut self, table: BuiltinTable) -> Self {
        self.builtins = table;
        self
    }

    /// Every task in the file, in source order.
    pub fn tasks(&self) -> &'a [Task] {
        &self.file.tasks
    }

    pub fn task(&self, name: &str) -> Option<&'a Task> {
        self.file.tasks.iter().find(|t| t.name == name)
    }

    /// Run one task by name. Returns the exit code the run ends with.
    ///
    /// This task has no caller, so the two ways of stopping early meet here
    /// and give the same answer: an `exit` unwound to the top, and a `return`
    /// that ended this task, both end the run with the code they named — zero
    /// by default, so a bare `return` on the last reachable line is a success.
    pub fn run_task(&mut self, name: &str, args: &[String]) -> Result<i32> {
        self.run_globals()?;
        match self.call_task(name, args, false)? {
            Called::Exited(code) => Ok(code),
            Called::Done(out) => Ok(out.code),
        }
    }

    /// Evaluate top-level assignments. Idempotent: they run once, before the
    /// first task, and `list`/`help`/`check`/`spec` never get here at all.
    pub fn run_globals(&mut self) -> Result<()> {
        if self.globals_done {
            return Ok(());
        }
        self.globals_done = true;
        for assign in &self.file.globals {
            let value = self.expand_to_string(&assign.value)?;
            self.globals.insert(assign.name.clone(), value);
        }
        Ok(())
    }

    // -- variables ---------------------------------------------------------

    fn frame(&self) -> &Frame {
        self.frames.last().expect("one frame always exists")
    }

    fn cwd(&self) -> &Path {
        &self.frame().cwd
    }

    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            // Derived from the frame, so they cannot be stored in a map.
            "CWD" => return Some(vars::display(self.cwd())),
            // The name the task was *called* by, which after `include ... as`
            // is the namespaced one: a task merged in as `libs::build` sees
            // `libs::build`, the same string `chore list` shows and the same
            // one that has to be typed to run it. Answering `build` would name
            // something that cannot be called.
            "TASK" => return Some(self.frame().task.clone()),
            "NOW" => return Some(self.now.clone()),
            // Answered from the field, not the map: `$ROOT` is one directory
            // per invocation, and an assignment — in the top-level file or in
            // one merged in by `include` — must not move it. The builtins
            // already read `self.root` through `Ctx::root`, so resolving it
            // anywhere else is what would let `$ROOT` and `remove`'s refusal
            // to delete the root disagree about where the root is.
            "ROOT" => return Some(vars::display(&self.root)),
            _ => {}
        }
        self.frame()
            .vars
            .get(name)
            .or_else(|| self.globals.get(name))
            .cloned()
    }

    fn assign(&mut self, name: &str, value: String) {
        // Top level writes globals; inside a task an assignment is local, so a
        // task cannot quietly rewrite the file's configuration for its caller.
        if self.frames.len() > 1 {
            self.frames
                .last_mut()
                .unwrap()
                .vars
                .insert(name.into(), value);
        } else {
            self.globals.insert(name.into(), value);
        }
    }

    // -- statements --------------------------------------------------------

    /// Run a block until it ends or something stops it early. Both kinds of
    /// early stop leave every enclosing block — including a `for` body, which
    /// is why `return` inside a loop leaves the task rather than the loop.
    fn block(&mut self, block: &Block) -> Result<Flow> {
        for stmt in block {
            match self.stmt(stmt)? {
                Flow::Normal => {}
                stopped => return Ok(stopped),
            }
        }
        Ok(Flow::Normal)
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<Flow> {
        match stmt {
            Stmt::Assign(a) => {
                let value = self.expand_to_string(&a.value)?;
                self.assign(&a.name, value);
            }
            Stmt::Command(chain) => {
                let flags = Flags {
                    echo: true,
                    needed: false,
                };
                let out = self.chain(chain, Dest::Stream, None, flags)?;
                if let Some(flow) = self.exit_requested() {
                    return Ok(flow);
                }
                if !out.success() && self.mode == Mode::Dry {
                    // The failure is already on stderr. A preview stops only
                    // at `fail`, which unwinds as an error and never gets
                    // here.
                    return Ok(Flow::Normal);
                }
                if !out.success() {
                    // Fail fast: only `try` may swallow this.
                    return Err(Error::Run {
                        message: format!(
                            "`{}` exited with code {}",
                            run::describe(chain, &mut |w| self.preview(w)),
                            out.code
                        ),
                    });
                }
            }
            Stmt::Try(chain) => {
                let flags = Flags {
                    echo: true,
                    needed: false,
                };
                // `try` swallows a nonzero exit *and* the run errors that come
                // from one — a failed capture inside it, a missing program —
                // but never an I/O fault, which is not the command's verdict.
                match self.chain(chain, Dest::Stream, None, flags) {
                    Ok(_) | Err(Error::Run { .. }) => {}
                    Err(e) => return Err(e),
                }
                if let Some(flow) = self.exit_requested() {
                    return Ok(flow);
                }
            }
            Stmt::If(node) => {
                let (taken, undecided) = self.tracking(|me| me.cond(&node.cond));
                // Under `--dry` a condition is believed only when its command
                // answered; one that failed leaves the condition undecided and
                // previews the `then` branch. Deciding here, above the whole
                // condition, is what makes the rule compose: a failure inside
                // an `&&` or under a `not` still lands on `then`, because
                // there is no truth value for the `not` to flip.
                if taken? || (undecided && self.mode == Mode::Dry) {
                    return self.block(&node.then);
                } else if let Some(other) = &node.otherwise {
                    return self.block(other);
                }
            }
            Stmt::For(node) => {
                let mut items = Vec::new();
                for word in &node.items {
                    items.extend(self.expand(word)?);
                }
                for item in items {
                    self.assign(&node.var, item);
                    // There is no `break`, so anything that stops the body
                    // stops the loop and everything around it: `return` in a
                    // `for` ends the task, not the iteration.
                    match self.block(&node.body)? {
                        Flow::Normal => {}
                        stopped => return Ok(stopped),
                    }
                }
            }
            Stmt::Exit(code) => return Ok(Flow::Exit(self.status_code(code, "exit")?)),
            Stmt::Return(code) => return Ok(Flow::Return(self.status_code(code, "return")?)),
        }
        Ok(Flow::Normal)
    }

    /// The code written after `exit` or `return`, or zero when it was left
    /// off. `keyword` only names the statement in the error.
    fn status_code(&mut self, code: &Option<Word>, keyword: &str) -> Result<i32> {
        let Some(word) = code else {
            return Ok(0);
        };
        let text = self.expand_to_string(word)?;
        text.trim().parse().map_err(|_| Error::Run {
            message: format!("{keyword} code `{text}` is not a number"),
        })
    }

    /// A called task may have run `exit`; that unwinds through the caller too.
    fn exit_requested(&mut self) -> Option<Flow> {
        self.pending_exit.take().map(Flow::Exit)
    }

    /// Run `f`, reporting whether `--dry` had to paper over a command inside
    /// it. The flag still propagates outwards, so a capture nested in a
    /// condition marks the condition unevaluated too.
    fn tracking<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> (T, bool) {
        let outer = std::mem::replace(&mut self.unevaluated, false);
        let value = f(self);
        let hit = self.unevaluated;
        self.unevaluated = outer || hit;
        (value, hit)
    }

    /// Turn a command `--dry` could not carry out into a failed command:
    /// report the reason, and remember that the answer is a stand-in rather
    /// than the command's own verdict.
    pub(super) fn dry_failed(&mut self, message: &str) -> Output {
        self.unevaluated = true;
        let _ = writeln!(self.err, "--dry: {message}");
        let _ = self.err.flush();
        Output::failed(1)
    }

    // -- conditions --------------------------------------------------------

    fn cond(&mut self, cond: &Cond) -> Result<bool> {
        match cond {
            Cond::Compare { left, op, right } => {
                let l = self.expand_to_string(left)?;
                let r = self.expand_to_string(right)?;
                Ok(match op {
                    CompareOp::Eq => l == r,
                    CompareOp::Ne => l != r,
                    CompareOp::Contains => l.contains(&r),
                    CompareOp::StartsWith => l.starts_with(&r),
                    CompareOp::EndsWith => l.ends_with(&r),
                })
            }
            Cond::Command(chain) => {
                let flags = Flags {
                    echo: false,
                    needed: true,
                };
                // Captured, not streamed: `if which cargo` is a test, and its
                // path on the terminal would be noise. Nonzero is the answer,
                // not a failure, so a run error here means false as well.
                match self.chain(chain, Dest::Capture, None, flags) {
                    Ok(out) => Ok(out.success()),
                    Err(Error::Run { .. }) => Ok(false),
                    Err(e) => Err(e),
                }
            }
            Cond::Not(inner) => Ok(!self.cond(inner)?),
            Cond::And(a, b) => Ok(self.cond(a)? && self.cond(b)?),
            Cond::Or(a, b) => Ok(self.cond(a)? || self.cond(b)?),
        }
    }
}

/// `$NOW`, as `YYYY-MM-DDTHH:MM:SSZ` in UTC.
fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let time = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's algorithm),
/// so `$NOW` needs no dependency.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
