//! The read-only variables every chorefile can rely on.

use std::path::{Path, PathBuf};

/// `macos`, `linux` or `windows`.
pub const OS: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else {
    "linux"
};

/// `x86_64` or `arm64`.
pub const ARCH: &str = if cfg!(target_arch = "aarch64") {
    "arm64"
} else {
    "x86_64"
};

/// The Windows toolchain: `gnu`, `msvc`, or empty everywhere else.
pub const ENV: &str = if cfg!(all(target_os = "windows", target_env = "msvc")) {
    "msvc"
} else if cfg!(all(target_os = "windows", target_env = "gnu")) {
    "gnu"
} else {
    ""
};

/// `.exe` on Windows, empty elsewhere.
pub const EXE: &str = if cfg!(target_os = "windows") {
    ".exe"
} else {
    ""
};

/// The variables that are set before a chorefile starts, in `list` order.
///
/// `$ROOT`, `$CWD`, `$TASK` and `$NOW` depend on the run, so they are filled
/// in by the interpreter rather than named here.
pub const BUILTIN_NAMES: &[&str] = &[
    "OS", "ARCH", "ENV", "PLATFORM", "EXE", "HOME", "ROOT", "CWD", "TASK", "NOW",
];

/// `$PLATFORM`, always `$OS-$ARCH`.
pub fn platform() -> String {
    format!("{OS}-{ARCH}")
}

/// `$HOME`. Empty if the platform will not say.
pub fn home() -> String {
    let key = if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    };
    std::env::var(key).unwrap_or_default()
}

/// The values that do not change over a run.
pub fn statics(root: &Path) -> Vec<(&'static str, String)> {
    vec![
        ("OS", OS.to_string()),
        ("ARCH", ARCH.to_string()),
        ("ENV", ENV.to_string()),
        ("PLATFORM", platform()),
        ("EXE", EXE.to_string()),
        ("HOME", home()),
        ("ROOT", display(root)),
    ]
}

/// Paths are written with `/` in a chorefile and reported back the same way,
/// on every platform. Conversion to `\` happens only when handing a path to
/// the OS.
pub fn display(path: &Path) -> String {
    let s = path.to_string_lossy();
    if cfg!(target_os = "windows") {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

/// Turn a chorefile path into one the host filesystem accepts.
pub fn to_native(path: &str) -> PathBuf {
    if cfg!(target_os = "windows") {
        PathBuf::from(path.replace('/', "\\"))
    } else {
        PathBuf::from(path)
    }
}
