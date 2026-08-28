//! `changed`, the up-to-date check, and the state file it remembers with.
//!
//! A task that rebuilds something usually wants to skip the work when its
//! inputs are the same as last time. Doing that in the language would need a
//! grammar for inputs and outputs; doing it as a condition builtin needs
//! nothing, because `if changed src Cargo.toml { ... }` is already a
//! condition, and the interpreter already knows how to read one.
//!
//! Two conventions from [`fs`](super::fs) carry over. Paths reach the host
//! through [`Ctx::path`], and a *miss* is an answer rather than a failure:
//! `changed` never returns [`Error::Run`] except for a usage error, because a
//! builtin that fails inside an `if` leaves the condition undecided under
//! `--dry` and stops the task under fail-fast. Even an unreadable file or an
//! unwritable state file is answered, not raised.
//!
//! The digest itself is SHA-256 from [`fs::Sha256`], not a second hash: one
//! implementation in the binary, and a state file whose contents can be
//! compared against `sha256 <file>` by hand.

use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::exec::{Builtin, Ctx, Output};
use crate::vars;

use super::fs::{Sha256, hex};

/// The builtins implemented in this module, for the interpreter's dispatch.
pub fn lookup(name: &str) -> Option<Builtin> {
    Some(match name {
        "changed" => changed,
        _ => return None,
    })
}

/// The state lives under `$ROOT` rather than beside the outputs: one place to
/// delete, one directory to gitignore, and a task that `cd`s around still
/// finds the record it wrote last time.
const DIR: &str = ".chore";
const FILE: &str = "state";

/// First line of the state file. A run that does not recognise it starts from
/// scratch instead of misreading records written by a later format, which is
/// safe because state is a cache: the worst a fresh start costs is one extra
/// build.
const HEADER: &str = "chore state v1";

/// `changed <path...>`. Exit 0 when any path differs from the last recorded
/// run, 1 when every one of them is unchanged.
///
/// Exit 0 records the new state, so the next run sees "unchanged"; exit 1
/// records nothing, since the record it would write is the one already there.
/// `--force` reports changed without looking, because a forced run is a
/// request to do the work anyway, and `--dry` reads but never writes: a
/// preview that recorded state would leave the next real run believing work
/// it never did was already done.
fn changed(ctx: &mut Ctx<'_>) -> Result<Output> {
    let args = ctx.rest().to_vec();
    if args.is_empty() {
        return Err(Error::Run {
            message: "usage: changed <path...>".to_string(),
        });
    }

    let key = key(ctx.task, &args);
    let digest = snapshot(ctx, &args);
    let path = state_file(ctx.root);
    let mut lines = load(&path);

    // `--force` still records, so the run it forces leaves the same trail a
    // normal one would; skipping the write would make the *next* unforced run
    // redo the work as well.
    let differs = ctx.force || record(&lines, &key).is_none_or(|old| old != digest);
    if !differs {
        return Ok(Output::failed(1));
    }
    if !ctx.dry {
        put(&mut lines, &key, &digest, &label(ctx.task, &args));
        if let Err(e) = save(&path, &lines) {
            // A state file we cannot write means the next run repeats the
            // work: wasteful, not wrong, and nothing worth stopping a build
            // over. Say so once and answer the question that was asked.
            let _ = writeln!(
                ctx.err,
                "changed: cannot record state in {}: {e}",
                vars::display(&path)
            );
        }
    }
    Ok(Output::ok())
}

// --- hashing the inputs ----------------------------------------------------

/// One digest over every argument, in the order they were written.
///
/// The argument's own text goes into the hash before its contents, so
/// `changed src` and `changed lib` cannot collide on identical trees, and the
/// digest changes when the chorefile changes what it watches.
fn snapshot(ctx: &Ctx<'_>, args: &[String]) -> String {
    let mut hasher = Sha256::new();
    for arg in args {
        field(&mut hasher, arg.as_bytes());
        entry(&mut hasher, &ctx.path(arg), arg);
    }
    hex(&hasher.finish())
}

/// One entry: a tag saying what it is, its path relative to the argument, and
/// its contents when it has any.
///
/// The relative path is hashed for every entry, so a rename inside a watched
/// directory is a change even though the bytes underneath are identical. A
/// missing path is hashed as missing rather than skipped, which is what makes
/// a delete a change and a re-creation a change again.
fn entry(hasher: &mut Sha256, path: &Path, rel: &str) {
    field(hasher, rel.as_bytes());
    match fs::symlink_metadata(path) {
        Err(_) => field(hasher, b"missing"),
        // `symlink_metadata`, so a symlinked directory is hashed as a link
        // and not walked: the same rule `find` uses, and the one that keeps a
        // cycle from hanging the check.
        Ok(meta) if meta.is_dir() => {
            field(hasher, b"dir");
            let mut names: Vec<OsString> = match fs::read_dir(path) {
                Ok(entries) => entries.flatten().map(|e| e.file_name()).collect(),
                Err(_) => return field(hasher, b"unreadable"),
            };
            // Sorted, so two runs over the same tree hash the same however
            // the filesystem happens to order its directory.
            names.sort();
            for name in names {
                let name = name.to_string_lossy().into_owned();
                entry(hasher, &path.join(&name), &format!("{rel}/{name}"));
            }
        }
        Ok(_) => {
            field(hasher, b"file");
            if !contents(hasher, path) {
                field(hasher, b"unreadable");
            }
        }
    }
}

/// Stream a file into the hash. False when it could not be read, which is an
/// input state like any other: a file that becomes readable again is a
/// change, and one that stays unreadable is not.
fn contents(hasher: &mut Sha256, path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    loop {
        match file.read(&mut buf) {
            Ok(0) => return true,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return false,
        }
    }
}

/// Length-prefixed, so no arrangement of names and bytes can hash the same as
/// a different arrangement: `changed ab c` and `changed a bc` are different
/// questions and must have different answers.
fn field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

// --- the state file --------------------------------------------------------

/// The key a record is filed under: the calling task and the exact argument
/// list. Two tasks watching the same paths ask separate questions and must
/// not answer each other's, and one task watching two sets of paths keeps two
/// records.
///
/// It is hashed rather than written out because a task name and its arguments
/// can contain anything, tabs and newlines included, and a fixed-width key
/// keeps the line format from needing an escaping scheme. The readable form
/// survives as the label.
fn key(task: &str, args: &[String]) -> String {
    let mut hasher = Sha256::new();
    field(&mut hasher, task.as_bytes());
    for arg in args {
        field(&mut hasher, arg.as_bytes());
    }
    hex(&hasher.finish())
}

/// The human-readable third field. Nothing reads it back: it is there so that
/// someone looking at the file can tell which task a record belongs to.
/// Control characters become spaces, since the format is one record per line.
fn label(task: &str, args: &[String]) -> String {
    let text = if task.is_empty() {
        args.join(" ")
    } else {
        format!("{task}: {}", args.join(" "))
    };
    text.chars()
        .map(|c| if c.is_control() || c == '\t' { ' ' } else { c })
        .take(200)
        .collect()
}

fn state_file(root: &Path) -> PathBuf {
    root.join(DIR).join(FILE)
}

/// The record lines, without the header. A missing file, an unreadable one
/// and one written by a format we do not know all read as "no records".
fn load(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some(HEADER) {
        return Vec::new();
    }
    lines
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The digest stored for `key`, if any. Fields after the second are ignored,
/// which is the room a later version has to add one without this version
/// choking on it.
fn record<'a>(lines: &'a [String], key: &str) -> Option<&'a str> {
    lines.iter().find_map(|line| {
        let (k, rest) = line.split_once('\t')?;
        (k == key).then(|| rest.split('\t').next().unwrap_or_default())
    })
}

/// Replace this key's record, or append it. Other tasks' records are carried
/// through untouched: the file is shared, and a run must only ever rewrite
/// its own line.
fn put(lines: &mut Vec<String>, key: &str, digest: &str, label: &str) {
    let line = format!("{key}\t{digest}\t{label}");
    match lines.iter().position(|l| l.split('\t').next() == Some(key)) {
        Some(at) => lines[at] = line,
        None => lines.push(line),
    }
}

/// Write through a temp file in the same directory and rename over the old
/// one, so an interrupted run leaves either the previous state or the new
/// one, never half a file that would be read as a truncated record set.
fn save(path: &Path, lines: &[String]) -> std::io::Result<()> {
    let mut text = String::from(HEADER);
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    text.push('\n');

    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let temp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
    fs::write(&temp, &text)?;
    if let Err(e) = fs::rename(&temp, path) {
        // Windows can refuse the rename while something else holds the
        // destination open; the direct write is the honest fallback, and the
        // temp file must not be left behind either way.
        let _ = fs::remove_file(&temp);
        return fs::write(path, &text).map_err(|_| e);
    }
    Ok(())
}
