//! Behaviour of the filesystem and utility builtins.
//!
//! Each test gets its own directory under the system temp dir and removes it
//! afterwards. Assertions that depend on Unix permission bits are gated, so
//! the suite is meaningful on Windows too.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use chorefile::builtins::fs as builtins;
use chorefile::error::{Error, Result};
use chorefile::exec::{Ctx, Output};

/// A temp directory that cleans itself up, so a failing assertion does not
/// leave litter behind.
struct Dir(PathBuf);

impl Dir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("chorefile-fs-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parents");
        }
        fs::write(&path, text).expect("write");
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Run a builtin in `dir`, returning its result and both of its streams.
/// Both sinks are buffers, as they are under a capture or a redirect, so
/// `interactive` is false — the same view a `$(...)` gets.
fn run_streams(dir: &Path, dry: bool, args: &[&str]) -> (Result<Output>, String, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let builtin = builtins::lookup(&args[0]).unwrap_or_else(|| panic!("no builtin {}", args[0]));
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let result = {
        let mut ctx = Ctx {
            args: &args,
            cwd: dir,
            root: dir,
            stdin: None,
            dry,
            out: &mut out,
            err: &mut err,
            interactive: false,
        };
        builtin(&mut ctx)
    };
    (
        result,
        String::from_utf8(out).expect("utf-8 output"),
        String::from_utf8(err).expect("utf-8 diagnostics"),
    )
}

/// Run a builtin in `dir`, returning its result and whatever it printed.
fn run_in(dir: &Path, dry: bool, args: &[&str]) -> (Result<Output>, String) {
    let (result, out, _) = run_streams(dir, dry, args);
    (result, out)
}

/// The common case: expect success, return what was printed.
fn run(dir: &Path, args: &[&str]) -> String {
    let (result, out) = run_in(dir, false, args);
    result.expect("builtin failed");
    out
}

fn code(dir: &Path, args: &[&str]) -> i32 {
    let (result, _) = run_in(dir, false, args);
    result.expect("builtin errored").code
}

fn message(result: Result<Output>) -> String {
    match result {
        Err(Error::Run { message }) => message,
        other => panic!("expected a run error, got {other:?}"),
    }
}

// --- lookup ----------------------------------------------------------------

#[test]
fn lookup_covers_this_modules_builtins_only() {
    for name in [
        "copy", "move", "remove", "mkdir", "chmod", "which", "find", "read", "write", "sha256",
        "exists", "echo", "env", "fail", "sleep",
    ] {
        assert!(builtins::lookup(name).is_some(), "{name} missing");
    }
    for name in ["download", "extract", "archive", "cp", ""] {
        assert!(builtins::lookup(name).is_none(), "{name} unexpected");
    }
}

// --- echo / read / write ---------------------------------------------------

#[test]
fn echo_joins_with_spaces_and_ends_with_a_newline() {
    let dir = Dir::new("echo");
    assert_eq!(run(dir.path(), &["echo", "a", "b", "c"]), "a b c\n");
    assert_eq!(run(dir.path(), &["echo"]), "\n");
}

#[test]
fn write_overwrites_and_read_trims() {
    let dir = Dir::new("write");
    run(dir.path(), &["write", "out/note.txt", "first"]);
    run(dir.path(), &["write", "out/note.txt", "second"]);
    assert_eq!(run(dir.path(), &["read", "out/note.txt"]), "second\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("out/note.txt")).unwrap(),
        "second\n"
    );
}

#[test]
fn read_of_a_missing_file_names_the_path() {
    let dir = Dir::new("read-missing");
    let (result, _) = run_in(dir.path(), false, &["read", "nope.txt"]);
    let msg = message(result);
    assert!(msg.starts_with("read: cannot read "), "{msg}");
    assert!(msg.contains("nope.txt"), "{msg}");
}

#[test]
fn wrong_arity_gets_a_usage_message() {
    let dir = Dir::new("usage");
    let (result, _) = run_in(dir.path(), false, &["copy", "only-one"]);
    assert_eq!(message(result), "usage: copy <src> <dest>");
    let (result, _) = run_in(dir.path(), false, &["mkdir"]);
    assert_eq!(message(result), "usage: mkdir <path...>");
}

// --- copy / move -----------------------------------------------------------

#[test]
fn copy_duplicates_a_file() {
    let dir = Dir::new("copy-file");
    dir.write("a.txt", "hello");
    run(dir.path(), &["copy", "a.txt", "b.txt"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "hello"
    );
    assert!(dir.path().join("a.txt").exists());
}

#[test]
fn copy_is_recursive() {
    let dir = Dir::new("copy-tree");
    dir.write("src/one.txt", "1");
    dir.write("src/deep/two.txt", "2");
    dir.write("src/deep/deeper/three.txt", "3");
    run(dir.path(), &["copy", "src", "dest"]);

    assert_eq!(
        fs::read_to_string(dir.path().join("dest/one.txt")).unwrap(),
        "1"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("dest/deep/two.txt")).unwrap(),
        "2"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("dest/deep/deeper/three.txt")).unwrap(),
        "3"
    );
}

#[test]
fn copy_into_an_existing_directory_keeps_the_name() {
    let dir = Dir::new("copy-into");
    dir.write("a.txt", "hello");
    run(dir.path(), &["mkdir", "out"]);
    run(dir.path(), &["copy", "a.txt", "out"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("out/a.txt")).unwrap(),
        "hello"
    );
}

#[test]
fn copy_creates_missing_parents() {
    let dir = Dir::new("copy-parents");
    dir.write("a.txt", "hello");
    run(dir.path(), &["copy", "a.txt", "deep/nested/a.txt"]);
    assert!(dir.path().join("deep/nested/a.txt").is_file());
}

#[test]
fn move_renames_and_leaves_nothing_behind() {
    let dir = Dir::new("move");
    dir.write("tree/a.txt", "hello");
    run(dir.path(), &["move", "tree", "moved"]);
    assert!(!dir.path().join("tree").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("moved/a.txt")).unwrap(),
        "hello"
    );
}

#[test]
fn move_of_a_missing_source_fails() {
    let dir = Dir::new("move-missing");
    let (result, _) = run_in(dir.path(), false, &["move", "gone", "there"]);
    assert!(message(result).contains("gone"));
}

// --- remove / mkdir --------------------------------------------------------

#[test]
fn remove_is_recursive_and_quiet_about_missing_paths() {
    let dir = Dir::new("remove");
    dir.write("tree/deep/a.txt", "1");
    dir.write("loose.txt", "2");
    run(
        dir.path(),
        &["remove", "tree", "loose.txt", "never-existed"],
    );
    assert!(!dir.path().join("tree").exists());
    assert!(!dir.path().join("loose.txt").exists());
    // Twice is not an error.
    run(dir.path(), &["remove", "tree"]);
}

#[test]
fn remove_refuses_root_itself() {
    let dir = Dir::new("remove-root");
    dir.write("keep.txt", "1");
    let (result, _) = run_in(dir.path(), false, &["remove", "."]);
    assert!(message(result).contains("$ROOT"));
    assert!(dir.path().join("keep.txt").exists());
}

#[test]
fn remove_refuses_the_filesystem_root() {
    let dir = Dir::new("remove-fs-root");
    let root = if cfg!(windows) { "C:/" } else { "/" };
    let (result, _) = run_in(dir.path(), false, &["remove", root]);
    assert!(message(result).contains("filesystem root"));
}

#[test]
fn mkdir_creates_parents_and_tolerates_existing() {
    let dir = Dir::new("mkdir");
    run(dir.path(), &["mkdir", "a/b/c", "d"]);
    assert!(dir.path().join("a/b/c").is_dir());
    assert!(dir.path().join("d").is_dir());
    run(dir.path(), &["mkdir", "a/b/c"]);
}

// --- exists / which --------------------------------------------------------

#[test]
fn exists_reports_through_the_exit_code() {
    let dir = Dir::new("exists");
    dir.write("here.txt", "1");
    assert_eq!(code(dir.path(), &["exists", "here.txt"]), 0);
    assert_eq!(code(dir.path(), &["exists", "gone.txt"]), 1);
    assert_eq!(code(dir.path(), &["exists", "."]), 0);
    assert_eq!(run(dir.path(), &["exists", "here.txt"]), "");
}

#[test]
fn which_finds_a_program_on_path() {
    let dir = Dir::new("which");
    let name = if cfg!(windows) { "cmd" } else { "sh" };
    let out = run(dir.path(), &["which", name]);
    assert!(out.trim().ends_with(name) || out.to_lowercase().contains(name));
    assert!(!out.contains('\\'), "paths are printed with `/`: {out}");
}

#[test]
fn which_exits_nonzero_when_absent() {
    let dir = Dir::new("which-absent");
    let (result, out) = run_in(
        dir.path(),
        false,
        &["which", "definitely-not-a-real-program-xyzzy"],
    );
    assert_eq!(result.expect("no error").code, 1);
    assert_eq!(out, "");
}

#[cfg(unix)]
#[test]
fn which_honours_the_execute_bit_for_an_explicit_path() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Dir::new("which-path");
    let script = dir.write("bin/tool", "#!/bin/sh\n");
    assert_eq!(code(dir.path(), &["which", "bin/tool"]), 1);
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        run(dir.path(), &["which", "bin/tool"])
            .trim()
            .ends_with("tool")
    );
}

// --- find ------------------------------------------------------------------

#[test]
fn find_walks_recursively_and_matches_globs() {
    let dir = Dir::new("find");
    dir.write("src/main.rs", "");
    dir.write("src/lib.rs", "");
    dir.write("src/deep/mod.rs", "");
    dir.write("src/notes.txt", "");

    let out = run(dir.path(), &["find", "src", "*.rs"]);
    let mut lines: Vec<&str> = out.lines().collect();
    lines.sort();
    assert_eq!(lines, ["src/deep/mod.rs", "src/lib.rs", "src/main.rs"]);
}

#[test]
fn find_accepts_several_patterns_and_exact_names() {
    let dir = Dir::new("find-multi");
    dir.write("a/one.rs", "");
    dir.write("a/two.toml", "");
    dir.write("a/Cargo.toml", "");
    dir.write("a/skip.md", "");

    let out = run(dir.path(), &["find", "a", "*.rs", "Cargo.toml"]);
    let mut lines: Vec<&str> = out.lines().collect();
    lines.sort();
    assert_eq!(lines, ["a/Cargo.toml", "a/one.rs"]);
}

#[test]
fn find_supports_question_marks_and_matches_directories() {
    let dir = Dir::new("find-dirs");
    dir.write("root/pkg/file.txt", "");
    dir.write("root/ab.c", "");
    dir.write("root/abc.c", "");

    assert_eq!(run(dir.path(), &["find", "root", "pkg"]), "root/pkg\n");
    assert_eq!(run(dir.path(), &["find", "root", "a?.c"]), "root/ab.c\n");
}

#[test]
fn find_reports_paths_relative_to_the_root_as_written() {
    let dir = Dir::new("find-prefix");
    dir.write("a/b.txt", "");
    assert_eq!(run(dir.path(), &["find", ".", "b.txt"]), "a/b.txt\n");
    assert_eq!(run(dir.path(), &["find", "a/", "b.txt"]), "a/b.txt\n");
}

#[test]
fn find_needs_a_directory() {
    let dir = Dir::new("find-missing");
    let (result, _) = run_in(dir.path(), false, &["find", "nowhere", "*"]);
    assert!(message(result).starts_with("find: not a directory"));
}

// --- env / fail / sleep ----------------------------------------------------

#[test]
fn env_gets_and_sets() {
    let dir = Dir::new("env");
    let name = "CHOREFILE_TEST_ENV_VAR";
    run(dir.path(), &["env", name, "value"]);
    assert_eq!(run(dir.path(), &["env", name]), "value\n");

    // The diagnostic goes to `err`: on stdout it would end up inside a
    // `$(env NAME)` capture, and in `Output::stderr` it would reach nobody.
    let (result, out, err) = run_streams(dir.path(), false, &["env", "CHOREFILE_TEST_UNSET_VAR"]);
    let output = result.expect("no error");
    assert_eq!(output.code, 1);
    assert_eq!(out, "");
    assert!(output.stderr.is_empty());
    assert!(err.contains("is not set"), "diagnostics: {err:?}");
}

#[test]
fn fail_always_errors_with_its_message() {
    let dir = Dir::new("fail");
    let (result, _) = run_in(dir.path(), false, &["fail", "no", "toolchain"]);
    assert_eq!(message(result), "no toolchain");
}

#[test]
fn sleep_accepts_fractional_seconds_and_rejects_junk() {
    let dir = Dir::new("sleep");
    let start = std::time::Instant::now();
    run(dir.path(), &["sleep", "0.05"]);
    assert!(start.elapsed() >= std::time::Duration::from_millis(40));

    let (result, _) = run_in(dir.path(), false, &["sleep", "soon"]);
    assert!(message(result).contains("not a number"));
}

// --- chmod -----------------------------------------------------------------

#[test]
fn chmod_rejects_a_non_octal_mode() {
    let dir = Dir::new("chmod-bad");
    dir.write("f.txt", "");
    let (result, _) = run_in(dir.path(), false, &["chmod", "rwx", "f.txt"]);
    assert!(message(result).contains("not an octal mode"));
}

#[cfg(unix)]
#[test]
fn chmod_sets_the_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Dir::new("chmod");
    let file = dir.write("run.sh", "#!/bin/sh\n");
    run(dir.path(), &["chmod", "755", "run.sh"]);
    let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
}

/// On Windows only the owner-write bit is meaningful, and it maps to the
/// read-only flag.
#[cfg(not(unix))]
#[test]
fn chmod_toggles_the_read_only_flag() {
    let dir = Dir::new("chmod-win");
    let file = dir.write("f.txt", "");
    run(dir.path(), &["chmod", "444", "f.txt"]);
    assert!(fs::metadata(&file).unwrap().permissions().readonly());
    run(dir.path(), &["chmod", "644", "f.txt"]);
    assert!(!fs::metadata(&file).unwrap().permissions().readonly());
}

// --- sha256 ----------------------------------------------------------------

#[test]
fn sha256_matches_the_fips_180_4_vectors() {
    let dir = Dir::new("sha256");
    let cases = [
        (
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        // The 448-bit multi-block vector: padding spills into a second block.
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];
    for (input, expected) in cases {
        dir.write("payload", input);
        assert_eq!(
            run(dir.path(), &["sha256", "payload"]).trim(),
            expected,
            "input {input:?}"
        );
    }
}

#[test]
fn sha256_handles_input_larger_than_one_read() {
    let dir = Dir::new("sha256-big");
    // 1,000,000 * 'a', the fourth standard vector, forces many buffered blocks.
    dir.write("payload", &"a".repeat(1_000_000));
    assert_eq!(
        run(dir.path(), &["sha256", "payload"]).trim(),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

// --- dry mode --------------------------------------------------------------

#[test]
fn dry_skips_everything_with_an_effect() {
    let dir = Dir::new("dry-effects");
    dir.write("a.txt", "hello");
    dir.write("tree/b.txt", "1");

    for args in [
        vec!["copy", "a.txt", "copied.txt"],
        vec!["move", "a.txt", "moved.txt"],
        vec!["remove", "tree"],
        vec!["mkdir", "made"],
        vec!["chmod", "700", "a.txt"],
        vec!["write", "written.txt", "text"],
        vec!["env", "CHOREFILE_TEST_DRY_VAR", "set"],
        vec!["sleep", "30"],
    ] {
        let (result, out) = run_in(dir.path(), true, &args);
        assert!(result.expect("dry run failed").success(), "{args:?}");
        assert_eq!(out, "", "{args:?}");
    }

    assert!(!dir.path().join("copied.txt").exists());
    assert!(!dir.path().join("moved.txt").exists());
    assert!(!dir.path().join("made").exists());
    assert!(!dir.path().join("written.txt").exists());
    assert!(dir.path().join("a.txt").exists());
    assert!(dir.path().join("tree/b.txt").exists());
    assert!(std::env::var("CHOREFILE_TEST_DRY_VAR").is_err());
}

#[test]
fn dry_still_runs_the_read_only_builtins() {
    let dir = Dir::new("dry-reads");
    dir.write("a.txt", "hello");

    let (result, out) = run_in(dir.path(), true, &["read", "a.txt"]);
    assert!(result.unwrap().success());
    assert_eq!(out, "hello\n");

    let (result, out) = run_in(dir.path(), true, &["find", ".", "a.txt"]);
    assert!(result.unwrap().success());
    assert_eq!(out, "a.txt\n");

    let (result, out) = run_in(dir.path(), true, &["sha256", "a.txt"]);
    assert!(result.unwrap().success());
    assert_eq!(
        out.trim(),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );

    let (result, _) = run_in(dir.path(), true, &["exists", "a.txt"]);
    assert!(result.unwrap().success());
    let (result, _) = run_in(dir.path(), true, &["exists", "gone.txt"]);
    assert_eq!(result.unwrap().code, 1);

    let (result, out) = run_in(dir.path(), true, &["echo", "still", "printed"]);
    assert!(result.unwrap().success());
    assert_eq!(out, "still printed\n");

    let (result, _) = run_in(dir.path(), true, &["which", "definitely-not-real-xyzzy"]);
    assert_eq!(result.unwrap().code, 1);
}

// --- path convention -------------------------------------------------------

#[test]
fn printed_paths_always_use_forward_slashes() {
    let dir = Dir::new("slashes");
    dir.write("a/b/c.txt", "");
    let out = run(dir.path(), &["find", "a", "c.txt"]);
    assert_eq!(out, "a/b/c.txt\n");
    assert!(!out.contains('\\'));
}
