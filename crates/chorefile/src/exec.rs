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

/// The environment a command runs in: the names `env` has bound, layered
/// over the process environment.
///
/// Chore never calls `std::env::set_var`. A set belongs to the frame that
/// wrote it, exactly as `cd` and a local do — a `run` task that sets
/// `TERRA_SOCKET` must not still be setting it for the task called after it —
/// and `parallel` puts several interpreters on several threads, where writing
/// the process environment is racy and, in edition 2024, unsafe. So the
/// bindings live here instead: the interpreter layers them onto every child
/// process with [`std::process::Command::env`], and every builtin that reads
/// the environment reads it through [`EnvOverlay::get`] rather than through
/// `std::env::var`, so `env HTTPS_PROXY ...` is visible to the `download`
/// that follows it.
#[derive(Default, Clone)]
pub struct EnvOverlay {
    /// Innermost last, so a search from the back finds the binding in force
    /// and a replay from the front lets it win on a child's command line. A
    /// `Vec` rather than a map because a scope is left by truncating back to
    /// the length it had on the way in, which is the whole of the scoping.
    layers: Vec<(String, Option<String>)>,
}

impl EnvOverlay {
    /// What `name` holds for a command running now: the innermost binding, or
    /// the process environment when nothing bound it. `None` is "unset", the
    /// answer `env NAME` reports as exit 1.
    pub fn get(&self, name: &str) -> Option<String> {
        for (bound, value) in self.layers.iter().rev() {
            if same_name(bound, name) {
                return value.clone();
            }
        }
        std::env::var(name).ok()
    }

    /// Bind a name from here until the scope that opened is unwound.
    pub(crate) fn set(&mut self, name: String, value: Option<String>) {
        self.layers.push((name, value));
    }

    /// The depth to hand back to [`unwind`](Self::unwind) when this scope ends.
    pub(crate) fn depth(&self) -> usize {
        self.layers.len()
    }

    /// Drop every binding made since `depth` was taken.
    pub(crate) fn unwind(&mut self, depth: usize) {
        self.layers.truncate(depth);
    }

    /// Every binding, outermost first, for a process about to be spawned.
    pub(crate) fn bindings(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.layers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_deref()))
    }
}

/// Windows environment names are case-insensitive, so `env path ...` and
/// `$PATH` have to be one entry there and two everywhere else. ASCII case is
/// the whole rule: it is what the Windows API itself applies.
fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
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
    /// The name of the task this command is running inside, as `$TASK` gives
    /// it, or `""` at the top level. `changed` keys its state on it, so two
    /// tasks watching the same paths keep separate answers.
    pub task: &'a str,
    /// Piped input, when this command is on the right of a `|`.
    pub stdin: Option<&'a [u8]>,
    /// The environment as this command sees it. A builtin that wants a
    /// variable asks this rather than `std::env::var`, so a name `env` bound
    /// in an enclosing frame is visible to it and one bound in a sibling
    /// `parallel` task is not.
    pub env: &'a EnvOverlay,
    /// `--dry`: echo, and do nothing that has an effect. Builtins that only
    /// read still run, because conditions and captures depend on them.
    pub dry: bool,
    /// `--force`: the run was asked to do its work again whatever it did last
    /// time. Only an up-to-date check has anything to do with this; `changed`
    /// reports changed without consulting its state.
    pub force: bool,
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
