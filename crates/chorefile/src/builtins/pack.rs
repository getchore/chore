//! `extract` and `archive`.
//!
//! Every codec here is pure Rust apart from zstd, so a chorefile never needs
//! `unzip` or `tar` on `PATH` and the binary stays static.
//!
//! Archive entry names are attacker-controlled: a `.tar.gz` from the internet
//! can name `../../.ssh/authorized_keys` and, handed to a naive extractor,
//! write there. Every entry goes through [`safe_entry`] before it becomes a
//! path, and nothing else in this module joins a name onto `dest`.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::exec::{self, Ctx, Output};

/// The builtins defined in this module.
pub fn lookup(name: &str) -> Option<exec::Builtin> {
    match name {
        "extract" => Some(extract),
        "archive" => Some(archive),
        _ => None,
    }
}

fn run_err(message: impl Into<String>) -> Error {
    Error::Run {
        message: message.into(),
    }
}

fn io_err(what: &str, path: &Path, e: impl std::fmt::Display) -> Error {
    run_err(format!("{what} {}: {e}", crate::vars::display(path)))
}

// ---------------------------------------------------------------------------
// formats
// ---------------------------------------------------------------------------

/// A stream compressor. Whether a compressed stream holds a tar or a single
/// file is decided by looking inside, not by the extension, so `.gz` and
/// `.tar.gz` need no separate spelling here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Gz,
    Xz,
    Zst,
}

/// What `extract` is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    Compressed(Codec),
}

/// What `archive` should produce. A tar may be compressed; a zip is already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Zip,
    Tar(Option<Codec>),
}

/// Format from the filename, lowercased so `TOOL.TAR.GZ` works.
pub fn format_from_name(name: &str) -> Option<Format> {
    let n = name.to_ascii_lowercase();
    let ends = |s: &str| n.ends_with(s);
    Some(match () {
        _ if ends(".zip") => Format::Zip,
        _ if ends(".tar") => Format::Tar,
        // `.tgz` and friends collapse into the same case as `.tar.gz`: the
        // container is settled by sniffing the decompressed bytes.
        _ if ends(".gz") || ends(".tgz") => Format::Compressed(Codec::Gz),
        _ if ends(".xz") || ends(".txz") || ends(".lzma") => Format::Compressed(Codec::Xz),
        _ if ends(".zst") || ends(".tzst") => Format::Compressed(Codec::Zst),
        _ => return None,
    })
}

/// Format from the leading bytes, for archives whose name says nothing.
pub fn sniff(head: &[u8]) -> Option<Format> {
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Some(Format::Zip);
    }
    if head.starts_with(&[0x1f, 0x8b]) {
        return Some(Format::Compressed(Codec::Gz));
    }
    if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        return Some(Format::Compressed(Codec::Xz));
    }
    if head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Some(Format::Compressed(Codec::Zst));
    }
    if is_tar(head) {
        return Some(Format::Tar);
    }
    None
}

/// A tar header carries `ustar` at offset 257. There is no other magic, so a
/// short read is simply "not a tar".
pub fn is_tar(head: &[u8]) -> bool {
    head.len() >= 262 && &head[257..262] == b"ustar"
}

/// Output format for `archive`, from the destination's extension.
pub fn target_from_name(name: &str) -> Option<Target> {
    let n = name.to_ascii_lowercase();
    let ends = |s: &str| n.ends_with(s);
    Some(match () {
        _ if ends(".zip") => Target::Zip,
        _ if ends(".tar.gz") || ends(".tgz") => Target::Tar(Some(Codec::Gz)),
        _ if ends(".tar.xz") || ends(".txz") => Target::Tar(Some(Codec::Xz)),
        _ if ends(".tar.zst") || ends(".tzst") => Target::Tar(Some(Codec::Zst)),
        _ if ends(".tar") => Target::Tar(None),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// entry-name safety
// ---------------------------------------------------------------------------

/// Turn an archive entry name into a relative path that cannot escape the
/// destination, or reject it.
///
/// `Ok(None)` means the entry vanished under `--strip` and should be skipped;
/// `Err` means the archive tried to write outside `dest` and the whole
/// extraction must stop, because a malicious archive is not something to
/// half-apply.
///
/// `flatten` reduces a surviving entry to its base name. It is applied here,
/// after the component check, rather than by the callers: the base name of a
/// name that already passed this function is an ordinary component — never
/// empty, `.`, `..`, or a drive prefix — so a flattened entry cannot reach
/// outside `dest` either. Doing it anywhere else would put a second place
/// that builds a path out of an archive name.
pub fn safe_entry(name: &str, strip: usize, flatten: bool) -> Result<Option<PathBuf>> {
    let reject = || run_err(format!("extract: refusing unsafe archive entry {name:?}"));

    // Archives written on Windows use `\`; treat both as separators so a
    // `..\..` entry cannot slip past the component check.
    let normalized = name.replace('\\', "/");

    if normalized.starts_with('/') {
        return Err(reject());
    }
    // `C:foo` and `C:/foo` are both absolute enough to be dangerous.
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(reject());
    }
    if normalized.contains('\0') {
        return Err(reject());
    }

    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(reject()),
            part => parts.push(part),
        }
    }

    if parts.len() <= strip {
        return Ok(None);
    }
    if flatten {
        // `parts` is non-empty here, and every part is an ordinary component.
        return Ok(Some(PathBuf::from(parts[parts.len() - 1])));
    }
    Ok(Some(parts[strip..].iter().collect()))
}

/// A symlink target is as dangerous as an entry name: it is resolved by the
/// OS, so `link -> /etc/passwd` turns a later write into a write outside
/// `dest`. Only plainly-relative targets are allowed.
fn safe_link_target(target: &Path) -> bool {
    !target.is_absolute()
        && !target.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
}

// ---------------------------------------------------------------------------
// extract
// ---------------------------------------------------------------------------

/// A parsed `extract` invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractArgs {
    pub archive: String,
    pub dest: String,
    pub member: Option<String>,
    pub strip: usize,
    /// Write every entry directly into `dest` under its base name.
    ///
    /// Without it, `--member` keeps the path the entry had *inside* the
    /// archive, so pulling one binary out of a release tarball lands it
    /// somewhere you cannot predict without opening the archive first:
    ///
    /// ```text
    /// extract out.tar.gz got --member sona   ->  got/pkg/bin/sona
    /// ```
    ///
    /// which forces every `extract --member` to be followed by a `move` to
    /// put the file where it was actually wanted. With `--flatten` the same
    /// call lands `got/sona` and the `move` disappears.
    pub flatten: bool,
}

/// Parse `extract <archive> <dest> [--member name] [--strip n] [--flatten]`.
pub fn parse_extract(rest: &[String]) -> Result<ExtractArgs> {
    let mut args = ExtractArgs::default();
    let mut positional = Vec::new();
    let mut saw_strip = false;
    let mut it = rest.iter();

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--member" => {
                args.member = Some(
                    it.next()
                        .cloned()
                        .ok_or_else(|| run_err("extract: --member needs a value"))?,
                );
            }
            "--strip" => {
                saw_strip = true;
                let v = it
                    .next()
                    .ok_or_else(|| run_err("extract: --strip needs a value"))?;
                args.strip = v
                    .parse()
                    .map_err(|_| run_err(format!("extract: --strip {v} is not a number")))?;
            }
            "--flatten" => args.flatten = true,
            other if other.starts_with("--") => {
                return Err(run_err(format!("extract: unknown option {other}")));
            }
            other => positional.push(other.to_string()),
        }
    }

    // Both flags drop path components, and combining them reads two ways:
    // strip-then-flatten (where the strip does nothing) and flatten-then-strip
    // (where the file usually vanishes). Neither reading is worth guessing at,
    // and `--flatten` alone is what anyone writing the pair actually wants.
    if saw_strip && args.flatten {
        return Err(run_err(
            "extract: --strip and --flatten cannot be combined; --flatten already drops every \
directory, so drop the --strip",
        ));
    }

    let [archive, dest] = positional.as_slice() else {
        return Err(run_err("extract: expected <archive> <dest>"));
    };
    args.archive = archive.clone();
    args.dest = dest.clone();
    Ok(args)
}

fn extract(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = parse_extract(ctx.rest())?;
    if ctx.dry {
        return Ok(Output::ok());
    }

    let src = ctx.path(&args.archive);
    let dest_is_dir_hint = args.dest.ends_with('/') || args.dest.ends_with('\\');
    let dest = ctx.path(&args.dest);

    let mut head = [0u8; 512];
    let read = {
        let mut f = File::open(&src).map_err(|e| io_err("extract: cannot open", &src, e))?;
        f.read(&mut head)
            .map_err(|e| io_err("extract: cannot read", &src, e))?
    };
    let head = &head[..read];

    let format = format_from_name(&src.to_string_lossy())
        .or_else(|| sniff(head))
        .ok_or_else(|| {
            run_err(format!(
                "extract: cannot tell the format of {} from its name or contents",
                crate::vars::display(&src)
            ))
        })?;

    match format {
        Format::Zip => extract_zip(&src, &dest, &args),
        Format::Tar => {
            let file = File::open(&src).map_err(|e| io_err("extract: cannot open", &src, e))?;
            extract_tar(BufReader::new(file), &src, &dest, &args)
        }
        Format::Compressed(codec) => {
            extract_compressed(codec, &src, &dest, dest_is_dir_hint, &args)
        }
    }?;

    Ok(Output::ok())
}

/// A compressed stream is decompressed to a temp file first, then sniffed.
///
/// The extra write buys the "bare `.gz`" case for free: the same code path
/// serves `tool.tar.gz` and `notes.txt.gz`, and an archive whose extension
/// lies about its container still extracts correctly.
fn extract_compressed(
    codec: Codec,
    src: &Path,
    dest: &Path,
    dest_is_dir_hint: bool,
    args: &ExtractArgs,
) -> Result<()> {
    let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|e| io_err("extract: cannot create", parent, e))?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent.unwrap_or(Path::new(".")))
        .map_err(|e| io_err("extract: cannot create a temp file beside", dest, e))?;

    let input = File::open(src).map_err(|e| io_err("extract: cannot open", src, e))?;
    decompress(codec, BufReader::new(input), BufWriter::new(&mut temp), src)?;

    let mut file = temp
        .reopen()
        .map_err(|e| io_err("extract: cannot reread", src, e))?;
    let mut head = [0u8; 512];
    let read = file
        .read(&mut head)
        .map_err(|e| io_err("extract: cannot read", src, e))?;
    file.rewind()
        .map_err(|e| io_err("extract: cannot read", src, e))?;

    if is_tar(&head[..read]) {
        return extract_tar(BufReader::new(file), src, dest, args);
    }

    // One plain file. `dest` is the output path unless it looks like a
    // directory, in which case the compressed name minus its extension is.
    if args.strip != 0 || args.member.is_some() || args.flatten {
        return Err(run_err(format!(
            "extract: {} holds a single file, so --member, --strip and --flatten do not apply",
            crate::vars::display(src)
        )));
    }
    let out = if dest_is_dir_hint || dest.is_dir() {
        let stem = src.file_stem().ok_or_else(|| {
            run_err(format!(
                "extract: {} has no name",
                crate::vars::display(src)
            ))
        })?;
        dest.join(stem)
    } else {
        dest.to_path_buf()
    };
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| io_err("extract: cannot create", parent, e))?;
    }
    temp.persist(&out)
        .map_err(|e| io_err("extract: cannot write", &out, e.error))?;
    Ok(())
}

fn decompress(
    codec: Codec,
    mut input: impl io::BufRead,
    mut output: impl Write,
    src: &Path,
) -> Result<()> {
    let result = match codec {
        Codec::Gz => io::copy(&mut flate2::bufread::GzDecoder::new(input), &mut output).map(|_| ()),
        Codec::Xz => lzma_rs::xz_decompress(&mut input, &mut output)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        Codec::Zst => zstd::stream::copy_decode(input, &mut output),
    };
    result
        .and_then(|()| output.flush())
        .map_err(|e| io_err("extract: cannot decompress", src, e))
}

/// The base names already written under `--flatten`, so a second entry that
/// wants one is refused instead of overwriting the first.
///
/// Overwriting is the one outcome nobody can debug: the run succeeds and the
/// file that ends up in `dest` depends on the order entries happen to sit in
/// the archive. Refusing names both entries, and the fix — a narrower
/// `--member`, or two calls — is then obvious. Without `--flatten` this never
/// records anything, since distinct archive entries already have distinct
/// paths.
#[derive(Default)]
struct Claimed(std::collections::HashMap<PathBuf, String>);

impl Claimed {
    fn claim(&mut self, rel: &Path, name: &str, args: &ExtractArgs) -> Result<()> {
        if !args.flatten {
            return Ok(());
        }
        match self.0.entry(rel.to_path_buf()) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(name.to_string());
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(slot) => Err(run_err(format!(
                "extract: --flatten would write both {:?} and {name:?} to {:?}; \
name one of them with --member, or extract them separately",
                slot.get(),
                rel.to_string_lossy(),
            ))),
        }
    }
}

fn extract_zip(src: &Path, dest: &Path, args: &ExtractArgs) -> Result<()> {
    let file = File::open(src).map_err(|e| io_err("extract: cannot open", src, e))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file))
        .map_err(|e| io_err("extract: cannot read zip", src, e))?;

    let mut found = false;
    let mut claimed = Claimed::default();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| io_err("extract: cannot read zip", src, e))?;
        let name = entry.name().to_string();
        if !wanted(&name, args) {
            continue;
        }
        let Some(rel) = safe_entry(&name, args.strip, args.flatten)? else {
            continue;
        };

        if entry.is_dir() {
            // A flattened extraction has no directories to put anything in,
            // so a directory entry has nothing left to name.
            if args.flatten {
                continue;
            }
            found = true;
            let out = dest.join(rel);
            fs::create_dir_all(&out).map_err(|e| io_err("extract: cannot create", &out, e))?;
            continue;
        }
        found = true;
        claimed.claim(&rel, &name, args)?;
        let out = dest.join(rel);
        let mode = entry.unix_mode();
        write_file(&out, &mut entry, mode)?;
    }
    finish(found, src, args)
}

fn extract_tar<R: Read>(reader: R, src: &Path, dest: &Path, args: &ExtractArgs) -> Result<()> {
    let mut tar = tar::Archive::new(reader);
    let mut found = false;
    let mut claimed = Claimed::default();

    for entry in tar
        .entries()
        .map_err(|e| io_err("extract: cannot read tar", src, e))?
    {
        let mut entry = entry.map_err(|e| io_err("extract: cannot read tar", src, e))?;
        let name = entry
            .path()
            .map_err(|e| io_err("extract: cannot read tar", src, e))?
            .to_string_lossy()
            .into_owned();
        if !wanted(&name, args) {
            continue;
        }
        let Some(rel) = safe_entry(&name, args.strip, args.flatten)? else {
            continue;
        };
        let kind = entry.header().entry_type();

        if kind.is_dir() {
            // See the zip loop: a flattened tree has no directories in it.
            if args.flatten {
                continue;
            }
            found = true;
            let out = dest.join(rel);
            fs::create_dir_all(&out).map_err(|e| io_err("extract: cannot create", &out, e))?;
            continue;
        }
        if !kind.is_file() && !kind.is_symlink() && !kind.is_hard_link() {
            // Devices, fifos and sockets have no portable meaning in a
            // project tree, so they are dropped rather than half-created.
            continue;
        }
        found = true;
        claimed.claim(&rel, &name, args)?;
        let out = dest.join(rel);
        if kind.is_symlink() || kind.is_hard_link() {
            link(&entry, &out, &name)?;
            continue;
        }
        let mode = entry.header().mode().ok();
        write_file(&out, &mut entry, mode)?;
    }
    finish(found, src, args)
}

/// Does this entry pass `--member`? A member matches either its full path in
/// the archive or just its filename, so `--member chore` finds `bin/chore`.
fn wanted(name: &str, args: &ExtractArgs) -> bool {
    let Some(member) = &args.member else {
        return true;
    };
    let normalized = name.replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    normalized == member.trim_end_matches('/')
        || normalized.rsplit('/').next() == Some(member.as_str())
}

fn finish(found: bool, src: &Path, args: &ExtractArgs) -> Result<()> {
    match &args.member {
        Some(member) if !found => Err(run_err(format!(
            "extract: {} has no entry {member}",
            crate::vars::display(src)
        ))),
        _ => Ok(()),
    }
}

fn write_file(out: &Path, from: &mut dyn Read, mode: Option<u32>) -> Result<()> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| io_err("extract: cannot create", parent, e))?;
    }
    let mut file = File::create(out).map_err(|e| io_err("extract: cannot write", out, e))?;
    io::copy(from, &mut file).map_err(|e| io_err("extract: cannot write", out, e))?;
    drop(file);
    set_mode(out, mode)
}

/// Preserve the executable bit, which is the only mode a downloaded toolchain
/// actually depends on. Windows has no such bit, so this is a no-op there
/// rather than an error.
#[cfg(unix)]
fn set_mode(out: &Path, mode: Option<u32>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = mode.filter(|m| m & 0o111 != 0) else {
        return Ok(());
    };
    // Take only the permission bits, and never grant more than 0o755: an
    // archive asking for setuid is not something to honour.
    let perms = fs::Permissions::from_mode(mode & 0o755);
    fs::set_permissions(out, perms).map_err(|e| io_err("extract: cannot chmod", out, e))
}

#[cfg(not(unix))]
fn set_mode(_out: &Path, _mode: Option<u32>) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn link<R: Read>(entry: &tar::Entry<'_, R>, out: &Path, name: &str) -> Result<()> {
    let target = entry
        .link_name()
        .ok()
        .flatten()
        .ok_or_else(|| run_err(format!("extract: {name} is a link with no target")))?;
    if !safe_link_target(&target) {
        return Err(run_err(format!(
            "extract: refusing link {name:?} -> {:?}, which points outside the destination",
            target.display()
        )));
    }
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| io_err("extract: cannot create", parent, e))?;
    }
    let _ = fs::remove_file(out);
    std::os::unix::fs::symlink(&target, out).map_err(|e| io_err("extract: cannot link", out, e))
}

/// Windows needs a privilege to create symlinks, so a link entry is skipped
/// rather than failing an otherwise good extraction.
#[cfg(not(unix))]
fn link<R: Read>(_entry: &tar::Entry<'_, R>, _out: &Path, _name: &str) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// archive
// ---------------------------------------------------------------------------

/// One thing to pack: where it lives on disk, and the name it takes inside
/// the archive.
///
/// Both writers work from the same list, so a tar and a zip built from the
/// same arguments hold the same entries in the same order.
#[derive(Debug)]
struct Entry {
    path: PathBuf,
    name: String,
    dir: bool,
}

fn archive(ctx: &mut Ctx<'_>) -> Result<Output> {
    let rest = ctx.rest();
    // The destination is the last argument, so everything before it is a
    // source. One source is the old `archive <src> <dest>` spelling.
    let Some((dest_arg, src_args)) = rest.split_last().filter(|(_, srcs)| !srcs.is_empty()) else {
        return Err(run_err("archive: expected <src...> <dest>"));
    };
    if ctx.dry {
        return Ok(Output::ok());
    }

    let dest = ctx.path(dest_arg);
    let target = target_from_name(&dest.to_string_lossy()).ok_or_else(|| {
        run_err(format!(
            "archive: {} does not end in .zip, .tar, .tar.gz, .tar.xz or .tar.zst{}",
            crate::vars::display(&dest),
            // With several sources, mistaking the order is the likely cause,
            // so say which argument was read as the destination.
            if src_args.len() > 1 {
                "; the last argument is the destination"
            } else {
                ""
            }
        ))
    })?;

    let mut entries = Vec::new();
    for src_arg in src_args {
        collect(ctx, src_arg, &mut entries)?;
    }
    // Two sources may not claim the same name: a zip would hold a duplicate
    // entry and a tar would silently shadow the first on extraction.
    let mut seen = std::collections::HashSet::new();
    for entry in &entries {
        if !seen.insert(entry.name.as_str()) {
            return Err(run_err(format!(
                "archive: two sources both add {:?}; give them different names or pack them separately",
                entry.name
            )));
        }
    }

    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| io_err("archive: cannot create", parent, e))?;
    }

    match target {
        Target::Zip => write_zip(&entries, &dest)?,
        Target::Tar(None) => {
            let file =
                File::create(&dest).map_err(|e| io_err("archive: cannot write", &dest, e))?;
            write_tar(&entries, BufWriter::new(file), &dest)?;
        }
        Target::Tar(Some(codec)) => {
            // xz has no streaming writer in lzma-rs, so the tar is staged in a
            // temp file and compressed in one pass. Doing it the same way for
            // every codec keeps one path instead of three.
            let parent = dest.parent().unwrap_or(Path::new("."));
            let temp = tempfile::NamedTempFile::new_in(parent)
                .map_err(|e| io_err("archive: cannot create a temp file beside", &dest, e))?;
            write_tar(
                &entries,
                BufWriter::new(
                    temp.reopen()
                        .map_err(|e| io_err("archive: cannot write", &dest, e))?,
                ),
                &dest,
            )?;

            let input =
                File::open(temp.path()).map_err(|e| io_err("archive: cannot reread", &dest, e))?;
            let out = File::create(&dest).map_err(|e| io_err("archive: cannot write", &dest, e))?;
            compress(codec, BufReader::new(input), BufWriter::new(out), &dest)?;
        }
    }
    Ok(Output::ok())
}

/// Expand one `src` argument into the entries it contributes.
///
/// Normally the source's own name is the top-level entry, matching `tar cf`
/// and `zip -r`: extracting somewhere gives you the directory back, not its
/// contents loose in the destination. A trailing `/` asks for the contents
/// instead, the same way `extract` reads a trailing slash on its `dest`.
fn collect(ctx: &Ctx<'_>, arg: &str, out: &mut Vec<Entry>) -> Result<()> {
    let contents_only = arg.ends_with('/') || arg.ends_with('\\');
    // The slash is the request, not part of the name: some platforms refuse
    // to stat `notes.txt/` at all, and the error for that must say what is
    // really wrong rather than "does not exist".
    let trimmed = arg.trim_end_matches(['/', '\\']);
    let src = ctx.path(if trimmed.is_empty() { arg } else { trimmed });
    if !src.exists() {
        return Err(run_err(format!(
            "archive: {} does not exist",
            crate::vars::display(&src)
        )));
    }

    if contents_only {
        if !src.is_dir() {
            return Err(run_err(format!(
                "archive: {} ends in a slash, which asks for its contents, but it is not a \
directory",
                crate::vars::display(&src)
            )));
        }
        return walk(&src, String::new(), out);
    }

    let base = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            run_err(format!(
                "archive: {} has no name",
                crate::vars::display(&src)
            ))
        })?;
    let dir = src.is_dir();
    out.push(Entry {
        path: src.clone(),
        name: base.clone(),
        dir,
    });
    if dir {
        walk(&src, base, out)?;
    }
    Ok(())
}

/// Every path under `root`, named `prefix`-relative with `/` separators.
/// Sorted so an archive built twice from the same tree is byte-identical.
fn walk(root: &Path, prefix: String, out: &mut Vec<Entry>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(root)
        .map_err(|e| io_err("archive: cannot read", root, e))?
        .collect::<io::Result<Vec<_>>>()
        .map_err(|e| io_err("archive: cannot read", root, e))?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        // A symlinked directory is recorded but not walked: following one
        // would duplicate the tree, and a cycle would never terminate. The
        // tar writer keeps the link itself, since it does not follow symlinks.
        let meta =
            fs::symlink_metadata(&path).map_err(|e| io_err("archive: cannot read", &path, e))?;
        let link = meta.file_type().is_symlink();
        let dir = if link { path.is_dir() } else { meta.is_dir() };
        out.push(Entry {
            path: path.clone(),
            name: rel.clone(),
            dir,
        });
        if dir && !link {
            walk(&path, rel, out)?;
        }
    }
    Ok(())
}

fn write_tar<W: Write>(entries: &[Entry], writer: W, dest: &Path) -> Result<()> {
    let mut builder = tar::Builder::new(writer);
    builder.follow_symlinks(false);
    for entry in entries {
        builder
            .append_path_with_name(&entry.path, &entry.name)
            .map_err(|e| io_err("archive: cannot write", dest, e))?;
    }
    builder
        .into_inner()
        .and_then(|mut w| w.flush())
        .map_err(|e| io_err("archive: cannot write", dest, e))
}

fn write_zip(entries: &[Entry], dest: &Path) -> Result<()> {
    let file = File::create(dest).map_err(|e| io_err("archive: cannot write", dest, e))?;
    let mut zip = zip::ZipWriter::new(BufWriter::new(file));

    for entry in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let options = mode_of(&entry.path).map_or(options, |m| options.unix_permissions(m));
        if entry.dir {
            zip.add_directory(format!("{}/", entry.name), options)
                .map_err(|e| io_err("archive: cannot write", dest, e))?;
        } else {
            zip.start_file(entry.name.clone(), options)
                .map_err(|e| io_err("archive: cannot write", dest, e))?;
            let mut f = File::open(&entry.path)
                .map_err(|e| io_err("archive: cannot read", &entry.path, e))?;
            io::copy(&mut f, &mut zip).map_err(|e| io_err("archive: cannot write", dest, e))?;
        }
    }

    zip.finish()
        .map_err(|e| io_err("archive: cannot write", dest, e))?;
    Ok(())
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(path.metadata().ok()?.permissions().mode())
}

/// Windows has no unix mode to record, and inventing one would make archives
/// built there extract with the wrong bits elsewhere.
#[cfg(not(unix))]
fn mode_of(_path: &Path) -> Option<u32> {
    None
}

fn compress(
    codec: Codec,
    mut input: impl io::BufRead,
    mut output: impl Write,
    dest: &Path,
) -> Result<()> {
    let result = match codec {
        Codec::Gz => {
            let mut enc =
                flate2::write::GzEncoder::new(&mut output, flate2::Compression::default());
            io::copy(&mut input, &mut enc)
                .and_then(|_| enc.finish())
                .map(|_| ())
        }
        Codec::Xz => lzma_rs::xz_compress(&mut input, &mut output),
        Codec::Zst => zstd::stream::copy_encode(&mut input, &mut output, 3),
    };
    result
        .and_then(|()| output.flush())
        .map_err(|e| io_err("archive: cannot compress", dest, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_come_from_the_extension() {
        assert_eq!(format_from_name("a.zip"), Some(Format::Zip));
        assert_eq!(format_from_name("a.TAR"), Some(Format::Tar));
        assert_eq!(
            format_from_name("a.tar.gz"),
            Some(Format::Compressed(Codec::Gz))
        );
        assert_eq!(
            format_from_name("a.tgz"),
            Some(Format::Compressed(Codec::Gz))
        );
        assert_eq!(
            format_from_name("a.txt.xz"),
            Some(Format::Compressed(Codec::Xz))
        );
        assert_eq!(
            format_from_name("a.tar.zst"),
            Some(Format::Compressed(Codec::Zst))
        );
        assert_eq!(format_from_name("a.bin"), None);
    }

    #[test]
    fn magic_bytes_are_the_fallback() {
        assert_eq!(sniff(b"PK\x03\x04rest"), Some(Format::Zip));
        assert_eq!(
            sniff(&[0x1f, 0x8b, 0x08]),
            Some(Format::Compressed(Codec::Gz))
        );
        assert_eq!(
            sniff(&[0x28, 0xb5, 0x2f, 0xfd]),
            Some(Format::Compressed(Codec::Zst))
        );
        assert_eq!(sniff(b"nothing"), None);
    }

    #[test]
    fn archive_targets_prefer_the_longer_extension() {
        assert_eq!(
            target_from_name("a.tar.gz"),
            Some(Target::Tar(Some(Codec::Gz)))
        );
        assert_eq!(target_from_name("a.tar"), Some(Target::Tar(None)));
        assert_eq!(target_from_name("a.zip"), Some(Target::Zip));
        assert_eq!(target_from_name("a.gz"), None);
    }
}
