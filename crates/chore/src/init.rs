//! `chore init`: write a starter chorefile into the working directory.
//!
//! The gap this closes is the one between reading the spec and having
//! something to edit. `chore help` explains the language and `chore spec`
//! hands an agent the whole reference, but neither leaves a file behind, and
//! the first chorefile is exactly the thing a newcomer has no template for.
//!
//! What it writes is deliberately not a tutorial. Four tasks, one of them
//! calling the other two, is enough to show the shapes that matter: a task
//! body, the comment above a task becoming its description in `chore list`,
//! and a task calling another by name. Anything longer would be read once and
//! deleted, and the language reference already lives in `chore help`.
//!
//! Nothing here discovers a chorefile. `init` is what someone runs when there
//! is no chorefile to find, and walking up to a parent project's file would
//! only give the command an opinion about a directory the user did not name.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::style::Style;

/// The starter chorefile, verbatim.
///
/// It must pass `chore check` with zero findings, which is why every command
/// in it is a builtin: `check` reports `curl`, `cp`, `rm` and friends as
/// non-portable, and a file that lints against the tool that wrote it is a
/// poor first impression. The e2e tests run `check` on what this writes.
const STARTER: &str = r#"# Project tasks. `chore list` shows them, `chore <task>` runs one.
# The comment directly above a task is the description `chore list` prints.

# Say hello, so there is something to run before anything else is filled in.
task hello {
    echo "hello from chore"
}

# Build the project.
task build {
    echo "nothing to build yet"
}

# Run the tests.
task test {
    echo "no tests yet"
}

# Build and test: the gate to run before pushing.
task ci {
    build
    test
}
"#;

/// Write `./chorefile`, or explain why it was left alone.
///
/// The refusal is the whole safety story: a chorefile is hand-written work,
/// and a command that overwrote one would destroy it with no way back. So the
/// existence check is not a convenience, and there is no `--force` to get past
/// it: editing the file that is already there is what the user wanted anyway.
///
/// `Ok(false)` means the file existed and nothing was written; the caller
/// turns that into the usage exit code.
pub fn write(out: &mut dyn Write, dir: &Path, style: Style) -> io::Result<bool> {
    let path = dir.join(chorefile::FILE_NAME);
    // `try_exists` rather than `exists`, because `exists` answers false for a
    // path it merely could not stat. A permissions problem on the directory
    // would read as "no chorefile here" and lead straight into an overwrite.
    // An error is not a licence to write, so it counts as present.
    if path.try_exists().unwrap_or(true) {
        return Ok(false);
    }
    std::fs::write(&path, STARTER)?;
    writeln!(
        out,
        "wrote {}\n\n{} to see the tasks, {} to run one",
        display(&path),
        style.accent("chore list"),
        style.accent("chore hello")
    )?;
    Ok(true)
}

/// What to say when the file is already there.
///
/// Named rather than inlined at the call site so the message and the file it
/// talks about are decided in one place, next to the code that declined.
pub fn occupied(dir: &Path) -> String {
    format!(
        "{} already exists, and init never overwrites one (edit it, or run init elsewhere)",
        display(&dir.join(chorefile::FILE_NAME))
    )
}

/// The path as a person would type it: `chorefile` in the directory they are
/// standing in, not the absolute path they already know they are at.
fn display(path: &Path) -> String {
    match path.file_name() {
        Some(name) => PathBuf::from(".").join(name).display().to_string(),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file this command writes is the one example of the language that
    /// every new user reads, so a stray reserved name in it would teach the
    /// wrong thing and `check` would flag it the moment they ran it.
    #[test]
    fn the_starter_defines_no_reserved_task() {
        for line in STARTER.lines() {
            if let Some(rest) = line.strip_prefix("task ") {
                let name = rest.split_whitespace().next().unwrap_or_default();
                assert!(
                    !chorefile::RESERVED_TASKS.contains(&name),
                    "starter defines reserved task `{name}`"
                );
            }
        }
    }

    /// `chore list` reads the comment directly above a `task` as its
    /// description, and demonstrating that convention is half the point of
    /// shipping a starter at all.
    #[test]
    fn every_starter_task_has_a_description_above_it() {
        let lines: Vec<&str> = STARTER.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("task ") {
                let above = i.checked_sub(1).map(|j| lines[j]).unwrap_or_default();
                assert!(above.starts_with('#'), "no description above `{line}`");
            }
        }
    }
}
