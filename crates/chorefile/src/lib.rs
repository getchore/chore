//! Parser and interpreter for the chorefile task language.
//!
//! A chorefile is a small subset of POSIX sh. It is never handed to a host
//! shell: `chorefile` lexes it, parses it, and runs it itself, so a chorefile
//! behaves identically on macOS, Linux and Windows.

pub mod ast;
pub mod builtins;
pub mod check;
pub mod dotenv;
pub mod error;
pub mod exec;
pub mod interp;
pub mod lex;
pub mod parse;
pub mod require;
pub mod resolve;
pub mod spec;
pub mod vars;

pub use error::{Error, Result};

/// The one filename `chore` looks for, walking up from the working directory.
///
/// Lowercase only: on case-insensitive filesystems `Chorefile` and `chorefile`
/// are the same file and on Linux they are not, so accepting both spellings
/// makes a chorefile that resolves differently per platform.
pub const FILE_NAME: &str = "chorefile";

/// Extension for files pulled in with `include`.
pub const FILE_EXT: &str = "chore";

/// Separator between an include's `as` namespace and a task name (`libs::build`).
pub const NAMESPACE_SEP: &str = "::";

/// Subcommand names that cannot be used as task names.
///
/// `check` is deliberately absent: it is the one subcommand a chorefile may
/// take back. Every Cargo project wants `task check { cargo check }`, and
/// nothing depends on `chore check` meaning the lint the way completion
/// scripts and tooling depend on `chore list` — so `chore check` runs the task
/// where one exists, and `chore --check` is the lint that never yields.
pub const RESERVED_TASKS: &[&str] = &["list", "help", "spec", "completions", "init"];

/// Find the chorefile governing `from`: the nearest one at `from` or above it.
///
/// Walking up is what makes `chore build` work from anywhere inside a project,
/// and the directory holding the file it finds becomes `$ROOT`.
///
/// The name is matched against the directory listing, not by asking the
/// filesystem whether `dir/chorefile` opens: on macOS and Windows that call
/// says yes to `Chorefile`, and the same tree then fails on Linux. Reading the
/// entries is what makes discovery one rule on every platform, and what lets
/// the error name the near miss it walked past.
pub fn find(from: &std::path::Path) -> Result<std::path::PathBuf> {
    for dir in from.ancestors() {
        if let Some(found) = exact(dir, FILE_NAME) {
            return Ok(found);
        }
    }
    Err(Error::NotFound {
        from: from.to_path_buf(),
        near: near_miss(from),
    })
}

/// `dir/name` if an entry spelled exactly `name` exists there and is a file.
///
/// `None` on a directory that cannot be listed, which is also what a caller
/// that then tries to open the file will hear from the filesystem.
pub fn exact(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_name() == name {
            let path = entry.path();
            return path.is_file().then_some(path);
        }
    }
    None
}

/// The file in `dir` someone most likely meant when no chorefile was found
/// there: another spelling of the name, or a `.chore` fragment sitting on
/// its own. The first one found, so the message stays one line.
pub fn near_miss(dir: &std::path::Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    let spelling = names
        .iter()
        .find(|n| n.eq_ignore_ascii_case(FILE_NAME) && n.as_str() != FILE_NAME);
    let fragment = names.iter().find(|n| {
        std::path::Path::new(n)
            .extension()
            .is_some_and(|x| x == FILE_EXT)
    });
    spelling.or(fragment).cloned()
}
