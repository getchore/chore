//! Behaviour of `changed`, the up-to-date check.
//!
//! Every test gets its own directory, which doubles as `$ROOT`, so the state
//! file each one writes is its own and the suite can run in parallel.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use chorefile::builtins::state;
use chorefile::error::{Error, Result};
use chorefile::exec::{Ctx, EnvOverlay, Output};

/// A temp directory that cleans itself up, so a failing assertion does not
/// leave litter behind.
struct Dir(PathBuf);

impl Dir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chorefile-state-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parents");
        }
        fs::write(&path, text).expect("write");
    }

    fn state(&self) -> Option<String> {
        fs::read_to_string(self.0.join(".chore").join("state")).ok()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One call, with the two flags the interpreter would supply from `--dry` and
/// `--force` and the name of the task the command sits in.
fn call(dir: &Path, task: &str, dry: bool, force: bool, argv: &[&str]) -> Result<Output> {
    let args: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let builtin = state::lookup(&args[0]).expect("builtin");
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let mut ctx = Ctx {
        args: &args,
        cwd: dir,
        root: dir,
        task,
        stdin: None,
        env: &EnvOverlay::default(),
        dry,
        force,
        out: &mut out,
        err: &mut err,
        interactive: false,
    };
    builtin(&mut ctx)
}

/// True when `changed` said "changed". Anything but a clean 0 or 1 is a bug:
/// this builtin answers, it never fails.
fn changed_for(dir: &Path, task: &str, argv: &[&str]) -> bool {
    let out = call(dir, task, false, false, argv).expect("changed errored");
    assert!(
        out.code == 0 || out.code == 1,
        "unexpected exit {}",
        out.code
    );
    out.success()
}

fn changed(dir: &Path, argv: &[&str]) -> bool {
    changed_for(dir, "build", argv)
}

// --- lookup ----------------------------------------------------------------

#[test]
fn lookup_covers_this_modules_builtins_only() {
    assert!(state::lookup("changed").is_some());
    for name in ["exists", "sha256", "read", ""] {
        assert!(state::lookup(name).is_none(), "{name} unexpected");
    }
}

#[test]
fn no_arguments_is_a_usage_error() {
    let dir = Dir::new("usage");
    match call(dir.path(), "build", false, false, &["changed"]) {
        Err(Error::Run { message }) => assert!(message.contains("usage: changed"), "{message}"),
        other => panic!("expected a usage error, got {other:?}"),
    }
}

// --- the basic question ----------------------------------------------------

#[test]
fn first_run_is_changed_and_the_second_is_not() {
    let dir = Dir::new("first");
    dir.write("Cargo.toml", "[package]\n");
    assert!(changed(dir.path(), &["changed", "Cargo.toml"]));
    assert!(!changed(dir.path(), &["changed", "Cargo.toml"]));
}

#[test]
fn editing_a_file_is_changed_again() {
    let dir = Dir::new("edit");
    dir.write("Cargo.toml", "[package]\n");
    assert!(changed(dir.path(), &["changed", "Cargo.toml"]));
    dir.write("Cargo.toml", "[package]\nname = \"x\"\n");
    assert!(changed(dir.path(), &["changed", "Cargo.toml"]));
    assert!(!changed(dir.path(), &["changed", "Cargo.toml"]));
}

#[test]
fn an_unchanged_run_records_nothing() {
    let dir = Dir::new("norecord");
    dir.write("a.txt", "one\n");
    assert!(changed(dir.path(), &["changed", "a.txt"]));
    let after_first = dir.state().expect("state written");
    assert!(!changed(dir.path(), &["changed", "a.txt"]));
    assert_eq!(dir.state().as_deref(), Some(after_first.as_str()));
}

#[test]
fn the_state_file_carries_a_version_marker() {
    let dir = Dir::new("version");
    dir.write("a.txt", "one\n");
    assert!(changed(dir.path(), &["changed", "a.txt"]));
    let text = dir.state().expect("state written");
    assert_eq!(text.lines().next(), Some("chore state v1"));
    // One record, three tab-separated fields, the last of them readable.
    let record = text.lines().nth(1).expect("a record");
    let fields: Vec<&str> = record.split('\t').collect();
    assert_eq!(fields.len(), 3, "{record}");
    assert_eq!(fields[0].len(), 64, "{record}");
    assert_eq!(fields[1].len(), 64, "{record}");
    assert!(fields[2].contains("build: a.txt"), "{record}");
}

#[test]
fn a_state_file_from_an_unknown_version_is_ignored() {
    let dir = Dir::new("unknown");
    dir.write("a.txt", "one\n");
    dir.write(".chore/state", "chore state v99\nnonsense\n");
    assert!(changed(dir.path(), &["changed", "a.txt"]));
    assert!(!changed(dir.path(), &["changed", "a.txt"]));
}

// --- directories -----------------------------------------------------------

#[test]
fn a_directory_is_hashed_recursively() {
    let dir = Dir::new("recurse");
    dir.write("src/main.rs", "fn main() {}\n");
    dir.write("src/deep/nested/lib.rs", "pub fn a() {}\n");
    assert!(changed(dir.path(), &["changed", "src"]));
    assert!(!changed(dir.path(), &["changed", "src"]));

    // A file several levels down still moves the digest.
    dir.write("src/deep/nested/lib.rs", "pub fn b() {}\n");
    assert!(changed(dir.path(), &["changed", "src"]));
    assert!(!changed(dir.path(), &["changed", "src"]));

    // So does a new file, and so does deleting one again.
    dir.write("src/deep/extra.rs", "\n");
    assert!(changed(dir.path(), &["changed", "src"]));
    fs::remove_file(dir.path().join("src/deep/extra.rs")).expect("remove");
    assert!(changed(dir.path(), &["changed", "src"]));
}

#[test]
fn a_rename_inside_a_directory_is_a_change() {
    let dir = Dir::new("rename");
    dir.write("src/a.rs", "same bytes\n");
    assert!(changed(dir.path(), &["changed", "src"]));
    fs::rename(dir.path().join("src/a.rs"), dir.path().join("src/b.rs")).expect("rename");
    assert!(changed(dir.path(), &["changed", "src"]));
    assert!(!changed(dir.path(), &["changed", "src"]));
}

#[test]
fn an_empty_directory_appearing_is_a_change() {
    let dir = Dir::new("emptydir");
    dir.write("src/a.rs", "x\n");
    assert!(changed(dir.path(), &["changed", "src"]));
    fs::create_dir(dir.path().join("src/sub")).expect("mkdir");
    assert!(changed(dir.path(), &["changed", "src"]));
}

// --- missing paths ---------------------------------------------------------

#[test]
fn a_missing_path_is_changed_once_and_then_stays_recorded() {
    let dir = Dir::new("missing");
    assert!(changed(dir.path(), &["changed", "generated.txt"]));
    // Still missing, and the miss was recorded, so nothing changed.
    assert!(!changed(dir.path(), &["changed", "generated.txt"]));
    // Creating it is a change; deleting it again is another.
    dir.write("generated.txt", "hello\n");
    assert!(changed(dir.path(), &["changed", "generated.txt"]));
    fs::remove_file(dir.path().join("generated.txt")).expect("remove");
    assert!(changed(dir.path(), &["changed", "generated.txt"]));
    assert!(!changed(dir.path(), &["changed", "generated.txt"]));
}

// --- several paths ---------------------------------------------------------

#[test]
fn any_one_of_several_paths_changing_is_enough() {
    let dir = Dir::new("several");
    dir.write("src/main.rs", "fn main() {}\n");
    dir.write("Cargo.toml", "[package]\n");
    assert!(changed(dir.path(), &["changed", "src", "Cargo.toml"]));
    assert!(!changed(dir.path(), &["changed", "src", "Cargo.toml"]));
    dir.write("Cargo.toml", "[package]\nversion = \"2\"\n");
    assert!(changed(dir.path(), &["changed", "src", "Cargo.toml"]));
}

#[test]
fn a_different_argument_list_is_a_different_record() {
    let dir = Dir::new("arglist");
    dir.write("a.txt", "a\n");
    dir.write("b.txt", "b\n");
    assert!(changed(dir.path(), &["changed", "a.txt"]));
    // Same task, different question: it has never been asked before.
    assert!(changed(dir.path(), &["changed", "a.txt", "b.txt"]));
    assert!(!changed(dir.path(), &["changed", "a.txt"]));
    assert!(!changed(dir.path(), &["changed", "a.txt", "b.txt"]));
}

#[test]
fn two_tasks_watching_the_same_paths_do_not_collide() {
    let dir = Dir::new("tasks");
    dir.write("src/main.rs", "fn main() {}\n");
    assert!(changed_for(dir.path(), "build", &["changed", "src"]));
    // `test` has never asked, so the answer it gets is its own.
    assert!(changed_for(dir.path(), "test", &["changed", "src"]));
    assert!(!changed_for(dir.path(), "build", &["changed", "src"]));
    assert!(!changed_for(dir.path(), "test", &["changed", "src"]));

    // One edit, and both of them see it once.
    dir.write("src/main.rs", "fn main() { work() }\n");
    assert!(changed_for(dir.path(), "build", &["changed", "src"]));
    assert!(changed_for(dir.path(), "test", &["changed", "src"]));
    assert!(!changed_for(dir.path(), "build", &["changed", "src"]));

    // Two records, and the second did not overwrite the first.
    let text = dir.state().expect("state written");
    assert_eq!(text.lines().count(), 3, "{text}");
}

// --- --force ---------------------------------------------------------------

#[test]
fn force_always_reports_changed() {
    let dir = Dir::new("force");
    dir.write("a.txt", "one\n");
    assert!(changed(dir.path(), &["changed", "a.txt"]));
    for _ in 0..3 {
        let out = call(dir.path(), "build", false, true, &["changed", "a.txt"]).expect("errored");
        assert!(out.success(), "--force reported unchanged");
    }
    // The forced runs recorded what they saw, so an unforced one that follows
    // does not repeat the work a second time.
    assert!(!changed(dir.path(), &["changed", "a.txt"]));
}

// --- --dry -----------------------------------------------------------------

#[test]
fn dry_reads_the_state_but_never_writes_it() {
    let dir = Dir::new("dry");
    dir.write("a.txt", "one\n");

    // A preview of a first run answers "changed" and leaves no state behind,
    // so the real run that follows still does the work.
    let out = call(dir.path(), "build", true, false, &["changed", "a.txt"]).expect("errored");
    assert!(out.success());
    assert!(dir.state().is_none(), "--dry wrote state");
    assert!(changed(dir.path(), &["changed", "a.txt"]));

    // With state on disk, a preview reads it and answers "unchanged" without
    // touching the file.
    let before = dir.state().expect("state written");
    let out = call(dir.path(), "build", true, false, &["changed", "a.txt"]).expect("errored");
    assert_eq!(out.code, 1);
    assert_eq!(dir.state().as_deref(), Some(before.as_str()));
}
