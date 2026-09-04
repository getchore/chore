//! End-to-end tests: build a chorefile in a temp directory, run the real
//! binary against it, and look at stdout, stderr and the exit code.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A temp directory that removes itself, so a failing test does not leave a
/// tree behind. Hand-rolled: the binary takes no dependencies, and one
/// `mkdir`/`rm -r` pair does not justify a dev-dependency either.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "chore-cli-{}-{}",
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

const SAMPLE: &str = r#"
# build the project
task build {
    echo building
}

task greet name {
    echo hello $1
}

# say every argument back
task echoes {
    echo count $#
    echo args $@
}
"#;

#[test]
fn runs_a_task() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["build"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("building"), "{}", run.stdout);
}

#[test]
fn passes_arguments_through() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["greet", "world"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("hello world"), "{}", run.stdout);
}

#[test]
fn flag_shaped_arguments_reach_the_task() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["echoes", "--nocapture", "-q"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("count 2"), "{}", run.stdout);
    assert!(run.stdout.contains("args --nocapture -q"), "{}", run.stdout);
}

#[test]
fn our_own_flags_are_not_passed_to_the_task() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["echoes", "--dry", "one", "--force"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("count 1"), "{}", run.stdout);
}

#[test]
fn double_dash_hands_over_our_flags() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["echoes", "--", "--dry"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("args --dry"), "{}", run.stdout);
}

#[test]
fn dry_skips_effects() {
    let dir = Dir::new();
    dir.chorefile("task touch {\n    write out.txt hello\n}\n");

    let run = chore(&dir, &["touch", "--dry"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        !dir.path().join("out.txt").exists(),
        "--dry wrote the file: {}",
        run.stdout
    );

    let run = chore(&dir, &["touch"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(dir.path().join("out.txt").exists());
}

#[test]
fn force_disables_run_once() {
    let dir = Dir::new();
    dir.chorefile("task note {\n    echo once\n}\n\ntask both {\n    note\n    note\n}\n");

    let plain = chore(&dir, &["both"]);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    assert_eq!(printed(&plain.stdout, "once"), 1, "{}", plain.stdout);

    let forced = chore(&dir, &["both", "--force"]);
    assert_eq!(forced.code, 0, "{}", forced.stderr);
    assert_eq!(printed(&forced.stdout, "once"), 2, "{}", forced.stdout);
}

#[test]
fn list_shows_names_and_docs_in_source_order() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    // The listing is one provenance line and then the tasks, so the rows are
    // parsed the way they always were, one line further down.
    let names: Vec<&str> = task_lines(&run.stdout)
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(names, ["build", "greet", "echoes"], "{}", run.stdout);
    assert!(run.stdout.contains("build the project"), "{}", run.stdout);
}

/// A task's description is the first line of the comment block above it, so a
/// header separated by a blank line describes the file and a multi-line block
/// leads with the summary rather than trailing off into its own caveats.
#[test]
fn list_shows_the_first_line_of_the_comment_block_above_each_task() {
    let dir = Dir::new();
    dir.chorefile(
        "# Tasks for this project.\n\
         # Not a description of anything below.\n\
         \n\
         # Run the app under the debugger.\n\
         # In CI, where CI=true, skips that styling;\n\
         # else falls back to ad-hoc.\n\
         task run {\n    echo running\n}\n\
         \n\
         # Build it.\n\
         task build {\n    echo building\n}\n",
    );
    let run = chore(&dir, &["list", "--names"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "run\tRun the app under the debugger.\nbuild\tBuild it.\n"
    );

    let run = chore(&dir, &["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        !run.stdout.contains("ad-hoc") && !run.stdout.contains("Not a description"),
        "the rest of the block leaked into the listing:\n{}",
        run.stdout
    );
}

/// A block separated from the task by a blank line is a comment about the file,
/// not a description, and a task takes only the block that touches it.
#[test]
fn list_leaves_a_detached_comment_block_out() {
    let dir = Dir::new();
    dir.chorefile(
        "# File header\n\
         # second line\n\
         \n\
         task build {\n    echo building\n}\n\
         # Test it.\n\
         task test {\n    echo testing\n}\n",
    );
    let run = chore(&dir, &["list", "--names"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "build\t\ntest\tTest it.\n");
}

/// The task rows of a `chore list`: everything after the line that says which
/// chorefile answered.
fn task_lines(stdout: &str) -> impl Iterator<Item = &str> {
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or_default();
    assert!(
        first.starts_with("using ") && first.contains("$ROOT = "),
        "no provenance line above the list:\n{stdout}"
    );
    lines
}

/// `chore list` says which chorefile it is listing and where `$ROOT` is,
/// because the nearest-chorefile search means neither is obvious from where
/// the command was typed.
#[test]
fn list_names_the_chorefile_in_the_current_directory() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let first = run.stdout.lines().next().unwrap_or_default();
    // Relative to the working directory, so the file that is right here is
    // spelled the way a person would spell it.
    assert!(
        first.starts_with("using chorefile, $ROOT = "),
        "{}",
        run.stdout
    );
    // `$ROOT` is absolute: `.` would look identical from a subproject.
    let root = first.rsplit("$ROOT = ").next().expect("a root");
    assert!(root.ends_with(&dir_name(&dir)), "{}", run.stdout);
}

/// Found by walking up: the relative spelling is the part that says "the file
/// governing this listing is not the one you are standing in".
#[test]
fn list_from_a_subdirectory_shows_the_chorefile_it_walked_up_to() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let nested = dir.path().join("a/b");
    std::fs::create_dir_all(&nested).expect("nested");
    let run = chore_in(&nested, &["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let first = run.stdout.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("using ../../chorefile, $ROOT = "),
        "{}",
        run.stdout
    );
    // And `$ROOT` is the top-level directory, not the one we stood in.
    let root = first.rsplit("$ROOT = ").next().expect("a root");
    assert!(root.ends_with(&dir_name(&dir)), "{}", run.stdout);
    // The tasks below it are still the tasks, parsed as they always were.
    let names: Vec<&str> = task_lines(&run.stdout)
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(names, ["build", "greet", "echoes"], "{}", run.stdout);
}

/// A subdirectory with a chorefile of its own is a different project with a
/// different `$ROOT`, and the two listings are told apart by the `$ROOT` — the
/// relative path is `chorefile` in both.
#[test]
fn list_inside_a_subproject_reports_that_subproject_as_root() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let handoff = dir.path().join("handoff");
    std::fs::create_dir_all(&handoff).expect("handoff");
    std::fs::write(handoff.join("chorefile"), "task ship {\n    echo ship\n}\n").expect("write");

    let above = chore(&dir, &["list"]);
    let inside = chore_in(&handoff, &["list"]);
    assert_eq!(inside.code, 0, "{}", inside.stderr);
    let line = |out: &str| out.lines().next().unwrap_or_default().to_string();
    let (above, inside) = (line(&above.stdout), line(&inside.stdout));
    assert!(above.starts_with("using chorefile, $ROOT = "), "{above}");
    assert!(inside.starts_with("using chorefile, $ROOT = "), "{inside}");
    // Same spelling of the file, different worlds, and the line says so.
    assert_ne!(above, inside);
    assert!(inside.ends_with("/handoff"), "{inside}");
}

/// The unique tail of a test's temp directory, for asserting on a path
/// without spelling out this host's temp root.
fn dir_name(dir: &Dir) -> String {
    dir.path()
        .file_name()
        .expect("temp dir name")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn list_json_has_the_documented_fields() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["list", "--json"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    // A bare array. The text listing names the chorefile and `$ROOT`; the JSON
    // deliberately does not, because carrying them would mean an object and
    // that breaks `jq '.[]'` for everyone. See `list_json_is_an_array_of_tasks`.
    assert!(run.stdout.trim_start().starts_with('['), "{}", run.stdout);
    // The whole record, except `file`, whose spelling is the host's business.
    assert!(
        run.stdout.contains(
            r#""name": "greet", "description": null, "params": ["name"], "namespace": null, "file": ""#
        ),
        "{}",
        run.stdout
    );
    // `file` is checked against the contract rather than against this
    // platform's spelling of a path: chorefiles are written with `/` and
    // reported with `/` everywhere, so a Windows `\\` here, or the `\\?\\`
    // prefix `canonicalize` adds there, would be the bug. The directory name
    // is unique per test, so ending with it is an exact-enough match.
    let dir_name = dir
        .path()
        .file_name()
        .expect("temp dir name")
        .to_string_lossy()
        .into_owned();
    let file = run
        .stdout
        .split(r#""file": ""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a file field");
    assert!(
        file.ends_with(&format!("{dir_name}/chorefile")),
        "file field was {file:?}, expected it to end with {dir_name}/chorefile"
    );
    assert!(
        !file.contains('\\'),
        "file field was {file:?}, expected `/` separators"
    );
    assert!(
        run.stdout
            .contains(r#""name": "build", "description": "build the project""#),
        "{}",
        run.stdout
    );
    // Without an `include` there is one file and no namespace, and every task
    // still says which file it came from.
    assert_eq!(
        run.stdout.matches(r#""namespace": null"#).count(),
        3,
        "{}",
        run.stdout
    );
}

/// `list --json` is an array of tasks, and stays one.
///
/// The text listing gained a line naming the chorefile and `$ROOT`; the JSON
/// deliberately did not, because an array has nowhere to put a fact about the
/// whole listing and turning it into an object breaks every consumer doing
/// `jq '.[]'`. That is a major-release change with a note, not a quiet one.
/// This test exists so the shape cannot drift by accident.
#[test]
fn list_json_is_an_array_of_tasks() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let nested = dir.path().join("a/b");
    std::fs::create_dir_all(&nested).expect("nested");
    let run = chore_in(&nested, &["list", "--json"]);
    assert_eq!(run.code, 0, "{}", run.stderr);

    let text = run.stdout.trim();
    assert!(text.starts_with('['), "{text}");
    assert!(text.ends_with(']'), "{text}");
    // The provenance line belongs to the text listing only; --json is for
    // tools, and its first byte is the array.
    assert!(!run.stdout.contains("using "), "{}", run.stdout);
    // A task still names the file it was written in, which is how a tool
    // learns where things live until the array becomes an object.
    let name = dir_name(&dir);
    assert!(
        run.stdout.contains(&format!("{name}/chorefile")),
        "{}",
        run.stdout
    );
}

#[test]
fn no_arguments_leads_with_the_tasks_and_points_at_help() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    // Which chorefile answered, then the tasks it holds.
    assert!(run.stdout.starts_with("using chorefile,"), "{}", run.stdout);
    assert!(run.stdout.contains("\nAvailable tasks:"), "{}", run.stdout);
    assert!(run.stdout.contains("build"), "{}", run.stdout);
    // The grammar is a page long and nobody asked for it, so the answer is a
    // pointer to where it lives rather than the page itself.
    assert!(!run.stdout.contains("usage: chore"), "{}", run.stdout);
    assert!(run.stdout.contains("chore help"), "{}", run.stdout);
}

#[test]
fn no_arguments_with_no_chorefile_falls_back_to_usage() {
    let dir = Dir::new();
    let run = chore(&dir, &[]);
    // Nothing to list, so the usage block is all there is to say, and the
    // missing chorefile is still the error it has always been.
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("usage: chore"), "{}", run.stdout);
    assert!(run.stderr.contains("chorefile"), "{}", run.stderr);
}

#[test]
fn help_still_carries_the_usage_block() {
    let dir = Dir::new();
    let run = chore(&dir, &["help"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("usage: chore"), "{}", run.stdout);
    assert!(run.stdout.contains("--force"), "{}", run.stdout);
}

/// Every subcommand a person might pipe somewhere is captured here rather
/// than at a terminal, so this asserts what a pipe gets: never an escape.
/// The machine-readable formats are the ones that would actually break.
#[test]
fn piped_output_carries_no_escape_sequences() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    for args in [
        &["list"][..],
        &["list", "--json"],
        &["list", "--names"],
        &["spec"],
        &["help"],
        &[],
    ] {
        let run = chore(&dir, args);
        assert!(
            !run.stdout.contains('\x1b'),
            "chore {args:?} coloured a pipe:\n{}",
            run.stdout
        );
    }
}

#[test]
fn subcommands_shadow_tasks_of_the_same_name() {
    let dir = Dir::new();
    dir.chorefile("task list {\n    echo SHADOWED\n}\n");
    let run = chore(&dir, &["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(!run.stdout.contains("SHADOWED"), "{}", run.stdout);
    assert!(run.stdout.contains("list"), "{}", run.stdout);
}

#[test]
fn finds_the_chorefile_from_a_subdirectory() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let nested = dir.path().join("a/b/c");
    std::fs::create_dir_all(&nested).expect("nested");
    let run = chore_in(&nested, &["build"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("building"), "{}", run.stdout);
}

#[test]
fn missing_chorefile_is_a_usage_error() {
    let dir = Dir::new();
    let run = chore(&dir, &["build"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(run.stderr.starts_with("chore: "), "{}", run.stderr);
    assert!(run.stderr.contains("chorefile"), "{}", run.stderr);
}

#[test]
fn unknown_task_is_a_usage_error() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["nope"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(run.stderr.contains("nope"), "{}", run.stderr);
}

#[test]
fn unknown_option_before_a_task_is_a_usage_error() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["--wat", "build"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
}

#[test]
fn a_failing_task_exits_one() {
    let dir = Dir::new();
    dir.chorefile("task boom {\n    fail nope\n}\n");
    let run = chore(&dir, &["boom"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(run.stderr.starts_with("chore: "), "{}", run.stderr);
}

#[test]
fn an_explicit_exit_code_survives() {
    let dir = Dir::new();
    dir.chorefile("task quit {\n    exit 3\n}\n");
    let run = chore(&dir, &["quit"]);
    assert_eq!(run.code, 3, "{}{}", run.stdout, run.stderr);
}

#[test]
fn a_syntax_error_is_reported_with_its_file() {
    let dir = Dir::new();
    dir.chorefile("task broken {\n");
    let run = chore(&dir, &["broken"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(run.stderr.contains("chorefile"), "{}", run.stderr);
}

// `check`, `help` and `spec` render data the library owns, so these assert the
// wiring and the exit codes rather than the wording of the reference.

#[test]
fn check_accepts_a_clean_file() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["check"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
}

#[test]
fn check_reports_findings_and_exits_one() {
    let dir = Dir::new();
    dir.chorefile("task fetch {\n    curl https://example.com\n}\n");
    let run = chore(&dir, &["check"]);
    assert_eq!(run.code, 1, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("chorefile:2:"), "{}", run.stdout);
}

#[test]
fn check_needs_a_chorefile() {
    let dir = Dir::new();
    let run = chore(&dir, &["check"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
}

// `check` is the one subcommand a chorefile can take back, so the word and the
// flag mean different things depending on the file underneath them.

#[test]
fn a_task_named_check_wins_the_word_but_not_the_flag() {
    let dir = Dir::new();
    dir.chorefile("task check {\n    echo ran the task\n}\n");

    let run = chore(&dir, &["check"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("ran the task"), "{}", run.stdout);

    // The flag is what a script writes: it lints whatever the chorefile says,
    // and a clean file lints clean.
    let run = chore(&dir, &["--check"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("ran the task"), "{}", run.stdout);
}

#[test]
fn without_a_task_named_check_both_spellings_lint() {
    let dir = Dir::new();
    dir.chorefile("task fetch {\n    curl https://example.com\n}\n");
    for args in [&["check"][..], &["--check"][..]] {
        let run = chore(&dir, args);
        assert_eq!(run.code, 1, "{args:?}: {}{}", run.stdout, run.stderr);
        assert!(run.stdout.contains("chorefile:2:"), "{}", run.stdout);
    }
}

/// The reserved-name error would be the change undone: the point of freeing
/// the name is that a chorefile may use it.
#[test]
fn linting_a_file_with_a_check_task_says_nothing_about_reserved_names() {
    let dir = Dir::new();
    dir.chorefile("task check {\n    echo hi\n}\n");
    let run = chore(&dir, &["--check"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("subcommand"), "{}", run.stdout);
}

/// Nothing runs, so nothing the other flags do has anywhere to land.
#[test]
fn the_check_flag_stands_alone() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    for args in [&["--check", "--dry"][..], &["--dry", "--check"][..]] {
        let run = chore(&dir, args);
        assert_eq!(run.code, 2, "{args:?}: {}{}", run.stdout, run.stderr);
    }
}

#[test]
fn help_lists_the_builtins() {
    let dir = Dir::new();
    let run = chore(&dir, &["help"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("download"), "{}", run.stdout);
}

#[test]
fn help_explains_one_builtin() {
    let dir = Dir::new();
    let run = chore(&dir, &["help", "download"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.contains("download"), "{}", run.stdout);
}

#[test]
fn help_rejects_an_unknown_topic() {
    let dir = Dir::new();
    let run = chore(&dir, &["help", "nosuchbuiltin"]);
    assert_eq!(run.code, 2, "{}{}", run.stdout, run.stderr);
    assert!(run.stderr.contains("help topic"), "{}", run.stderr);
}

/// `include` is where the naming rule is written, and it is not a builtin,
/// so `chore help include` has to answer or the rule is unreachable from the
/// binary. The same for `task`, and for `files` itself.
#[test]
fn help_answers_for_a_statement_form_and_for_files() {
    let dir = Dir::new();
    let run = chore(&dir, &["help", "include"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(
        run.stdout.starts_with("include path [as name]"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains(".chore"), "{}", run.stdout);

    let run = chore(&dir, &["help", "task"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);

    let run = chore(&dir, &["help", "files"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    for name in [
        "chorefile",
        "rust.chore",
        "release.chore",
        "docker.chore",
        ".chore/",
    ] {
        assert!(run.stdout.contains(name), "{name} missing:\n{}", run.stdout);
    }
}

/// The usage block is what an agent reads first, so the file names are in it:
/// which one is discovered, and what a fragment is called.
#[test]
fn usage_names_the_files() {
    let dir = Dir::new();
    let run = chore(&dir, &["--help"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    let usage = run
        .stdout
        .split("chorefile 1")
        .next()
        .unwrap_or(&run.stdout);
    assert!(usage.contains("\nfiles\n"), "{usage}");
    assert!(usage.contains("--file <path>"), "{usage}");
    for name in [
        "rust.chore",
        "release.chore",
        "docker.chore",
        "<name>.chore",
    ] {
        assert!(usage.contains(name), "{name} missing:\n{usage}");
    }
    assert!(!run.stdout.contains("tasks.chore"), "{}", run.stdout);
}

#[test]
fn spec_prints_json() {
    let dir = Dir::new();
    let run = chore(&dir, &["spec"]);
    assert_eq!(run.code, 0, "{}{}", run.stdout, run.stderr);
    assert!(run.stdout.trim_start().starts_with('{'), "{}", run.stdout);
}

/// How many times a line was printed. Counting substrings would also count
/// the `$ echo once` the interpreter echoes before running it.
fn printed(stdout: &str, line: &str) -> usize {
    stdout.lines().filter(|l| l.trim() == line).count()
}

#[test]
fn an_unmet_require_stops_a_run() {
    // A version no build will ever be, so this test outlives every release.
    let dir = Dir::new();
    dir.chorefile("require 99.0.0\ntask build {\n    echo building\n}\n");
    let run = chore(&dir, &["build"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    // The task did not run, and the globals above it were never evaluated.
    assert!(!run.stdout.contains("building"), "{}", run.stdout);
    assert!(
        run.stderr.contains("requires chore 99.0.0 or newer"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("install.sh"), "{}", run.stderr);
    assert!(run.stderr.contains("chorefile:1:1"), "{}", run.stderr);
}

#[test]
fn a_met_require_runs() {
    let dir = Dir::new();
    dir.chorefile("require 0.0.0\ntask build {\n    echo building\n}\n");
    let run = chore(&dir, &["build"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("building"), "{}", run.stdout);
}

#[test]
fn list_warns_about_an_unmet_require_and_still_lists() {
    // `list` answers "what is here", which an old binary can still answer.
    // The warning is on stderr, so stdout is exactly what it always was.
    let dir = Dir::new();
    dir.chorefile("require 99.0.0\n\n# build the project\ntask build {\n    echo hi\n}\n");
    let run = chore(&dir, &["list"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.contains("build"), "{}", run.stdout);
    assert!(!run.stdout.contains("99.0.0"), "{}", run.stdout);
    assert!(
        run.stderr.contains("requires chore 99.0.0"),
        "{}",
        run.stderr
    );

    // The machine-readable formats keep a clean stdout too.
    let run = chore(&dir, &["list", "--names"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "build\tbuild the project\n");
    assert!(run.stderr.contains("99.0.0"), "{}", run.stderr);
}

#[test]
fn check_reports_an_unmet_require() {
    let dir = Dir::new();
    dir.chorefile("require 99.0.0\ntask build {\n    echo hi\n}\n");
    let run = chore(&dir, &["check"]);
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.contains("chorefile:1:1"), "{}", run.stdout);
    assert!(
        run.stdout.contains("requires chore 99.0.0 or newer"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("help:"), "{}", run.stdout);
}

#[test]
fn help_and_spec_ignore_an_unmet_require() {
    // Neither reads a chorefile, so neither can be stopped by one.
    let dir = Dir::new();
    dir.chorefile("require 99.0.0\ntask build {\n    echo hi\n}\n");
    for args in [&["help"][..], &["spec"][..]] {
        let run = chore(&dir, args);
        assert_eq!(run.code, 0, "{args:?}: {}", run.stderr);
        assert!(run.stderr.is_empty(), "{args:?}: {}", run.stderr);
    }
}
