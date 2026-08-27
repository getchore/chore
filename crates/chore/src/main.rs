//! `chore` — run project tasks from a chorefile.
//!
//! The binary is a thin shell around the `chorefile` crate: it finds the file,
//! resolves its `include`s into one tree, and hands that to the interpreter.
//! Everything the CLI prints about the language itself comes from
//! `chorefile::spec`, so the reference lives in exactly one place.

mod args;
mod help;
mod lint;
mod list;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chorefile::interp::{Interpreter, Mode, Repeat};
use chorefile::resolve::{Merged, Sources};
use chorefile::{Error, parse, resolve};

use args::{Command, Invocation, UsageError};

/// Exit codes, as promised in docs/SPEC.md.
const OK: u8 = 0;
const FAILED: u8 = 1;
const USAGE: u8 = 2;

const USAGE_TEXT: &str = "\
usage: chore <task> [args...] [--dry] [--force]
       chore list [--json]        tasks and descriptions
       chore help [builtin]       syntax and builtins, or one builtin
       chore check                lint without running
       chore spec                 full reference as JSON, for agents

  --dry      echo commands without side effects
  --force    disable run-once";

/// The interpreter evaluates a chorefile by recursing, and its depth guard
/// allows 128 nested calls. That fits the 8 MB main thread Linux and macOS
/// give a process, and does not fit the 1 MB Windows gives a thread — so on
/// Windows the process died where the other platforms reported a clean
/// "recursed more than 128 levels deep". `chore` promises identical behavior
/// on every platform, so the work runs on a stack big enough to keep that
/// promise everywhere rather than on whatever the host happened to hand us.
const STACK: usize = 32 * 1024 * 1024;

fn main() -> ExitCode {
    match std::thread::Builder::new()
        .stack_size(STACK)
        .name("chore".into())
        .spawn(cli)
    {
        // A thread that panicked has already printed why.
        Ok(handle) => handle.join().unwrap_or(ExitCode::from(FAILED)),
        // If the OS will not give us a thread, run on the one we have: a
        // deep chorefile may still overflow, but a shallow one works.
        Err(_) => cli(),
    }
}

fn cli() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let code = match args::parse(argv) {
        Err(UsageError(message)) => {
            let _ = writeln!(io::stderr(), "chore: {message}");
            USAGE
        }
        Ok(invocation) => match dispatch(invocation, &mut out) {
            Ok(code) => code,
            Err(exit) => {
                let _ = writeln!(io::stderr(), "chore: {}", exit.message);
                exit.code
            }
        },
    };
    let _ = out.flush();
    ExitCode::from(code)
}

/// An error on its way to stderr, with the exit code it implies.
struct Exit {
    message: String,
    code: u8,
}

impl Exit {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: USAGE,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: FAILED,
        }
    }
}

impl From<io::Error> for Exit {
    fn from(e: io::Error) -> Self {
        Self::failed(e.to_string())
    }
}

/// Anything that is not the run itself is an error we print and exit on.
impl From<Error> for Exit {
    fn from(e: Error) -> Self {
        match e {
            // No chorefile is a usage problem: nothing about the request was
            // runnable in the first place.
            Error::NotFound { .. } => Self::usage(e.to_string()),
            Error::Syntax { .. } => Self::failed(e.to_string()),
            other => Self::failed(other.to_string()),
        }
    }
}

fn dispatch(invocation: Invocation, out: &mut dyn Write) -> Result<u8, Exit> {
    let Invocation {
        command,
        mode,
        repeat,
    } = invocation;

    match command {
        // `help` and `spec` describe the language, not a project, so they work
        // with no chorefile anywhere.
        Command::Version => {
            writeln!(out, "chore {}", env!("CARGO_PKG_VERSION"))?;
            Ok(OK)
        }
        Command::Help { topic } => match topic {
            Some(name) => help::builtin(out, &name).map(|()| OK),
            None => help::overview(out).map(|()| OK),
        },
        Command::Spec => {
            writeln!(out, "{}", chorefile::spec::json())?;
            Ok(OK)
        }
        Command::Usage => {
            writeln!(out, "{USAGE_TEXT}")?;
            let loaded = Loaded::discover()?;
            writeln!(out, "\ntasks:")?;
            list::text(out, &loaded.merged)?;
            Ok(OK)
        }
        Command::List { json } => {
            let loaded = Loaded::discover()?;
            if json {
                list::json(out, &loaded.merged)?;
            } else {
                list::text(out, &loaded.merged)?;
            }
            Ok(OK)
        }
        Command::Check => {
            // `check` resolves for itself rather than going through `Loaded`:
            // `check_path` turns a failure to merge — a missing included file,
            // a cycle — into a finding in the list instead of an error that
            // ends the run, which is what a CI gate wants. A chorefile too
            // broken to load still gets a full report.
            let path = discover()?;
            let (findings, merged) = chorefile::check::check_path(&path);
            let fallback;
            let sources = match &merged {
                Some(merged) => &merged.sources,
                // Nothing merged, so nothing knows the text of the file the
                // findings point into. Supply the one file we do have, and
                // its findings keep their line and column.
                None => {
                    fallback = one_source(&path);
                    &fallback
                }
            };
            let errors = lint::report(out, &findings, sources)?;
            Ok(if errors == 0 { OK } else { FAILED })
        }
        Command::Run { task, args } => {
            let loaded = Loaded::discover()?;
            run(&loaded, &task, &args, mode, repeat)
        }
    }
}

/// The chorefile that governs the working directory, with its includes
/// followed and merged.
struct Loaded {
    /// The top-level chorefile itself. `Merged::root` is its *directory*, and
    /// the message for a task that does not exist names the file.
    path: PathBuf,
    merged: Merged,
}

impl Loaded {
    fn discover() -> Result<Self, Exit> {
        let path = discover()?;
        let merged = resolve::resolve(&path).map_err(|e| unresolved(&path, e))?;
        Ok(Self { path, merged })
    }

    /// What was searched for a task, for the message when one is missing.
    fn searched(&self) -> String {
        let path = self.path.display();
        match self.merged.sources.files().count() {
            0 | 1 => format!("{path}"),
            2 => format!("{path} or the file it includes"),
            n => format!("{path} or the {} files it includes", n - 1),
        }
    }
}

/// The chorefile governing the working directory: the nearest one at or above
/// it. Every subcommand that needs a project starts here.
fn discover() -> Result<PathBuf, Exit> {
    let cwd = std::env::current_dir().map_err(|e| Exit::usage(e.to_string()))?;
    Ok(chorefile::find(&cwd)?)
}

/// One file's text as a [`Sources`], for rendering diagnostics when no merge
/// succeeded and there is nothing else that knows the text.
fn one_source(path: &Path) -> Sources {
    let mut sources = Sources::default();
    if let Ok(text) = std::fs::read_to_string(path) {
        sources.insert(path, text);
    }
    sources
}

/// Whether the file we found parses on its own — that is, whether a failure to
/// resolve was about *this* file or about something it included.
fn parses(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| parse::parse(&text, path).is_ok())
}

/// A chorefile that could not be assembled: a missing include, a cycle, a
/// name two files both define, or a file that does not parse.
///
/// The resolver reports all of these as `Syntax`, so the exit code is decided
/// by asking which kind of failure it was: the file we found not parsing is
/// exit 1, exactly as it was before includes existed, and anything the
/// resolver could only have found by following an `include` is exit 2 — like
/// a missing chorefile, the project never assembled, so nothing about the
/// request was runnable. Re-parsing to ask that question costs nothing: we
/// are already on the way out.
///
/// An `Io` error carries no path of its own — "No such file or directory (os
/// error 2)" on its own line is useless — so the chorefile is named for it.
fn unresolved(path: &Path, e: Error) -> Exit {
    let message = match &e {
        Error::Io(io) => format!("{}: {io}", path.display()),
        _ => e.to_string(),
    };
    let code = if parses(path) { USAGE } else { FAILED };
    Exit { message, code }
}

fn run(
    loaded: &Loaded,
    task: &str,
    args: &[String],
    mode: Mode,
    repeat: Repeat,
) -> Result<u8, Exit> {
    // `Interpreter::merged` takes `$ROOT` from the merged tree, so an included
    // file's directory can never be handed in by mistake.
    let mut interp =
        Interpreter::merged(&loaded.merged, mode, repeat).with_output(Box::new(io::stdout()));
    if interp.task(task).is_none() {
        return Err(Exit::usage(format!(
            "no task `{task}` in {} (try `chore list`)",
            loaded.searched()
        )));
    }
    let code = interp.run_task(task, args)?;
    // A task's own `exit 3` is a verdict worth keeping, so the process exit
    // code is the run's, not a flat 1.
    Ok(u8::try_from(code).unwrap_or(FAILED))
}
