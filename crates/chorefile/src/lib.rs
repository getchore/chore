//! Parser and interpreter for the chorefile task language.
//!
//! A chorefile is a small subset of POSIX sh. It is never handed to a host
//! shell: `chorefile` lexes it, parses it, and runs it itself, so a chorefile
//! behaves identically on macOS, Linux and Windows.

pub mod ast;
pub mod builtins;
pub mod check;
pub mod error;
pub mod exec;
pub mod interp;
pub mod lex;
pub mod parse;
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
pub const RESERVED_TASKS: &[&str] = &["list", "help", "check", "spec", "completions", "init"];

/// Find the chorefile governing `from`: the nearest one at `from` or above it.
///
/// Walking up is what makes `chore build` work from anywhere inside a project,
/// and the directory holding the file it finds becomes `$ROOT`.
pub fn find(from: &std::path::Path) -> Result<std::path::PathBuf> {
    for dir in from.ancestors() {
        let candidate = dir.join(FILE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(Error::NotFound {
        from: from.to_path_buf(),
    })
}
