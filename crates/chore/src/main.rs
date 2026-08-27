//! `chore` — run project tasks from a chorefile.
//!
//! The binary is a thin shell around the `chorefile` crate: it finds the file,
//! parses it, and hands the tree to the interpreter. Everything the CLI prints
//! about the language itself comes from `chorefile::spec`, so the reference
//! lives in exactly one place.

mod args;
mod help;
mod lint;
mod list;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chorefile::ast::File;
use chorefile::interp::{Interpreter, Mode, Repeat};
use chorefile::{Error, parse};

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

fn main() -> ExitCode {
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
            let source = Source::discover()?;
            let file = source.parse()?;
            writeln!(out, "\ntasks:")?;
            list::text(out, &file.tasks)?;
            Ok(OK)
        }
        Command::List { json } => {
            let source = Source::discover()?;
            let file = source.parse()?;
            if json {
                list::json(out, &file.tasks)?;
            } else {
                list::text(out, &file.tasks)?;
            }
            Ok(OK)
        }
        Command::Check => {
            let source = Source::discover()?;
            let errors = lint::report(out, &source.path, &source.text)?;
            Ok(if errors == 0 { OK } else { FAILED })
        }
        Command::Run { task, args } => {
            let source = Source::discover()?;
            let file = source.parse()?;
            run(&file, &source.root(), &task, &args, mode, repeat)
        }
    }
}

/// The chorefile that governs the working directory, and its text.
struct Source {
    path: PathBuf,
    text: String,
}

impl Source {
    fn discover() -> Result<Self, Exit> {
        let cwd = std::env::current_dir().map_err(|e| Exit::usage(e.to_string()))?;
        let path = chorefile::find(&cwd)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Exit::failed(format!("{}: {e}", path.display())))?;
        Ok(Self { path, text })
    }

    fn parse(&self) -> Result<File, Exit> {
        Ok(parse::parse(&self.text, &self.path)?)
    }

    /// `$ROOT`: the directory holding the chorefile.
    fn root(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }
}

fn run(
    file: &File,
    root: &Path,
    task: &str,
    args: &[String],
    mode: Mode,
    repeat: Repeat,
) -> Result<u8, Exit> {
    let mut interp = Interpreter::new(file, root, mode, repeat).with_output(Box::new(io::stdout()));
    if interp.task(task).is_none() {
        return Err(Exit::usage(format!(
            "no task `{task}` in {} (try `chore list`)",
            file_label(root)
        )));
    }
    let code = interp.run_task(task, args)?;
    // A task's own `exit 3` is a verdict worth keeping, so the process exit
    // code is the run's, not a flat 1.
    Ok(u8::try_from(code).unwrap_or(FAILED))
}

fn file_label(root: &Path) -> String {
    root.join(chorefile::FILE_NAME).display().to_string()
}
