//! End-to-end tests that prove `chore` *runs* things.
//!
//! `cli.rs` covers the command line and the previews; everything here builds a
//! real chorefile in a temp directory, runs the compiled binary against it, and
//! then looks at the bytes on disk. A test that only checked the exit code
//! would prove almost nothing about an interpreter.
//!
//! No test here touches the network, so `download` is not exercised.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// harness — same shape as `cli.rs`: a hand-rolled temp dir that removes itself
// ---------------------------------------------------------------------------

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "chore-e2e-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(&path, text).expect("write");
        path
    }

    fn chorefile(&self, text: &str) -> &Self {
        self.write("chorefile", text);
        self
    }

    fn exists(&self, name: &str) -> bool {
        self.0.join(name).exists()
    }

    /// The file's contents, or a panic naming the file — a missing file in the
    /// middle of a pipeline is the failure worth reading about.
    fn read(&self, name: &str) -> String {
        let path = self.0.join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn bytes(&self, name: &str) -> Vec<u8> {
        let path = self.0.join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn of(output: Output) -> Self {
        Self {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Panic unless the run succeeded, showing both streams.
    fn ok(self) -> Self {
        assert_eq!(
            self.code, 0,
            "stdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
        self
    }

    /// How many times a line was *printed*. Substring counting would also
    /// match the `$ echo once` the interpreter echoes before running it.
    fn printed(&self, line: &str) -> usize {
        self.stdout.lines().filter(|l| l.trim() == line).count()
    }
}

fn chore_in(dir: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_chore"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run chore");
    Run::of(output)
}

fn chore(dir: &Dir, args: &[&str]) -> Run {
    chore_in(dir.path(), args)
}

// ---------------------------------------------------------------------------
// 1. a real build pipeline
// ---------------------------------------------------------------------------

/// The flagship: stage sources, copy a tree, archive it, hash it, unpack it
/// again and check the digest — all through builtins, all asserted on bytes.
const PIPELINE: &str = r#"
# stage the sources
task stage {
    mkdir build/demo
    write build/demo/main.txt "fn main"
    write build/demo/docs/README "read me"
    copy build/demo build/staged
}

# package the staged tree
task package {
    stage
    mkdir dist
    archive build/staged dist/demo.tar.gz
    write dist/demo.tar.gz.sha256 $(sha256 dist/demo.tar.gz)
}

# unpack the package and check it against its digest
task verify {
    package
    extract dist/demo.tar.gz check
    want=$(read dist/demo.tar.gz.sha256)
    got=$(sha256 dist/demo.tar.gz)
    if $want != $got { fail digest mismatch }
    echo verified $got
}
"#;

#[test]
fn a_build_pipeline_stages_packages_and_verifies_real_files() {
    let dir = Dir::new();
    dir.chorefile(PIPELINE);

    let run = chore(&dir, &["verify"]).ok();

    // The staged tree is a real recursive copy, not just the top directory.
    assert_eq!(dir.read("build/demo/main.txt"), "fn main\n");
    assert_eq!(dir.read("build/staged/main.txt"), "fn main\n");
    assert_eq!(dir.read("build/staged/docs/README"), "read me\n");

    // `archive` names the top-level entry after the source, so the archive
    // unpacks as `check/staged/...` rather than spilling into `check/`.
    assert_eq!(dir.read("check/staged/main.txt"), "fn main\n");
    assert_eq!(dir.read("check/staged/docs/README"), "read me\n");

    let digest = dir.read("dist/demo.tar.gz.sha256");
    let digest = digest.trim();
    assert_eq!(digest.len(), 64, "digest was {digest:?}");
    assert!(
        digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "digest was {digest:?}"
    );
    assert!(
        run.stdout.contains(&format!("verified {digest}")),
        "{}",
        run.stdout
    );

    // The archive is a real file with gzip's magic bytes, not an empty stub.
    let archive = dir.bytes("dist/demo.tar.gz");
    assert_eq!(&archive[..2], &[0x1f, 0x8b], "not a gzip stream");
}

#[test]
fn archiving_the_same_tree_twice_produces_identical_bytes() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task twice {
    mkdir tree/sub
    write tree/a.txt a
    write tree/sub/b.txt b
    archive tree one.tar.gz
    archive tree two.tar.gz
}
"#,
    );

    chore(&dir, &["twice"]).ok();
    assert_eq!(
        dir.bytes("one.tar.gz"),
        dir.bytes("two.tar.gz"),
        "archive is not reproducible"
    );
}

// ---------------------------------------------------------------------------
// 2. run-once semantics
// ---------------------------------------------------------------------------

const DIAMOND: &str = r#"
task d {
    echo tick >> log.txt
}

task b {
    d
}

task c {
    d
}

task a {
    b
    c
    d
}
"#;

#[test]
fn a_task_reached_three_ways_in_a_diamond_runs_once() {
    let dir = Dir::new();
    dir.chorefile(DIAMOND);

    chore(&dir, &["a"]).ok();
    assert_eq!(dir.read("log.txt"), "tick\n");
}

#[test]
fn force_makes_every_reach_of_a_diamond_run_the_task_again() {
    let dir = Dir::new();
    dir.chorefile(DIAMOND);

    chore(&dir, &["a", "--force"]).ok();
    assert_eq!(dir.read("log.txt"), "tick\ntick\ntick\n");
}

#[test]
fn run_once_is_keyed_on_arguments_as_well_as_name() {
    let dir = Dir::new();
    // Keyed on name *and* arguments: a parameterised task called with
    // different arguments has different work to do, so skipping it would be a
    // silently wrong build.
    dir.chorefile(
        r#"
task target name {
    echo $1 >> built.txt
}

task all {
    target linux
    target windows
    target linux
}
"#,
    );

    chore(&dir, &["all"]).ok();
    assert_eq!(dir.read("built.txt"), "linux\nwindows\n");
}

// ---------------------------------------------------------------------------
// 3. cd isolation
// ---------------------------------------------------------------------------

#[test]
fn a_cd_dies_with_the_task_that_made_it() {
    let dir = Dir::new();
    std::fs::create_dir_all(dir.path().join("sub")).expect("sub");
    dir.chorefile(
        r#"
task inner {
    cd sub
    write inside.txt here
}

task outer {
    inner
    write after.txt back
}
"#,
    );

    chore(&dir, &["outer"]).ok();
    assert_eq!(dir.read("sub/inside.txt"), "here\n");
    // The sibling wrote a relative path and must land at $ROOT, not in `sub`.
    assert_eq!(dir.read("after.txt"), "back\n");
    assert!(!dir.exists("sub/after.txt"), "cd leaked out of the callee");
}

// ---------------------------------------------------------------------------
// 4. word splitting
// ---------------------------------------------------------------------------

const SPLITTING: &str = r#"
task record {
    write count.txt $#
    write first.txt "$1"
}

task quoted {
    record "two words"
}

task unquoted {
    v="two words"
    record $v
}

task requoted {
    v="two words"
    record "$v"
}

task star {
    record *
}
"#;

#[test]
fn a_quoted_word_reaches_the_command_as_one_argument() {
    let dir = Dir::new();
    dir.chorefile(SPLITTING);

    chore(&dir, &["quoted"]).ok();
    assert_eq!(dir.read("count.txt"), "1\n");
    assert_eq!(dir.read("first.txt"), "two words\n");
}

#[test]
fn an_unquoted_variable_holding_two_words_arrives_as_two_arguments() {
    let dir = Dir::new();
    dir.chorefile(SPLITTING);

    chore(&dir, &["unquoted"]).ok();
    assert_eq!(dir.read("count.txt"), "2\n");
    assert_eq!(dir.read("first.txt"), "two\n");
}

#[test]
fn quoting_a_variable_at_the_call_site_puts_it_back_together() {
    let dir = Dir::new();
    dir.chorefile(SPLITTING);

    chore(&dir, &["requoted"]).ok();
    assert_eq!(dir.read("count.txt"), "1\n");
    assert_eq!(dir.read("first.txt"), "two words\n");
}

#[test]
fn a_glob_character_reaches_the_command_unexpanded() {
    let dir = Dir::new();
    dir.chorefile(SPLITTING);

    // argv goes to the OS directly, so nothing re-expands `*` on the way out.
    chore(&dir, &["star"]).ok();
    assert_eq!(dir.read("count.txt"), "1\n");
    assert_eq!(dir.read("first.txt"), "*\n");
}

// ---------------------------------------------------------------------------
// 5. chains and redirects
// ---------------------------------------------------------------------------

#[test]
fn and_runs_the_right_side_only_when_the_left_side_succeeded() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task chain {
    try exists nothing-here && write skipped.txt x
    exists chorefile && write ran.txt x
}
"#,
    );

    chore(&dir, &["chain"]).ok();
    assert!(!dir.exists("skipped.txt"), "&& ran after a failure");
    assert_eq!(dir.read("ran.txt"), "x\n");
}

#[test]
fn or_runs_the_right_side_only_when_the_left_side_failed() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task chain {
    exists nothing-here || write fallback.txt used
    try exists chorefile || write unused.txt no
}
"#,
    );

    chore(&dir, &["chain"]).ok();
    assert_eq!(dir.read("fallback.txt"), "used\n");
    assert!(!dir.exists("unused.txt"), "|| ran after a success");
}

#[test]
fn a_pipe_keeps_only_the_right_hand_sides_output() {
    let dir = Dir::new();
    dir.chorefile("task piped {\n    echo left | echo right > out.txt\n}\n");

    chore(&dir, &["piped"]).ok();
    // As in sh, the pipeline's stdout — and its status — are the last
    // command's; the left side's bytes went into the pipe and no further.
    assert_eq!(dir.read("out.txt"), "right\n");
}

#[test]
fn redirects_truncate_and_append() {
    let dir = Dir::new();
    dir.write("kept.txt", "old\n");
    dir.chorefile(
        r#"
task redirect {
    echo first > kept.txt
    echo second > kept.txt
    echo one > stack.txt
    echo two >> stack.txt
    echo three >> stack.txt
}
"#,
    );

    chore(&dir, &["redirect"]).ok();
    assert_eq!(dir.read("kept.txt"), "second\n");
    assert_eq!(dir.read("stack.txt"), "one\ntwo\nthree\n");
}

#[test]
fn a_redirected_builtin_leaves_nothing_for_the_next_command_to_print() {
    let dir = Dir::new();
    dir.chorefile("task chain {\n    echo hidden > cap.txt && echo after\n}\n");

    let run = chore(&dir, &["chain"]).ok();
    assert_eq!(dir.read("cap.txt"), "hidden\n");
    // The bytes went to the file and nowhere else: leaving them in the
    // command's output would let `&&` splice them back into stdout.
    assert_eq!(run.printed("hidden"), 0, "{}", run.stdout);
    assert_eq!(run.printed("after"), 1, "{}", run.stdout);
}

#[test]
fn a_stderr_redirect_catches_a_builtins_diagnostic() {
    let dir = Dir::new();
    // `env <NAME>` reports a miss as exit 1 plus a diagnostic, so `try` keeps
    // fail-fast out of the way while the redirect does its job.
    dir.chorefile(
        r#"
task diag {
    try env CHORE_E2E_DEFINITELY_UNSET 2> err.txt
    echo done
}
"#,
    );

    let run = chore(&dir, &["diag"]).ok();
    assert!(
        dir.read("err.txt").contains("CHORE_E2E_DEFINITELY_UNSET"),
        "err.txt was {:?}",
        dir.read("err.txt")
    );
    assert!(
        !run.stderr.contains("CHORE_E2E_DEFINITELY_UNSET"),
        "the diagnostic also reached the terminal: {}",
        run.stderr
    );
}

// ---------------------------------------------------------------------------
// 6. fail-fast and `try`
// ---------------------------------------------------------------------------

#[test]
fn a_failing_command_stops_the_task_where_it_stood() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task risky {
    write before.txt ok
    fail nope
    write after.txt never
}
"#,
    );

    let run = chore(&dir, &["risky"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert_eq!(dir.read("before.txt"), "ok\n");
    assert!(!dir.exists("after.txt"), "the run continued past a failure");
    assert!(run.stderr.contains("nope"), "{}", run.stderr);
}

#[test]
fn an_unknown_program_fails_the_task_like_any_other_command() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task risky {
    write before.txt ok
    ^chore-e2e-no-such-program
    write after.txt never
}
"#,
    );

    let run = chore(&dir, &["risky"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(!dir.exists("after.txt"));
    assert!(
        run.stderr.contains("chore-e2e-no-such-program"),
        "{}",
        run.stderr
    );
}

#[test]
fn try_lets_the_task_carry_on_past_a_failure() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task risky {
    write before.txt ok
    try fail nope
    try ^chore-e2e-no-such-program
    write after.txt reached
}
"#,
    );

    chore(&dir, &["risky"]).ok();
    assert_eq!(dir.read("before.txt"), "ok\n");
    assert_eq!(dir.read("after.txt"), "reached\n");
}

// ---------------------------------------------------------------------------
// 7. `--dry` has no effects
// ---------------------------------------------------------------------------

#[test]
fn dry_changes_nothing_in_a_populated_tree() {
    let dir = Dir::new();
    dir.write("top.txt", "top\n");
    dir.write("keep/data.txt", "precious\n");
    dir.chorefile(
        r#"
task destroy {
    stamp=$(read top.txt)
    remove keep
    remove top.txt
    mkdir fresh
    write fresh/new.txt $stamp
    copy chorefile backup
    archive fresh fresh.zip
}
"#,
    );

    let run = chore(&dir, &["destroy", "--dry"]).ok();

    assert_eq!(dir.read("top.txt"), "top\n");
    assert_eq!(dir.read("keep/data.txt"), "precious\n");
    for created in ["fresh", "backup", "fresh.zip"] {
        assert!(!dir.exists(created), "--dry created {created}");
    }

    // Every command is still shown...
    for shown in [
        "$ remove keep",
        "$ mkdir fresh",
        "$ archive fresh fresh.zip",
    ] {
        assert!(run.stdout.contains(shown), "{}", run.stdout);
    }
    // ...and the capture really ran: a `$(...)` that was skipped would leave
    // every interpolated value downstream empty, and the preview would
    // describe a run that could never happen.
    assert!(
        run.stdout.contains("$ write fresh/new.txt top"),
        "the capture did not run: {}",
        run.stdout
    );
}

#[test]
fn dry_still_stops_at_a_hard_failure() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task boom {
    echo before
    fail nope
    write after.txt never
}
"#,
    );

    // A preview that swallowed `fail` would describe a run that cannot happen.
    let run = chore(&dir, &["boom", "--dry"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(!dir.exists("after.txt"));
}

// ---------------------------------------------------------------------------
// 8. discovery
// ---------------------------------------------------------------------------

#[test]
fn a_run_from_a_nested_subdirectory_works_from_the_chorefiles_directory() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task where {
    write $ROOT/absolute.txt marker
    write relative.txt marker
}
"#,
    );
    let nested = dir.path().join("a/b/c");
    std::fs::create_dir_all(&nested).expect("nested");

    let run = chore_in(&nested, &["where"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    assert_eq!(dir.read("absolute.txt"), "marker\n");
    // Commands start at $ROOT, not at the directory `chore` was invoked from.
    assert_eq!(dir.read("relative.txt"), "marker\n");
    assert!(!nested.join("relative.txt").exists());
}

// ---------------------------------------------------------------------------
// 9. exit codes
// ---------------------------------------------------------------------------

#[test]
fn a_finished_run_exits_zero_and_leaves_its_work_behind() {
    let dir = Dir::new();
    dir.chorefile("task work {\n    write done.txt yes\n}\n");

    chore(&dir, &["work"]).ok();
    assert_eq!(dir.read("done.txt"), "yes\n");
}

#[test]
fn an_exit_inside_a_called_task_unwinds_the_whole_run_with_its_code() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task inner {
    write inner.txt ok
    exit 3
    write inner-after.txt never
}

task outer {
    inner
    write outer-after.txt never
}
"#,
    );

    let run = chore(&dir, &["outer"]);
    assert_eq!(run.code, 3, "{}{}", run.stdout, run.stderr);
    assert_eq!(dir.read("inner.txt"), "ok\n");
    assert!(!dir.exists("inner-after.txt"));
    assert!(!dir.exists("outer-after.txt"), "the caller kept going");
}

#[test]
fn a_malformed_subcommand_is_a_usage_error_that_touches_nothing() {
    let dir = Dir::new();
    dir.chorefile("task work {\n    write done.txt yes\n}\n");

    let run = chore(&dir, &["list", "--bogus"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(run.stderr.contains("usage"), "{}", run.stderr);
    assert!(!dir.exists("done.txt"));
}

// ---------------------------------------------------------------------------
// top-level statements
// ---------------------------------------------------------------------------

#[test]
fn list_needs_no_io_but_a_run_evaluates_the_globals() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
version=$(read version.txt)

# show the version
task show {
    echo v $version
}
"#,
    );

    // `list` only needs the parse tree, so it works even though the file the
    // global reads is missing.
    let listed = chore(&dir, &["list"]).ok();
    assert!(listed.stdout.contains("show"), "{}", listed.stdout);

    let failed = chore(&dir, &["show"]);
    assert_eq!(failed.code, 1, "{}{}", failed.stdout, failed.stderr);
    assert!(failed.stderr.contains("version.txt"), "{}", failed.stderr);

    dir.write("version.txt", "1.2.3\n");
    let run = chore(&dir, &["show"]).ok();
    assert_eq!(run.printed("v 1.2.3"), 1, "{}", run.stdout);
}

#[test]
fn a_task_wins_over_the_builtin_it_shadows() {
    let dir = Dir::new();
    // SPEC: at runtime a task wins over a builtin of the same name, and it is
    // `check` that reports the shadowing.
    dir.chorefile(
        r#"
task write {
    echo TASKWINS
}

task go {
    write a b
}
"#,
    );

    let run = chore(&dir, &["go"]).ok();
    assert_eq!(run.printed("TASKWINS"), 1, "{}", run.stdout);
    assert!(!dir.exists("a"), "the builtin ran instead of the task");

    let checked = chore(&dir, &["check"]);
    assert_eq!(checked.code, 1, "{}{}", checked.stdout, checked.stderr);
    assert!(checked.stdout.contains("write"), "{}", checked.stdout);
}

/// The line a justfile arrives with: a build that says half of what it has to
/// say on stderr, both halves into one log, and a `check` that is happy with
/// it.
#[test]
#[cfg(unix)]
fn a_merge_puts_both_streams_in_one_log() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task build {
    sh -c "echo compiling; echo warning: unused >&2" > build.log 2>&1
    echo done
}
"#,
    );

    let checked = chore(&dir, &["check"]);
    assert_eq!(checked.code, 0, "{}{}", checked.stdout, checked.stderr);
    let run = chore(&dir, &["build"]).ok();
    assert_eq!(dir.read("build.log"), "compiling\nwarning: unused\n");
    // Neither half reached the terminal, and the run carried on.
    assert!(!run.stderr.contains("unused"), "stderr was {}", run.stderr);
    assert!(run.stdout.contains("done"), "stdout was {}", run.stdout);
}

// ---------------------------------------------------------------------------
// bugs
// ---------------------------------------------------------------------------

#[test]
fn a_stderr_redirect_catches_the_message_from_a_builtin_that_fails() {
    let dir = Dir::new();
    dir.chorefile(
        r#"
task diag {
    try read missing.txt 2> err.txt
    echo done
}
"#,
    );

    let run = chore(&dir, &["diag"]).ok();
    // A `2>` on a program on PATH always creates the file, because the file is
    // opened before the child is spawned. A builtin returns its failure as an
    // error that unwinds past the code writing the file, so the redirect
    // catches nothing at exactly the moment there is something to catch.
    assert!(dir.exists("err.txt"), "2> created no file: {}", run.stdout);
    assert!(
        dir.read("err.txt").contains("missing.txt"),
        "err.txt was {:?}",
        dir.read("err.txt")
    );
    assert!(
        !run.stderr.contains("missing.txt"),
        "the diagnostic escaped the redirect: {}",
        run.stderr
    );
}

// ---------------------------------------------------------------------------
// include — a real second file on disk
//
// `include` is followed by `chorefile::resolve`, and `check` walks every file
// that contributed through `chorefile::check::check_path`. Both landed, so
// every test here runs.
// ---------------------------------------------------------------------------

/// A project with both kinds of include: one flat, one namespaced, and the
/// namespaced file in a subdirectory so `$ROOT` has somewhere wrong to point.
fn included_project() -> Dir {
    let dir = Dir::new();
    dir.chorefile(
        "include tasks.chore\n\
         include libs/chorefile as libs\n\
         \n\
         # build the project\n\
         task build {\n\
         \x20   echo top\n\
         }\n",
    );
    dir.write(
        "tasks.chore",
        "# lint the sources\n\
         task lint {\n\
         \x20   echo linting\n\
         }\n",
    );
    dir.write(
        "libs/chorefile",
        "# build the vendored library\n\
         task build {\n\
         \x20   echo lib building $1\n\
         \x20   write $ROOT/where.txt here\n\
         }\n",
    );
    dir
}

#[test]
fn list_shows_flat_and_namespaced_tasks_from_the_included_files() {
    let dir = included_project();
    let run = chore(&dir, &["list"]).ok();
    // The first line says which chorefile answered — the top-level one, not
    // any of the files it includes — and the tasks follow it.
    let first = run.stdout.lines().next().unwrap_or_default();
    assert!(first.starts_with("using chorefile, $ROOT = "), "{first}");
    let rows: Vec<&str> = run.stdout.lines().skip(1).collect();
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    // Merge order: each include, then the including file's own tasks. A flat
    // include keeps its bare name, `as libs` prefixes it.
    assert_eq!(names, ["lint", "libs::build", "build"], "{}", run.stdout);
    assert!(run.stdout.contains("lint the sources"), "{}", run.stdout);
    assert!(
        run.stdout.contains("build the vendored library"),
        "{}",
        run.stdout
    );

    // The descriptions line up in one column, sized by the longest name. The
    // provenance line is not a row and takes no part in the column.
    let columns: Vec<usize> = rows
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            // Where the description starts: past the indent, past the name,
            // past the padding.
            let name_end = 2 + l.split_whitespace().next().unwrap_or("").len();
            name_end + l[name_end..].chars().take_while(|c| *c == ' ').count()
        })
        .collect();
    assert!(
        columns.windows(2).all(|w| w[0] == w[1]),
        "descriptions are not aligned:\n{}",
        run.stdout
    );
}

#[test]
fn runs_a_task_from_a_flat_include() {
    let dir = included_project();
    let run = chore(&dir, &["lint"]).ok();
    assert_eq!(run.printed("linting"), 1, "{}", run.stdout);
}

#[test]
fn runs_a_namespaced_task_with_its_arguments_and_the_top_level_root() {
    let dir = included_project();
    let run = chore(&dir, &["libs::build", "sona"]).ok();
    assert_eq!(run.printed("lib building sona"), 1, "{}", run.stdout);
    // `$ROOT` is the top-level chorefile's directory in an included file too,
    // so the write lands beside the top-level chorefile, not in `libs/`.
    assert!(dir.exists("where.txt"), "{}", run.stdout);
    assert!(!dir.exists("libs/where.txt"), "$ROOT followed the include");

    // The bare name of a namespaced task is not a task.
    let bare = chore(&dir, &["build"]).ok();
    assert_eq!(bare.printed("top"), 1, "{}", bare.stdout);
}

#[test]
fn list_json_names_the_file_and_namespace_each_task_came_from() {
    let dir = included_project();
    let run = chore(&dir, &["list", "--json"]).ok();
    // Paths come back from the canonical working directory, which on macOS is
    // `/private/var/...` where the temp dir is `/var/...`.
    let root = dir.path().canonicalize().expect("canonical");
    let included = root.join("tasks.chore");
    let vendored = root.join("libs/chorefile");
    let top = root.join("chorefile");
    for (name, namespace, file) in [
        ("lint", "null", &included),
        ("libs::build", "\"libs\"", &vendored),
        ("build", "null", &top),
    ] {
        let line = run
            .stdout
            .lines()
            .find(|l| l.contains(&format!("\"name\": \"{name}\"")))
            .unwrap_or_else(|| panic!("no `{name}` in\n{}", run.stdout));
        assert!(
            line.contains(&format!("\"namespace\": {namespace}")),
            "{line}"
        );
        // Reported with `/` on every platform, so compare on the tail rather
        // than on this host's spelling of an absolute path.
        let tail = suffix(file);
        assert!(
            line.contains(&format!("/{tail}\"")),
            "{line}\nexpected the file field to end with /{tail}"
        );
    }
}

#[test]
fn dry_and_force_still_hold_across_an_include() {
    let dir = Dir::new();
    dir.chorefile("include tasks.chore\n");
    dir.write(
        "tasks.chore",
        "task once {\n\
         \x20   write marker.txt x\n\
         }\n\
         \n\
         task both {\n\
         \x20   once\n\
         \x20   once\n\
         }\n",
    );

    // --dry echoes without touching the disk, in an included file too.
    let dry = chore(&dir, &["once", "--dry"]).ok();
    assert!(dry.stdout.contains("marker.txt"), "{}", dry.stdout);
    assert!(!dir.exists("marker.txt"), "--dry wrote the file");

    // run-once and --force survive the merge.
    let plain = chore(&dir, &["both"]).ok();
    assert_eq!(
        plain.stdout.matches("$ write").count(),
        1,
        "{}",
        plain.stdout
    );
    let forced = chore(&dir, &["both", "--force"]).ok();
    assert_eq!(
        forced.stdout.matches("$ write").count(),
        2,
        "{}",
        forced.stdout
    );
}

#[test]
fn a_missing_include_is_a_clean_error_with_exit_two() {
    let dir = Dir::new();
    dir.chorefile("include vendor/tasks.chore\n\ntask build {\n    echo top\n}\n");
    let run = chore(&dir, &["list"]);
    assert_eq!(
        run.code, 2,
        "stdout:\n{}\nstderr:\n{}",
        run.stdout, run.stderr
    );
    assert!(run.stderr.starts_with("chore: "), "{}", run.stderr);
    // The path that could not be read is named, and nothing is debug-printed.
    assert!(run.stderr.contains("vendor/tasks.chore"), "{}", run.stderr);
    assert!(
        !run.stderr.contains('{') && !run.stderr.contains("Error"),
        "debug-printed error: {}",
        run.stderr
    );
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
}

#[test]
fn an_include_cycle_is_a_clean_error_with_exit_two() {
    let dir = Dir::new();
    dir.chorefile("include loop.chore\n");
    dir.write("loop.chore", "include chorefile\n");
    let run = chore(&dir, &["list"]);
    assert_eq!(
        run.code, 2,
        "stdout:\n{}\nstderr:\n{}",
        run.stdout, run.stderr
    );
    assert!(
        run.stderr.to_lowercase().contains("cycle"),
        "{}",
        run.stderr
    );
    assert_eq!(run.stderr.lines().count(), 1, "{}", run.stderr);
}

#[test]
fn a_missing_task_says_which_files_were_searched() {
    let dir = included_project();
    let run = chore(&dir, &["nope"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(run.stderr.contains("no task `nope`"), "{}", run.stderr);
    assert!(run.stderr.contains("chorefile"), "{}", run.stderr);
    // Naming only the top-level file would send the reader to the wrong place.
    assert!(run.stderr.contains("includes"), "{}", run.stderr);
}

#[test]
fn check_reports_a_finding_inside_the_included_file() {
    let dir = Dir::new();
    dir.chorefile("include tasks.chore\n\ntask build {\n    echo top\n}\n");
    let source =
        "# fetch the sdk\ntask sdk {\n    curl -L https://example.com/sdk.zip -o sdk.zip\n}\n";
    dir.write("tasks.chore", source);

    let run = chore(&dir, &["check"]);
    assert_eq!(
        run.code, 1,
        "stdout:\n{}\nstderr:\n{}",
        run.stdout, run.stderr
    );

    // The finding points into the file it lives in, at that file's line and
    // column — not at the offset read against the top-level chorefile.
    let offset = source.find("curl").expect("curl in the source");
    let line = source[..offset].matches('\n').count() + 1;
    let col = offset - source[..offset].rfind('\n').map_or(0, |i| i + 1) + 1;
    let expected = format!(
        "{}:{line}:{col}",
        chorefile::vars::display(&dir.path().join("tasks.chore"))
    );
    assert!(
        run.stdout.contains(&expected),
        "want {expected} in\n{}",
        run.stdout
    );
    assert!(run.stdout.contains("download"), "{}", run.stdout);
}

#[test]
fn check_reports_a_missing_include_as_a_finding_rather_than_giving_up() {
    let dir = Dir::new();
    dir.chorefile("include vendor/tasks.chore\n\ntask build {\n    echo top\n}\n");

    // `check` is a gate: it says what is wrong and where, and exits 1 like any
    // other error it finds. Running the same project is exit 2 instead, since
    // there is nothing to run.
    let checked = chore(&dir, &["check"]);
    assert_eq!(checked.code, 1, "{}{}", checked.stdout, checked.stderr);
    let at = format!("/{}:1:1", suffix(&dir.path().join("chorefile")));
    assert!(
        checked.stdout.contains(&at),
        "want a location ending {at} in\n{}",
        checked.stdout
    );
    assert!(
        checked.stdout.contains("vendor/tasks.chore"),
        "{}",
        checked.stdout
    );
}

/// The last two components of a path, `/`-joined: enough to identify a file
/// in a per-test temp directory without asserting how this platform spells an
/// absolute path. `canonicalize` is deliberately not used — on Windows it
/// prepends a `\\?\\` verbatim prefix that `chore` never emits.
fn suffix(path: &Path) -> String {
    let mut parts: Vec<String> = path
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.reverse();
    parts.join("/")
}

// completions
// ---------------------------------------------------------------------------

#[test]
fn list_names_is_one_task_per_line_with_a_tab() {
    let dir = Dir::new();
    dir.chorefile(
        "# build the thing\ntask build {\n    echo hi\n}\n\ntask bare {\n    echo hi\n}\n",
    );
    let out = chore(&dir, &["list", "--names"]).ok().stdout;
    // A task with no comment still prints its tab, so every line splits the
    // same way in a shell.
    assert_eq!(out, "build\tbuild the thing\nbare\t\n");
}

#[test]
fn completions_needs_no_chorefile() {
    // The script is about the shell, not about a project. Someone installing
    // completions has usually not cd'd anywhere in particular yet.
    let dir = Dir::new();
    let out = chore(&dir, &["completions", "zsh"]).ok().stdout;
    assert!(out.contains("compdef _chore chore"), "{out}");
}

#[test]
fn every_shell_script_asks_chore_for_the_task_list() {
    let dir = Dir::new();
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let out = chore(&dir, &["completions", shell]).ok().stdout;
        assert!(
            out.contains("chore list --names"),
            "{shell} script must call chore, not embed a task list:\n{out}"
        );
    }
}

#[test]
fn an_unknown_shell_is_a_usage_error() {
    let dir = Dir::new();
    let run = chore(&dir, &["completions", "nushell"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(
        run.stderr.contains("bash, zsh, fish, powershell"),
        "{}",
        run.stderr
    );
}

#[test]
fn completions_cannot_be_shadowed_by_a_task() {
    let dir = Dir::new();
    dir.chorefile("task completions {\n    echo shadowed\n}\n");
    // The subcommand wins, exactly as `list` does, which is why the name is
    // reserved. `check` is where the author is told.
    let out = chore(&dir, &["completions", "bash"]).ok().stdout;
    assert!(!out.contains("shadowed"), "{out}");
    let run = chore(&dir, &["check"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.contains("completions"), "{}", run.stdout);
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

#[test]
fn init_writes_a_chorefile_with_no_chorefile_anywhere() {
    // The whole point of the command: someone standing in an empty directory,
    // with nothing to discover, ends up with a file to edit.
    let dir = Dir::new();
    let run = chore(&dir, &["init"]).ok();
    assert!(dir.exists("chorefile"), "init wrote nothing");
    assert!(run.stdout.contains("chorefile"), "{}", run.stdout);
    // It must be a starting point, not a tutorial.
    let written = dir.read("chorefile");
    assert!(
        written.lines().count() < 25,
        "starter is {} lines:\n{written}",
        written.lines().count()
    );
}

#[test]
fn what_init_writes_passes_check_with_no_findings() {
    // A first chorefile that chore's own linter complains about would be the
    // worst possible introduction, so the file is checked by the same binary
    // that wrote it rather than by inspection.
    let dir = Dir::new();
    chore(&dir, &["init"]).ok();
    let run = chore(&dir, &["check"]).ok();
    assert!(!run.stdout.contains("warning"), "{}", run.stdout);
    assert!(!run.stdout.contains("error"), "{}", run.stdout);
}

#[test]
fn what_init_writes_runs_and_describes_itself() {
    // `chore list` reads the comment above a task as its description, and the
    // starter exists partly to teach that convention: every task it defines
    // must therefore come back out of `list` with words after its name.
    let dir = Dir::new();
    chore(&dir, &["init"]).ok();
    let names = chore(&dir, &["list", "--names"]).ok().stdout;
    for line in names.lines() {
        let (name, description) = line.split_once('\t').expect("name<TAB>description");
        assert!(!description.is_empty(), "`{name}` has no description");
    }
    // And the tasks are real: one of them calls the other two.
    let run = chore(&dir, &["ci"]).ok();
    assert!(
        run.stdout.contains("nothing to build yet"),
        "{}",
        run.stdout
    );
}

#[test]
fn init_refuses_to_overwrite_an_existing_chorefile() {
    // Hand-written work, and there is no undo, so the refusal is the feature.
    let dir = Dir::new();
    let mine = "task keep {\n    echo mine\n}\n";
    dir.chorefile(mine);
    let run = chore(&dir, &["init"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("already exists"), "{}", run.stderr);
    assert_eq!(dir.read("chorefile"), mine, "init overwrote the file");
}

#[test]
fn init_cannot_be_shadowed_by_a_task() {
    // Reserved like `list` and `completions`: the subcommand wins wherever it
    // is typed, and `check` is where the author of the task is told why.
    let dir = Dir::new();
    dir.chorefile("task init {\n    echo shadowed\n}\n");
    let run = chore(&dir, &["init"]);
    assert!(!run.stdout.contains("shadowed"), "{}", run.stdout);
    // A chorefile is already there, so the subcommand it ran was `init`
    // declining to overwrite it.
    assert_eq!(run.code, 2, "{}", run.stderr);
    let check = chore(&dir, &["check"]);
    assert_eq!(check.code, 1, "{}", check.stdout);
    assert!(check.stdout.contains("init"), "{}", check.stdout);
}

// ---------------------------------------------------------------------------
// spawn
// ---------------------------------------------------------------------------
//
// The one command that outlives the run. The child here is `chore` itself,
// which is the only program every platform running these tests is guaranteed
// to have — and it gives the test a child that takes a known, visible amount
// of time to finish its work.

#[test]
fn a_spawned_process_outlives_the_run_and_keeps_writing() {
    let bin = env!("CARGO_BIN_EXE_chore").replace('\\', "/");
    let dir = Dir::new();
    dir.chorefile(&format!(
        "\
# Slow enough that the parent is certainly gone before it finishes.
task slow {{
    sleep 2
    write done.txt finished
}}

task dev {{
    spawn \"{bin}\" slow > slow.log
    echo carried on
}}
"
    ));

    let run = chore(&dir, &["dev"]).ok();
    // The run went straight on to its next statement, and said what it left
    // running — on stderr, where diagnostics go.
    assert_eq!(run.printed("carried on"), 1, "{}", run.stdout);
    assert!(run.stderr.contains("spawned"), "{}", run.stderr);
    assert!(run.stderr.contains("pid"), "{}", run.stderr);
    // Nothing waited: the child is still sleeping now that chore has exited.
    assert!(!dir.exists("done.txt"), "chore waited for the child");

    // ...and it finishes anyway, orphaned, with its output in the file the
    // `>` named.
    assert!(
        wait_for(&dir, "done.txt"),
        "the spawned child never finished"
    );
    assert_eq!(dir.read("done.txt"), "finished\n");
    let log = dir.read("slow.log");
    assert!(
        log.contains("sleep 2"),
        "the redirect caught nothing: {log}"
    );
}

#[test]
fn spawn_of_a_task_is_refused_before_anything_runs() {
    let dir = Dir::new();
    dir.chorefile("task dev {\n    spawn slow\n}\n\ntask slow {\n    write ran.txt yes\n}\n");
    let run = chore(&dir, &["dev"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stderr.contains("is a task"), "{}", run.stderr);
    assert!(!dir.exists("ran.txt"));
    // And `check` says so without running anything at all.
    let check = chore(&dir, &["check"]);
    assert_eq!(check.code, 1, "{}", check.stdout);
    assert!(check.stdout.contains("spawn"), "{}", check.stdout);
}

#[test]
fn dry_spawns_nothing_and_writes_no_log() {
    let bin = env!("CARGO_BIN_EXE_chore").replace('\\', "/");
    let dir = Dir::new();
    dir.chorefile(&format!(
        "task slow {{\n    write done.txt finished\n}}\n\ntask dev {{\n    spawn \"{bin}\" slow > slow.log\n}}\n"
    ));
    let run = chore(&dir, &["dev", "--dry"]).ok();
    assert!(run.stdout.contains("$ spawn"), "{}", run.stdout);
    assert!(!dir.exists("slow.log"), "a preview truncated the log");
    assert!(!dir.exists("done.txt"));
}

/// Poll for a file a detached child is about to write. A fixed sleep would be
/// either slower than it needs to be or flaky on a loaded machine.
fn wait_for(dir: &Dir, name: &str) -> bool {
    for _ in 0..200 {
        if dir.exists(name) {
            // Created before it is written: give the child its moment.
            std::thread::sleep(std::time::Duration::from_millis(50));
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}
