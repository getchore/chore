//! The command line, parsed by hand.
//!
//! The grammar is tiny but two of its rules are unusual enough that a general
//! argument parser gets them wrong: `--dry` and `--force` may appear anywhere
//! after the task name, and *every other* flag after the task name belongs to
//! the task, not to `chore`. A parser that knows the whole grammar up front
//! would reject `chore test --nocapture` instead of passing it through.

use chorefile::RESERVED_TASKS;
use chorefile::interp::{Mode, Repeat};

/// The two flags `chore` claims for itself wherever they appear.
const DRY: &str = "--dry";
const FORCE: &str = "--force";

/// What the user asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `chore <task> [args...]`
    Run { task: String, args: Vec<String> },
    /// `chore list [--json]`
    List { json: bool },
    /// `chore help [builtin]`
    Help { topic: Option<String> },
    /// `chore check`
    Check,
    /// `chore spec`
    Spec,
    /// `chore` with nothing to do: usage, plus the task list.
    Usage,
    /// `chore --version`
    Version,
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

    // The four subcommands shadow tasks of the same name, so `chore list` is
    // never ambiguous — that is why they are reserved in the first place.
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
            0 => Ok(Command::List { json: false }),
            1 if args[0] == "--json" => Ok(Command::List { json: true }),
            _ => Err(UsageError("usage: chore list [--json]".into())),
        },
        "help" => match args.len() {
            0 => Ok(Command::Help { topic: None }),
            1 if !args[0].starts_with('-') => Ok(Command::Help {
                topic: Some(args[0].clone()),
            }),
            _ => Err(UsageError("usage: chore help [builtin]".into())),
        },
        "check" if args.is_empty() => Ok(Command::Check),
        "spec" if args.is_empty() => Ok(Command::Spec),
        other => Err(UsageError(format!("usage: chore {other}"))),
    }
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
        assert_eq!(parse_args(&["list"]).command, Command::List { json: false });
        assert_eq!(parse_args(&["spec"]).command, Command::Spec);
    }

    #[test]
    fn leading_flag_before_a_task_is_ours() {
        let got = parse_args(&["--dry", "build"]);
        assert_eq!(got.mode, Mode::Dry);
    }

    #[test]
    fn unknown_leading_flag_is_a_usage_error() {
        assert!(parse(["--nope".to_string()]).is_err());
    }
}
