//! `chore` — run project tasks from a chorefile.
//!
//! The binary is a thin shell around the `chorefile` crate: it finds the file,
//! resolves its `include`s into one tree, and hands that to the interpreter.
//! Everything the CLI prints about the language itself comes from
//! `chorefile::spec`, so the reference lives in exactly one place.

mod args;
mod completions;
mod help;
mod lint;
mod list;
mod style;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chorefile::interp::{Interpreter, Mode, Repeat};
use chorefile::resolve::{Merged, Sources};
use chorefile::{Error, parse, resolve};

use args::{Command, Invocation, ListFormat, UsageError};
use style::Style;

/// Exit codes, as promised in docs/SPEC.md.
const OK: u8 = 0;
const FAILED: u8 = 1;
const USAGE: u8 = 2;

/// The subcommands, as `chore --help` lists them: the word itself, the
/// arguments it takes, and what it is for. Split into three so the word can be
/// coloured without the colour landing on the alignment.
const SUBCOMMANDS: &[(&str, &str, &str)] = &[
    ("list", "[--json]", "tasks and descriptions"),
    ("help", "[builtin]", "syntax and builtins, or one builtin"),
    ("check", "", "lint without running"),
    ("spec", "", "full reference as JSON, for agents"),
    ("completions", "[shell]", "tab completion for task names"),
];

/// The two flags `chore` keeps for itself, and what they do.
const FLAGS: &[(&str, &str)] = &[
    ("--dry", "echo commands without side effects"),
    ("--force", "disable run-once"),
];

/// Width of the left column in the subcommand list, and in the flag list.
/// Literal, because escape sequences are bytes of zero width: padding a
/// styled string with `{:<width$}` would pad by however long the escapes are
/// and tear the column apart, so every line here pads its plain text and
/// colours only the word.
const SUBCOMMAND_COLUMN: usize = 21;
const FLAG_COLUMN: usize = 11;

/// The usage block, for `chore --help` and for a bare `chore` with no
/// chorefile to talk about instead.
fn usage(style: Style) -> String {
    let mut text = format!(
        "usage: chore <task> [args...] [{}] [{}]\n",
        style.accent("--dry"),
        style.accent("--force")
    );
    for (name, args, meaning) in SUBCOMMANDS {
        let left = if args.is_empty() {
            (*name).to_string()
        } else {
            format!("{name} {args}")
        };
        let pad = " ".repeat(SUBCOMMAND_COLUMN.saturating_sub(left.chars().count()));
        let left = left.replacen(name, &style.accent(name), 1);
        text.push_str(&format!("       chore {left}{pad}{meaning}\n"));
    }
    text.push('\n');
    for (flag, meaning) in FLAGS {
        let pad = " ".repeat(FLAG_COLUMN.saturating_sub(flag.chars().count()));
        text.push_str(&format!("  {}{pad}{meaning}\n", style.accent(flag)));
    }
    // The loop left a newline on the end and `writeln!` will add the other.
    text.pop();
    text
}

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
    // Asked once, here, and carried down rather than re-derived: `is_terminal`
    // is a syscall, and a decision made in two places is a decision that can
    // disagree with itself halfway down a page of output.
    let style = Style::stdout();

    let code = match args::parse(argv) {
        Err(UsageError(message)) => {
            complain(&message);
            USAGE
        }
        Ok(invocation) => match dispatch(invocation, &mut out, style) {
            Ok(code) => code,
            Err(exit) => {
                complain(&exit.message);
                exit.code
            }
        },
    };
    let _ = out.flush();
    ExitCode::from(code)
}

/// One line to stderr, in the `chore: ...` shape every failure uses.
///
/// The style is stderr's own: `chore build > log` on a terminal has a pipe on
/// stdout and a person on stderr, and the error is the line that person is
/// still there for.
fn complain(message: &str) {
    let style = Style::stderr();
    let _ = writeln!(io::stderr(), "{} {message}", style.error("chore:"));
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

fn dispatch(invocation: Invocation, out: &mut dyn Write, style: Style) -> Result<u8, Exit> {
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
            Some(name) => help::builtin(out, &name, style).map(|()| OK),
            // The usage block used to be the first thing bare `chore`
            // printed. Bare `chore` leads with the task list now, so the
            // block moves here, in front of the language reference: someone
            // who typed `help` is the one who wants both.
            None => {
                writeln!(out, "{}\n", usage(style))?;
                help::overview(out, style).map(|()| OK)
            }
        },
        Command::Spec => {
            writeln!(out, "{}", chorefile::spec::json())?;
            Ok(OK)
        }
        // Bare `chore` in a project answers the question the user actually
        // has, which is "what can I run here". The usage block is a page of
        // grammar they did not ask for, so it moves behind `chore --help` and
        // leaves one line pointing at it. With no chorefile there is no list
        // to lead with, so the old behaviour stands: print the usage block,
        // then fail on stderr the way every other missing-chorefile does.
        Command::Usage => {
            let loaded = match Loaded::discover() {
                Ok(loaded) => loaded,
                Err(exit) => {
                    writeln!(out, "{}", usage(style))?;
                    return Err(exit);
                }
            };
            writeln!(out, "{}", style.bold("Available tasks:"))?;
            list::text(out, &loaded.merged, style)?;
            writeln!(
                out,
                "\n{} to run one, {} for the language",
                style.accent("chore <task>"),
                style.accent("chore help")
            )?;
            Ok(OK)
        }
        Command::List { format } => {
            let loaded = Loaded::discover()?;
            match format {
                ListFormat::Text => list::text(out, &loaded.merged, style)?,
                // `--json` and `--names` are read by programs, so they are
                // plain no matter what the terminal would have allowed.
                ListFormat::Json => list::json(out, &loaded.merged)?,
                ListFormat::Names => list::names(out, &loaded.merged)?,
            }
            Ok(OK)
        }
        // Completion is about the shell, not about a project, so it works
        // with no chorefile anywhere. The script it prints is what discovers
        // the chorefile later, once the user hits Tab somewhere.
        // A shell named on the command line means a machine is asking, so
        // print the script for it to redirect. Bare `chore completions` means
        // a person is asking, so say what to add and where.
        Command::Completions { shell, write } => {
            if write {
                completions::write(out, shell.or_else(completions::Shell::detect))?;
            } else if let Some(shell) = shell {
                completions::script(out, shell)?;
            } else {
                completions::guide(out, completions::Shell::detect())?;
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
            let errors = lint::report(out, &findings, sources, style)?;
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
