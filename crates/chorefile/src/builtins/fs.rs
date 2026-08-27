//! Filesystem and utility builtins: everything a chorefile would otherwise
//! shell out to `cp`, `rm`, `find`, `cat` or `shasum` for.
//!
//! Two conventions run through the whole module.
//!
//! *Paths* are written with `/` in a chorefile on every platform. Arguments
//! reach the host filesystem through [`Ctx::path`], and anything printed goes
//! back out through [`vars::display`], so a chorefile never sees a `\`.
//!
//! *Failure* comes in two flavours. A command that could not do its job
//! returns [`Error::Run`] and stops the task. A command that answers a
//! question — `which`, `exists`, `env` reading an unset name — answers "no"
//! with a nonzero exit code instead, because those are the forms `if` and
//! `try` are built to consume; an error would make `if which cargo` unusable.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::exec::{Builtin, Ctx, Output};
use crate::vars;

/// The builtins implemented in this module, for the interpreter's dispatch.
pub fn lookup(name: &str) -> Option<Builtin> {
    Some(match name {
        "copy" => copy,
        "move" => move_,
        "remove" => remove,
        "mkdir" => mkdir,
        "chmod" => chmod,
        "which" => which,
        "find" => find,
        "read" => read,
        "write" => write,
        "sha256" => sha256,
        "exists" => exists,
        "echo" => echo,
        "env" => env,
        "fail" => fail,
        "sleep" => sleep,
        _ => return None,
    })
}

fn run(message: impl Into<String>) -> Error {
    Error::Run {
        message: message.into(),
    }
}

/// `<cmd>: <what> <path>: <os message>`, so a failure names the file it was
/// working on rather than just "permission denied".
fn at(cmd: &str, what: &str, path: &Path, e: &std::io::Error) -> Error {
    run(format!("{cmd}: {what} {}: {e}", vars::display(path)))
}

fn usage(text: &str) -> Error {
    run(format!("usage: {text}"))
}

fn line(ctx: &mut Ctx<'_>, text: &str) -> Result<()> {
    writeln!(ctx.out, "{text}").map_err(Error::Io)
}

/// One diagnostic line. It goes to `ctx.err` so that it reaches the terminal
/// even when stdout is captured, and lands in the file when `2>` asks for it.
fn diag(ctx: &mut Ctx<'_>, text: &str) -> Result<()> {
    writeln!(ctx.err, "{text}").map_err(Error::Io)
}

// --- copy / move -----------------------------------------------------------

/// `copy <src> <dest>` — a file or a whole directory tree.
///
/// When `dest` is an existing directory the source is placed inside it under
/// its own name, as `cp` and `mv` do; otherwise `dest` is the exact path the
/// copy is written to.
fn copy(ctx: &mut Ctx<'_>) -> Result<Output> {
    let [src, dest] = two(ctx, "copy <src> <dest>")?;
    if ctx.dry {
        return Ok(Output::ok());
    }
    let (src, dest) = (ctx.path(&src), resolve_dest(ctx, &src, &dest));
    copy_any(&src, &dest)?;
    Ok(Output::ok())
}

/// `move <src> <dest>` — rename where the filesystem allows it, copy and
/// delete when the two paths are on different volumes.
fn move_(ctx: &mut Ctx<'_>) -> Result<Output> {
    let [src, dest] = two(ctx, "move <src> <dest>")?;
    if ctx.dry {
        return Ok(Output::ok());
    }
    let (src, dest) = (ctx.path(&src), resolve_dest(ctx, &src, &dest));
    if !exists_on_disk(&src) {
        return Err(run(format!(
            "move: no such file or directory: {}",
            vars::display(&src)
        )));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| at("move", "cannot create", parent, &e))?;
    }
    if fs::rename(&src, &dest).is_ok() {
        return Ok(Output::ok());
    }
    copy_any(&src, &dest)?;
    delete(&src).map_err(|e| at("move", "cannot remove", &src, &e))?;
    Ok(Output::ok())
}

/// Placing a source *into* `dest` when `dest` is an existing directory.
fn resolve_dest(ctx: &Ctx<'_>, src: &str, dest: &str) -> PathBuf {
    let dest = ctx.path(dest);
    if dest.is_dir() {
        if let Some(name) = ctx.path(src).file_name() {
            return dest.join(name);
        }
    }
    dest
}

fn copy_any(src: &Path, dest: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(src).map_err(|e| at("copy", "cannot read", src, &e))?;
    if meta.is_dir() {
        copy_dir(src, dest)
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| at("copy", "cannot create", parent, &e))?;
        }
        fs::copy(src, dest).map_err(|e| at("copy", "cannot copy", src, &e))?;
        Ok(())
    }
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).map_err(|e| at("copy", "cannot create", dest, &e))?;
    for entry in fs::read_dir(src).map_err(|e| at("copy", "cannot read", src, &e))? {
        let entry = entry.map_err(|e| at("copy", "cannot read", src, &e))?;
        copy_any(&entry.path(), &dest.join(entry.file_name()))?;
    }
    Ok(())
}

// --- remove / mkdir --------------------------------------------------------

/// `remove <path...>` — recursive, and silent about paths that are already
/// gone, so a cleanup task is safe to run twice.
fn remove(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest().to_vec();
    if args.is_empty() {
        return Err(usage("remove <path...>"));
    }
    if ctx.dry {
        return Ok(Output::ok());
    }
    for arg in &args {
        let path = ctx.path(arg);
        guard_remove(ctx, &path)?;
        if !exists_on_disk(&path) {
            continue;
        }
        delete(&path).map_err(|e| at("remove", "cannot remove", &path, &e))?;
    }
    Ok(Output::ok())
}

/// A typo in a chorefile must not be able to empty a machine. `remove` refuses
/// the filesystem root and `$ROOT` itself; everything under `$ROOT` is fair
/// game.
fn guard_remove(ctx: &Ctx<'_>, path: &Path) -> Result<()> {
    let target = real(path);
    if is_fs_root(&target) {
        return Err(run(format!(
            "remove: refusing to remove the filesystem root: {}",
            vars::display(&target)
        )));
    }
    if target == real(ctx.root) {
        return Err(run(format!(
            "remove: refusing to remove $ROOT: {}",
            vars::display(&target)
        )));
    }
    Ok(())
}

/// A path with nothing but a root component: `/`, `C:\`, `\\server\share\`.
fn is_fs_root(path: &Path) -> bool {
    let mut parts = path.components();
    matches!(
        parts.next(),
        Some(Component::RootDir | Component::Prefix(_))
    ) && parts.all(|c| matches!(c, Component::RootDir | Component::Prefix(_)))
}

/// Canonicalize where possible; a path that does not exist cannot be the root
/// or `$ROOT`, so falling back to the literal path is safe.
fn real(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn delete(path: &Path) -> std::io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// `mkdir <path...>` — always `-p`: parents are created and an existing
/// directory is not an error.
fn mkdir(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest().to_vec();
    if args.is_empty() {
        return Err(usage("mkdir <path...>"));
    }
    if ctx.dry {
        return Ok(Output::ok());
    }
    for arg in &args {
        let path = ctx.path(arg);
        fs::create_dir_all(&path).map_err(|e| at("mkdir", "cannot create", &path, &e))?;
    }
    Ok(Output::ok())
}

// --- chmod -----------------------------------------------------------------

/// `chmod <mode> <path>` — an octal mode, with or without a leading `0`.
///
/// Windows has no permission bits to set. There, the mode is reduced to its
/// owner-write bit: clearing it marks the file read-only, setting it clears
/// the read-only flag. Every other bit — the execute bit in particular — is
/// ignored, because a Windows file is executable by extension, not by mode.
fn chmod(ctx: &mut Ctx<'_>) -> Result<Output> {
    let [mode, target] = two(ctx, "chmod <mode> <path>")?;
    let bits = u32::from_str_radix(mode.trim_start_matches("0o"), 8)
        .map_err(|_| run(format!("chmod: not an octal mode: {mode}")))?;
    if ctx.dry {
        return Ok(Output::ok());
    }
    let path = ctx.path(&target);
    let meta = fs::metadata(&path).map_err(|e| at("chmod", "cannot read", &path, &e))?;
    let mut perms = meta.permissions();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(bits);
    }
    #[cfg(not(unix))]
    {
        perms.set_readonly(bits & 0o200 == 0);
    }

    fs::set_permissions(&path, perms).map_err(|e| at("chmod", "cannot chmod", &path, &e))?;
    Ok(Output::ok())
}

// --- which -----------------------------------------------------------------

/// `which <name>` — prints the resolved path, or exits nonzero if the program
/// is not on `PATH`. Read-only, so it runs under `--dry` too: `if which cargo`
/// has to answer truthfully for the preview to mean anything.
///
/// On Windows a bare name is tried against every suffix in `PATHEXT` (falling
/// back to the usual `.COM;.EXE;.BAT;.CMD` when it is unset), and the name as
/// written is tried first so `which cargo.exe` resolves.
fn which(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest();
    let [name] = args else {
        return Err(usage("which <name>"));
    };
    let name = name.clone();

    // A name with a separator is a path, not a `PATH` lookup.
    if name.contains('/') || name.contains('\\') {
        let base = ctx.path(&name);
        return match candidates(&base).into_iter().find(|p| is_executable(p)) {
            Some(found) => {
                line(ctx, &vars::display(&found))?;
                Ok(Output::ok())
            }
            None => Ok(Output::failed(1)),
        };
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path).filter(|d| !d.as_os_str().is_empty()) {
        let base = dir.join(&name);
        if let Some(found) = candidates(&base).into_iter().find(|p| is_executable(p)) {
            line(ctx, &vars::display(&found))?;
            return Ok(Output::ok());
        }
    }
    Ok(Output::failed(1))
}

/// The name as written, plus the `PATHEXT` variants on Windows.
fn candidates(base: &Path) -> Vec<PathBuf> {
    let mut out = vec![base.to_path_buf()];
    if cfg!(windows) {
        let ext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let name = base.file_name().unwrap_or_default().to_string_lossy();
        for suffix in ext.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            out.push(base.with_file_name(format!("{name}{suffix}")));
        }
    }
    out
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Windows has no execute bit: a file is runnable if its extension says so,
/// which [`candidates`] has already decided.
#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

// --- find ------------------------------------------------------------------

/// `find <root> <name...>` — every entry under `root` whose name matches one
/// of the patterns, one per line, depth-first and sorted so two runs on the
/// same tree print the same thing.
///
/// Patterns match the file name only, and support `*` (any run of characters)
/// and `?` (exactly one). Matching is case-sensitive on every platform, so a
/// chorefile cannot depend on a case-insensitive filesystem.
fn find(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest().to_vec();
    if args.len() < 2 {
        return Err(usage("find <root> <name...>"));
    }
    let (root_arg, patterns) = (args[0].clone(), &args[1..]);
    let root = ctx.path(&root_arg);
    if !root.is_dir() {
        return Err(run(format!(
            "find: not a directory: {}",
            vars::display(&root)
        )));
    }

    // Results are reported relative to the root as the chorefile wrote it, so
    // `find src *.rs` yields paths that can be fed straight back to a command.
    let prefix = root_arg.trim_end_matches('/');
    let prefix = if prefix.is_empty() || prefix == "." {
        String::new()
    } else {
        format!("{prefix}/")
    };

    let mut found = Vec::new();
    walk(&root, &mut Vec::new(), patterns, &mut found)?;
    for rel in found {
        line(ctx, &format!("{prefix}{rel}"))?;
    }
    Ok(Output::ok())
}

fn walk(
    dir: &Path,
    rel: &mut Vec<String>,
    patterns: &[String],
    out: &mut Vec<String>,
) -> Result<()> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| at("find", "cannot read", dir, &e))? {
        let entry = entry.map_err(|e| at("find", "cannot read", dir, &e))?;
        entries.push((entry.file_name(), entry.path()));
    }
    entries.sort();

    for (name, path) in entries {
        let name = name.to_string_lossy().into_owned();
        rel.push(name.clone());
        if patterns.iter().any(|p| glob(p, &name)) {
            out.push(rel.join("/"));
        }
        // `symlink_metadata` keeps a symlinked directory from being walked,
        // which is what stops a cycle from hanging the run.
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            walk(&path, rel, patterns, out)?;
        }
        rel.pop();
    }
    Ok(())
}

/// Glob matching for `*` and `?`, iterative with backtracking so a pattern
/// full of stars cannot blow the stack.
fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut resume) = (None, 0);

    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            resume = ni;
            pi += 1;
        } else if let Some(s) = star {
            resume += 1;
            ni = resume;
            pi = s + 1;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

// --- read / write / sha256 / exists ----------------------------------------

/// `read <file>` — contents with surrounding whitespace removed, as `$(...)`
/// would trim them anyway. Read-only, so it runs under `--dry`.
fn read(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest();
    let [file] = args else {
        return Err(usage("read <file>"));
    };
    let path = ctx.path(file);
    let bytes = fs::read(&path).map_err(|e| at("read", "cannot read", &path, &e))?;
    let text = String::from_utf8_lossy(&bytes);
    line(ctx, text.trim())?;
    Ok(Output::ok())
}

/// `write <file> <text>` — overwrites, creating parent directories. Appending
/// is the interpreter's `>>`, so there is no flag for it here. A trailing
/// newline is added, which `read` trims back off.
fn write(ctx: &mut Ctx<'_>) -> Result<Output> {
    let [file, text] = two(ctx, "write <file> <text>")?;
    if ctx.dry {
        return Ok(Output::ok());
    }
    let path = ctx.path(&file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| at("write", "cannot create", parent, &e))?;
    }
    fs::write(&path, format!("{text}\n")).map_err(|e| at("write", "cannot write", &path, &e))?;
    Ok(Output::ok())
}

/// `sha256 <file>` — lowercase hex digest. Read-only, so `--dry` still runs
/// it; a `$(sha256 ...)` that returned nothing would poison the preview.
fn sha256(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest();
    let [file] = args else {
        return Err(usage("sha256 <file>"));
    };
    let path = ctx.path(file);
    let mut f = fs::File::open(&path).map_err(|e| at("sha256", "cannot read", &path, &e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| at("sha256", "cannot read", &path, &e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hex(&hasher.finish());
    line(ctx, &digest)?;
    Ok(Output::ok())
}

/// `exists <path>` — exit 0 when the path is there, 1 when it is not. Prints
/// nothing: it exists to be the condition of an `if`.
fn exists(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest();
    let [path] = args else {
        return Err(usage("exists <path>"));
    };
    let path = ctx.path(path);
    Ok(if exists_on_disk(&path) {
        Output::ok()
    } else {
        Output::failed(1)
    })
}

/// A broken symlink still *exists*, so this asks for link metadata rather
/// than following the link.
fn exists_on_disk(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

// --- echo / env / fail / sleep ---------------------------------------------

/// `echo <text...>` — arguments joined with single spaces, one newline.
fn echo(ctx: &mut Ctx<'_>) -> Result<Output> {
    let text = ctx.rest().join(" ");
    line(ctx, &text)?;
    Ok(Output::ok())
}

/// `env <NAME>` prints a variable, `env <NAME> <value>` sets one for the rest
/// of the run and for the processes it spawns.
///
/// Reading an unset name exits 1 rather than failing the task, so
/// `if env CI` and `try env CI` both behave. Reading runs under `--dry`;
/// setting does not.
fn env(ctx: &mut Ctx<'_>) -> Result<Output> {
    match ctx.rest() {
        [name] => match std::env::var(name) {
            Ok(value) => {
                line(ctx, &value)?;
                Ok(Output::ok())
            }
            Err(_) => {
                diag(ctx, &format!("env: {name} is not set"))?;
                Ok(Output::failed(1))
            }
        },
        [name, value] => {
            if !ctx.dry {
                let (name, value) = (name.clone(), value.clone());
                // SAFETY: `chore` is single-threaded while a task runs — the
                // interpreter waits for each command before starting the next
                // — so no other thread can be reading the environment here.
                unsafe { std::env::set_var(name, value) };
            }
            Ok(Output::ok())
        }
        _ => Err(usage("env <NAME> [value]")),
    }
}

/// `fail <msg>` — stops the task with `msg`. Under `try` it is just a nonzero
/// command like any other.
fn fail(ctx: &mut Ctx<'_>) -> Result<Output> {
    let msg = ctx.rest().join(" ");
    Err(run(if msg.is_empty() {
        "fail".to_string()
    } else {
        msg
    }))
}

/// `sleep <seconds>` — fractional seconds allowed. Skipped under `--dry`,
/// which is meant to be instant.
fn sleep(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest();
    let [secs] = args else {
        return Err(usage("sleep <seconds>"));
    };
    let secs: f64 = secs
        .parse()
        .ok()
        .filter(|s: &f64| s.is_finite() && *s >= 0.0)
        .ok_or_else(|| run(format!("sleep: not a number of seconds: {secs}")))?;
    if !ctx.dry {
        std::thread::sleep(Duration::from_secs_f64(secs));
    }
    Ok(Output::ok())
}

/// Exactly two arguments, or the usage line for the command.
fn two(ctx: &Ctx<'_>, spec: &str) -> Result<[String; 2]> {
    match ctx.rest() {
        [a, b] => Ok([a.clone(), b.clone()]),
        _ => Err(usage(spec)),
    }
}

// --- SHA-256 ---------------------------------------------------------------
//
// FIPS 180-4, section 6.2. Implemented here because `chore` ships as one
// static binary with no dependencies.

/// Round constants: the first 32 bits of the fractional parts of the cube
/// roots of the first 64 primes.
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Streaming SHA-256 state: the eight working words, a 64-byte block buffer,
/// and the message length the padding needs.
pub struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buffered: usize,
    len: u64,
}

impl Sha256 {
    /// Initial hash value: the fractional parts of the square roots of the
    /// first eight primes.
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            buffered: 0,
            len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.buffered > 0 {
            let take = data.len().min(64 - self.buffered);
            self.buf[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            // Still short of a block: the rest of `data` is nothing, and the
            // tail bookkeeping below must not clear what we just buffered.
            if self.buffered < 64 {
                return;
            }
            let block = self.buf;
            self.compress(&block);
            self.buffered = 0;
        }
        let mut chunks = data.chunks_exact(64);
        for chunk in &mut chunks {
            let mut block = [0u8; 64];
            block.copy_from_slice(chunk);
            self.compress(&block);
        }
        let tail = chunks.remainder();
        self.buf[..tail.len()].copy_from_slice(tail);
        self.buffered = tail.len();
    }

    /// Append `0x80`, zeroes, and the 64-bit big-endian bit length.
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0]);
        }
        self.buf[56..].copy_from_slice(&bits.to_be_bytes());
        let block = self.buf;
        self.compress(&block);

        let mut out = [0u8; 32];
        for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        for (slot, word) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(word);
        }
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

/// Lowercase hex, the form every checksum file and `--sha256` flag uses.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
