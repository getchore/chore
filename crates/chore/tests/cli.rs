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
    let names: Vec<&str> = run
        .stdout
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(names, ["build", "greet", "echoes"], "{}", run.stdout);
    assert!(run.stdout.contains("build the project"), "{}", run.stdout);
}

#[test]
fn list_json_has_the_documented_fields() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &["list", "--json"]);
    assert_eq!(run.code, 0, "{}", run.stderr);
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

#[test]
fn no_arguments_leads_with_the_tasks_and_points_at_help() {
    let dir = Dir::new();
    dir.chorefile(SAMPLE);
    let run = chore(&dir, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.starts_with("Available tasks:"), "{}", run.stdout);
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
