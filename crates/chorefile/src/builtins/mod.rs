//! The commands `chore` implements itself, so a chorefile never needs
//! `curl`, `unzip`, `tar`, `cp` or `rm`.

/// Every builtin name. These are reserved: a task may not shadow one, and
/// `check` reports it if one tries.
pub const NAMES: &[&str] = &[
    "download", "extract", "archive", "copy", "move", "remove", "mkdir", "chmod", "which", "find",
    "read", "write", "sha256", "exists", "changed", "echo", "env", "fail", "sleep",
];

/// Non-portable commands `check` flags, with the builtin to use instead.
pub const REPLACEMENTS: &[(&str, &str)] = &[
    ("curl", "download"),
    ("wget", "download"),
    ("unzip", "extract"),
    ("tar", "extract or archive"),
    ("cp", "copy"),
    ("mv", "move"),
    ("rm", "remove"),
    ("mkdir -p", "mkdir"),
    ("cat", "read"),
    ("shasum", "sha256"),
    ("sha256sum", "sha256"),
    ("test", "exists"),
    ("sleep", "sleep"),
];

pub fn is_builtin(name: &str) -> bool {
    NAMES.contains(&name)
}
pub mod fs;
pub mod net;
pub mod pack;
pub mod state;
