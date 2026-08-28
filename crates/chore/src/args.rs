//! The command line, parsed by hand.
//!
//! The grammar is tiny but two of its rules are unusual enough that a general
//! argument parser gets them wrong: `--dry` and `--force` may appear anywhere
//! after the task name, and *every other* flag after the task name belongs to
//! the task, not to `chore`. A parser that knows the whole grammar up front
//! would reject `chore test --nocapture` instead of passing it through.

use chorefile::RESERVED_TASKS;
use chorefile::interp::{Mode, Repeat};

use crate::completions::Shell;

/// The two flags `chore` claims for itself wherever they appear.
const DRY: &str = "--dry";
const FORCE: &str = "--force";

/// What the user asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `chore <task> [args...]`
    Run { task: String, args: Vec<String> },
    /// `chore list [--json|--names]`
    List { format: ListFormat },
    /// `chore help [builtin]`
    Help { topic: Option<String> },
    /// `chore check`
    Check,
    /// `chore spec`
    Spec,
    /// `chore completions [shell] [--write]`
    Completions { shell: Option<Shell>, write: bool },
    /// `chore init`
    Init,
    /// `chore` with nothing to do: usage, plus the task list.
    Usage,
    /// `chore --version`
    Version,
}

/// How `chore list` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    /// Aligned columns, for a person.
    Text,
    /// One object per task, for a tool.
    Json,
    /// `name<TAB>description`, for a completion script.
    Names,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub command: Command,
    pub mode: Mode,
    pub repeat: Repeat,
}

/// A usage error: printed to stderr, exit 2.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError(pub String);

pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Invocation, UsageError> {
    let mut mode = Mode::Run;
    let mut repeat = Repeat::Once;
    let mut rest = argv.into_iter().peekable();

    // Before the name, `--dry` and `--force` are the only flags that can
    // appear: there is no task yet to pass anything else on to.
    let name = loop {
        match rest.next() {
            Some(arg) if arg == DRY => mode = Mode::Dry,
            Some(arg) if arg == FORCE => repeat = Repeat::Always,
            Some(arg) if arg == "--help" || arg == "-h" => {
                break Some("help".to_string());
            }
            Some(arg) if arg == "--version" || arg == "-V" => {
                return Ok(Invocation {
                    command: Command::Version,
                    mode,
                    repeat,
                });
            }
            Some(arg) if arg.starts_with('-') && arg != "-" => {
                return Err(UsageError(format!("unknown option `{arg}`")));
            }
            other => break other,
        }
    };

    let Some(name) = name else {
        return Ok(Invocation {
            command: Command::Usage,
            mode,
            repeat,
        });
    };

    // A subcommand shadows a task of the same name, so `chore list` is never
    // ambiguous. That is why the names are reserved in the first place.
    let command = if RESERVED_TASKS.contains(&name.as_str()) {
        let tail: Vec<String> = rest.collect();
        subcommand(&name, tail)?
    } else {
        let mut args = Vec::new();
        // After `--` nothing is a flag any more, so a task can still receive a
        // literal `--dry`.
        let mut literal = false;
        for arg in rest {
            match arg.as_str() {
                _ if literal => args.push(arg),
                "--" => literal = true,
                DRY => mode = Mode::Dry,
                FORCE => repeat = Repeat::Always,
                _ => args.push(arg),
            }
        }
        Command::Run { task: name, args }
    };

    Ok(Invocation {
        command,
        mode,
        repeat,
    })
}

fn subcommand(name: &str, args: Vec<String>) -> Result<Command, UsageError> {
    match name {
        "list" => match args.len() {
            0 => Ok(Command::List {
                format: ListFormat::Text,
            }),
            1 if args[0] == "--json" => Ok(Command::List {
                format: ListFormat::Json,
            }),
            1 if args[0] == "--names" => Ok(Command::List {
                format: ListFormat::Names,
            }),
            _ => Err(UsageError("usage: chore list [--json|--names]".into())),
        },
        "help" => match args.len() {
            0 => Ok(Command::Help { topic: None }),
            1 if !args[0].starts_with('-') => Ok(Command::Help {
                topic: Some(args[0].clone()),
            }),
            _ => Err(UsageError("usage: chore help [builtin]".into())),
        },
        "check" if args.is_empty() => Ok(Command::Check),
        // `init` takes nothing: there is one starter chorefile and one place
        // it goes, so an argument here is a misunderstanding worth naming
        // rather than something to quietly ignore.
        "init" if args.is_empty() => Ok(Command::Init),
        "completions" => completions(args),
        "spec" if args.is_empty() => Ok(Command::Spec),
        other => Err(UsageError(format!("usage: chore {other}"))),
    }
}

/// `chore completions [bash|zsh|fish|powershell] [--write]`
///
/// With no shell, `$SHELL` decides and the output is advice. With a shell, the
/// output is the script, so it can be redirected into a file.
fn completions(args: Vec<String>) -> Result<Command, UsageError> {
    let mut shell = None;
    let mut write = false;
    for arg in args {
        match arg.as_str() {
            "--write" => write = true,
            name if !name.starts_with('-') => {
                let Some(parsed) = Shell::parse(name) else {
                    return Err(UsageError(format!(
                        "unknown shell `{name}` (bash, zsh, fish, powershell)"
                    )));
                };
                shell = Some(parsed);
            }
            other => {
                return Err(UsageError(format!(
                    "usage: chore completions [shell] [--write] (got `{other}`)"
                )));
            }
        }
    }
    Ok(Command::Completions { shell, write })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Invocation {
        parse(args.iter().map(|s| s.to_string())).expect("valid")
    }

    #[test]
    fn flags_may_follow_the_task_name() {
        let got = parse_args(&["build", "--dry", "x", "--force"]);
        assert_eq!(got.mode, Mode::Dry);
        assert_eq!(got.repeat, Repeat::Always);
        assert_eq!(
            got.command,
            Command::Run {
                task: "build".into(),
                args: vec!["x".into()],
            }
        );
    }

    #[test]
    fn unknown_flags_reach_the_task() {
        let got = parse_args(&["test", "--nocapture", "-q"]);
        assert_eq!(
            got.command,
            Command::Run {
                task: "test".into(),
                args: vec!["--nocapture".into(), "-q".into()],
            }
        );
    }

    #[test]
    fn double_dash_passes_our_own_flags_through() {
        let got = parse_args(&["build", "--", "--dry"]);
        assert_eq!(got.mode, Mode::Run);
        assert_eq!(
            got.command,
            Command::Run {
                task: "build".into(),
                args: vec!["--dry".into()],
            }
        );
    }

    #[test]
    fn subcommands_shadow_tasks() {
        assert_eq!(
            parse_args(&["list"]).command,
            Command::List {
                format: ListFormat::Text
            }
        );
        assert_eq!(parse_args(&["spec"]).command, Command::Spec);
    }

    #[test]
    fn leading_flag_before_a_task_is_ours() {
        let got = parse_args(&["--dry", "build"]);
        assert_eq!(got.mode, Mode::Dry);
    }

    #[test]
    fn list_formats() {
        assert_eq!(
            parse_args(&["list", "--names"]).command,
            Command::List {
                format: ListFormat::Names
            }
        );
        assert!(parse(["list".into(), "--nope".into()]).is_err());
    }

    #[test]
    fn completions_takes_a_shell_and_a_flag_in_either_order() {
        let expected = Command::Completions {
            shell: Some(Shell::Zsh),
            write: true,
        };
        assert_eq!(
            parse_args(&["completions", "zsh", "--write"]).command,
            expected
        );
        assert_eq!(
            parse_args(&["completions", "--write", "zsh"]).command,
            expected
        );
    }

    #[test]
    fn bare_completions_decides_for_itself() {
        assert_eq!(
            parse_args(&["completions"]).command,
            Command::Completions {
                shell: None,
                write: false
            }
        );
    }

    #[test]
    fn an_unknown_shell_says_which_ones_exist() {
        let err = parse(["completions".into(), "nushell".into()]).unwrap_err();
        assert!(err.0.contains("bash, zsh, fish, powershell"), "{}", err.0);
    }

    #[test]
    fn completions_shadows_a_task_of_that_name() {
        assert!(matches!(
            parse_args(&["completions"]).command,
            Command::Completions { .. }
        ));
    }

    #[test]
    fn init_takes_no_arguments() {
        assert_eq!(parse_args(&["init"]).command, Command::Init);
        assert!(parse(["init".into(), "--force".into()]).is_err());
    }

    #[test]
    fn unknown_leading_flag_is_a_usage_error() {
        assert!(parse(["--nope".to_string()]).is_err());
    }
}
