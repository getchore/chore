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

/// The rustc target triple for this host, spelled the way `rustc -vV` and
/// `cargo --target` spell it.
///
/// `$OS` and `$ARCH` answer "what am I running on"; a Rust, Zig or Tauri
/// toolchain wants the triple instead, and the mapping is not derivable from
/// the pair — `linux` alone cannot say `gnu` or `musl`, and `windows` alone
/// cannot say `msvc` or `gnu`. Every Rust chorefile was writing the same
/// five-branch table by hand, so chore states the fact it already knows.
///
/// The `target_env` arm is what keeps a musl build of chore — which is what
/// the Linux release is — from claiming a gnu triple and sending cargo after
/// a toolchain that is not installed.
///
/// Empty on any target not named here. A triple assembled from parts would be
/// well-formed enough for cargo to accept and then build the wrong thing;
/// empty fails at the point of use, where the message can be read.
pub const TRIPLE: &str = if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
    "aarch64-apple-darwin"
} else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
    "x86_64-apple-darwin"
} else if cfg!(all(
    target_arch = "x86_64",
    target_os = "linux",
    target_env = "gnu"
)) {
    "x86_64-unknown-linux-gnu"
} else if cfg!(all(
    target_arch = "aarch64",
    target_os = "linux",
    target_env = "gnu"
)) {
    "aarch64-unknown-linux-gnu"
} else if cfg!(all(
    target_arch = "x86_64",
    target_os = "linux",
    target_env = "musl"
)) {
    "x86_64-unknown-linux-musl"
} else if cfg!(all(
    target_arch = "aarch64",
    target_os = "linux",
    target_env = "musl"
)) {
    "aarch64-unknown-linux-musl"
} else if cfg!(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc"
)) {
    "x86_64-pc-windows-msvc"
} else if cfg!(all(
    target_arch = "aarch64",
    target_os = "windows",
    target_env = "msvc"
)) {
    "aarch64-pc-windows-msvc"
} else if cfg!(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "gnu",
    // `*-windows-gnullvm` also reports `target_env = "gnu"`; only the ABI
    // tells the two apart, and answering `-gnu` there would be a wrong triple.
    target_abi = ""
)) {
    "x86_64-pc-windows-gnu"
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
    "OS", "ARCH", "ENV", "PLATFORM", "TRIPLE", "EXE", "HOME", "ROOT", "CWD", "TASK", "NOW",
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
        ("TRIPLE", TRIPLE.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The suite runs on macOS, both Linux arches and both Windows arches, so
    /// the assertion has to hold on every one of them: `$TRIPLE` may not
    /// disagree with the `$OS`, `$ARCH` and `$ENV` this same module reports.
    #[test]
    fn triple_agrees_with_the_other_platform_variables() {
        let (arch, rest) = TRIPLE.split_once('-').expect("arch-vendor-os[-env]");
        assert_eq!(
            arch,
            match ARCH {
                "arm64" => "aarch64",
                other => other,
            }
        );
        let os = match OS {
            "macos" => "apple-darwin",
            "windows" => "pc-windows",
            _ => "unknown-linux",
        };
        assert!(rest.starts_with(os), "{TRIPLE} is not a {OS} triple");
        // `$ENV` speaks only for Windows; on Linux the triple carries the libc
        // that `$ENV` deliberately does not report.
        if OS == "windows" {
            assert_eq!(rest.trim_start_matches(os).trim_start_matches('-'), ENV);
        }
    }

    /// Empty is the answer for a target the table does not name, so it must not
    /// become the answer for one of the targets chore actually ships.
    #[test]
    fn triple_is_known_on_every_supported_host() {
        assert!(!TRIPLE.is_empty(), "no triple for this host");
    }

    /// The agreement test would still pass if every branch were off by the same
    /// mistake, so one literal is pinned per host the suite can run on. Cargo's
    /// `TARGET` is handed to build scripts only, not to a test binary, and this
    /// crate has no build script to forward it — hence the `cfg` arms.
    #[test]
    fn triple_matches_the_known_value_for_this_host() {
        #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
        assert_eq!(TRIPLE, "aarch64-apple-darwin");
        #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
        assert_eq!(TRIPLE, "x86_64-apple-darwin");
        #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
        assert_eq!(TRIPLE, "x86_64-unknown-linux-gnu");
        #[cfg(all(target_arch = "aarch64", target_os = "linux", target_env = "gnu"))]
        assert_eq!(TRIPLE, "aarch64-unknown-linux-gnu");
        #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "musl"))]
        assert_eq!(TRIPLE, "x86_64-unknown-linux-musl");
        #[cfg(all(target_arch = "aarch64", target_os = "linux", target_env = "musl"))]
        assert_eq!(TRIPLE, "aarch64-unknown-linux-musl");
        #[cfg(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"))]
        assert_eq!(TRIPLE, "x86_64-pc-windows-msvc");
        #[cfg(all(target_arch = "aarch64", target_os = "windows", target_env = "msvc"))]
        assert_eq!(TRIPLE, "aarch64-pc-windows-msvc");
    }

    /// A value that is not in both lists is either invisible to `check` or
    /// unset at run time, and the two failures look nothing alike.
    #[test]
    fn triple_is_both_named_and_set() {
        assert!(BUILTIN_NAMES.contains(&"TRIPLE"));
        let statics = statics(Path::new("/root"));
        assert_eq!(
            statics.iter().find(|(name, _)| *name == "TRIPLE"),
            Some(&("TRIPLE", TRIPLE.to_string()))
        );
    }
}
