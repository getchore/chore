//! The runtime contract shared by the interpreter and the builtins.
//!
//! Everything that runs a command — a task, a builtin, a program on `PATH` —
//! takes a [`Ctx`] and returns an [`Output`]. Keeping one shape for all three
//! is what lets `|`, `>` and `$(...)` treat them identically.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// What a command did.
#[derive(Debug, Default, Clone)]
pub struct Output {
    pub code: i32,
    /// Captured only when the caller asked for it — a plain command streams
    /// to the terminal instead, so a long build is not silent.
    pub stdout: Vec<u8>,
    /// Only a captured program on `PATH` fills this in. A builtin writes its
    /// diagnostics to [`Ctx::err`], which the interpreter points at the
    /// process stderr or at a `2>` file; nothing reads a builtin's
    /// `Output::stderr`.
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn ok() -> Self {
        Self::default()
    }

    pub fn failed(code: i32) -> Self {
        Self {
            code,
            ..Self::default()
        }
    }

    pub fn success(&self) -> bool {
        self.code == 0
    }

    /// `$(...)` yields stdout with surrounding whitespace removed.
    pub fn captured(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_string()
    }
}

/// Everything a command needs to run.
pub struct Ctx<'a> {
    /// Fully interpolated argv. `args[0]` is the command name.
    pub args: &'a [String],
    /// The current directory, tracked by the interpreter rather than the
    /// process, so `cd` inside one task cannot leak into another.
    pub cwd: &'a Path,
    /// The directory holding the top-level chorefile, for `$ROOT`.
    pub root: &'a Path,
    /// Piped input, when this command is on the right of a `|`.
    pub stdin: Option<&'a [u8]>,
    /// `--dry`: echo, and do nothing that has an effect. Builtins that only
    /// read still run, because conditions and captures depend on them.
    pub dry: bool,
    /// Where the command should write, when it is not captured.
    pub out: &'a mut dyn Write,
    /// Where diagnostics go. Separate from [`Ctx::out`] because the two can
    /// have different destinations: `cmd > file` captures stdout while stderr
    /// still streams, and `cmd 2> file` is the other way round. A builtin that
    /// stuffed its diagnostic into [`Output::stderr`] instead would lose it
    /// entirely for a streamed command, and would leave `2>` writing an empty
    /// file — a redirect that lied about what it caught.
    pub err: &'a mut dyn Write,
    /// True only when [`Ctx::out`] really is the user's terminal: the command
    /// is streaming *and* stdout is a tty.
    ///
    /// A builtin must consult this rather than probing `stdout().is_terminal()`
    /// itself. Under `$(...)` or on the left of a `|` the sink is a buffer
    /// while the process stdout may still be a tty, so the direct probe would
    /// answer "yes" and bleed `\r` progress redraws into the capture.
    pub interactive: bool,
}

impl Ctx<'_> {
    /// Arguments after the command name.
    pub fn rest(&self) -> &[String] {
        self.args.get(1..).unwrap_or_default()
    }

    /// Resolve a chorefile path (always written with `/`) against the current
    /// directory, converting separators for the host.
    pub fn path(&self, arg: &str) -> PathBuf {
        let p = crate::vars::to_native(arg);
        if p.is_absolute() { p } else { self.cwd.join(p) }
    }
}

/// The signature every builtin implements.
pub type Builtin = fn(&mut Ctx<'_>) -> Result<Output>;
