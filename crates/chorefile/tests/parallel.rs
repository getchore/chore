//! `parallel`: what concurrency must not change.
//!
//! These run whole chorefiles through the real builtins, because the
//! interesting claims are about wall-clock time, about files two threads
//! write, and about the order the blocks come out in. A hand-built AST and a
//! table of fake builtins could not show any of them.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chorefile::interp::{Interpreter, Mode, Repeat};
use chorefile::parse;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A temp directory that cleans itself up, so a failing assertion does not
/// leave litter behind. It doubles as `$ROOT`.
struct Dir(PathBuf);

impl Dir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "chorefile-parallel-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn read(&self, rel: &str) -> Option<String> {
        fs::read_to_string(self.0.join(rel)).ok()
    }

    fn exists(&self, rel: &str) -> bool {
        self.0.join(rel).exists()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A writer the test can read back. Shared with an `Arc` rather than an `Rc`
/// because a parallel child writes from another thread.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Log {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What one run of a task produced.
struct Ran {
    /// The exit code, or the message of the error that ended the run.
    code: Result<i32, String>,
    out: String,
    err: String,
    elapsed: Duration,
}

impl Ran {
    fn failed(&self) -> bool {
        !matches!(self.code, Ok(0))
    }
}

fn run(dir: &Dir, source: &str, task: &str) -> Ran {
    run_in(dir, source, task, Mode::Run)
}

fn run_in(dir: &Dir, source: &str, task: &str, mode: Mode) -> Ran {
    let file = parse::parse(source, Path::new("chorefile")).expect("parses");
    let (out, err) = (Log::default(), Log::default());
    let mut interp = Interpreter::new(&file, dir.path(), mode, Repeat::Once)
        .with_output(Box::new(out.clone()))
        .with_error_output(Box::new(err.clone()));
    let started = Instant::now();
    let code = interp.run_task(task, &[]).map_err(|e| e.to_string());
    Ran {
        code,
        out: out.text(),
        err: err.text(),
        elapsed: started.elapsed(),
    }
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

/// How long a run takes that sleeps once for `seconds`, on this machine.
///
/// Every timing assertion here is really "one sleep, not two", and a fixed
/// millisecond budget answers that only on a fast machine: a windows-11-arm
/// runner spends about 0.75s on process spawn and sleep granularity before
/// any sleeping happens, which is more than the gap being measured. Measuring
/// the same shape first turns the bound into a statement about the sleeps
/// rather than about the hardware.
fn baseline(tag: &str, seconds: &str) -> Duration {
    let dir = Dir::new(&format!("{tag}-baseline"));
    let source = format!("task once {{\n    sleep {seconds}\n}}\n");
    let ran = run(&dir, &source, "once");
    assert_eq!(ran.code, Ok(0), "baseline run failed: {}", ran.err);
    ran.elapsed
}

#[test]
fn two_tasks_really_run_at_the_same_time() {
    let dir = Dir::new("concurrent");
    let ran = run(
        &dir,
        "\
task ci {
    parallel a b
}
task a {
    sleep 0.6
}
task b {
    sleep 0.6
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    // Sequentially this is two 0.6s sleeps; overlapped it is one. The bound is
    // one sleep of headroom over a run that did exactly one, so it holds on a
    // slow machine and still fails if the tasks ran one after the other.
    let base = baseline("concurrent", "0.6");
    assert!(
        ran.elapsed < base + Duration::from_millis(600),
        "took {:?} against a {:?} baseline, so the tasks did not overlap",
        ran.elapsed,
        base
    );
}

#[test]
fn a_shared_dependency_runs_once_across_siblings() {
    // The promise the whole design exists for: `deps` is called by both
    // siblings, at the same instant, and runs exactly once between them.
    // `deps` sleeps first so both callers are inside it before either
    // finishes, which is the race a per-thread record would lose.
    let dir = Dir::new("shared-dep");
    let ran = run(
        &dir,
        "\
task ci {
    parallel a b
}
task a {
    deps
    echo a >> log.txt
}
task b {
    deps
    echo b >> log.txt
}
task deps {
    sleep 0.3
    echo dep >> log.txt
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    let log = dir.read("log.txt").expect("log");
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines.iter().filter(|l| **l == "dep").count(),
        1,
        "`deps` ran more than once: {log:?}"
    );
    assert_eq!(lines.iter().filter(|l| **l == "a").count(), 1, "{log:?}");
    assert_eq!(lines.iter().filter(|l| **l == "b").count(), 1, "{log:?}");
    // The second sibling waited for the first rather than running its own
    // copy: one 0.3s sleep between them, not two. Compared against a run that
    // slept once, so the bound measures the sleeps and not the machine.
    let base = baseline("shared-dep", "0.3");
    assert!(
        ran.elapsed < base + Duration::from_millis(300),
        "took {:?} against a {:?} baseline, so `deps` was slept through twice",
        ran.elapsed,
        base
    );
}

#[test]
fn a_shared_dependencys_value_is_replayed_not_rerun() {
    let dir = Dir::new("shared-value");
    let ran = run(
        &dir,
        "\
task ci {
    parallel a b
}
task a {
    got=$(id)
    echo a-$got >> log.txt
}
task b {
    got=$(id)
    echo b-$got >> log.txt
}
task id {
    sleep 0.2
    echo one >> ran.txt
    echo x1y
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    assert_eq!(dir.read("ran.txt").as_deref(), Some("one\n"));
    let log = dir.read("log.txt").expect("log");
    assert!(log.contains("a-x1y"), "{log:?}");
    assert!(log.contains("b-x1y"), "{log:?}");
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

#[test]
fn output_comes_out_in_blocks_in_the_order_the_tasks_were_named() {
    let dir = Dir::new("blocks");
    // The sleeps guarantee the two tasks are writing at the same time, so
    // anything but per-task buffering would interleave them.
    let ran = run(
        &dir,
        "\
task ci {
    parallel a b
}
task a {
    echo a1
    sleep 0.2
    echo a2
}
task b {
    echo b1
    sleep 0.2
    echo b2
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    assert_eq!(
        ran.out,
        "\
$ parallel a b
$ echo a1
a1
$ sleep 0.2
$ echo a2
a2
$ echo b1
b1
$ sleep 0.2
$ echo b2
b2
"
    );
}

// ---------------------------------------------------------------------------
// Failure
// ---------------------------------------------------------------------------

#[test]
fn one_failure_fails_the_call_and_the_others_still_ran() {
    let dir = Dir::new("failure");
    let ran = run(
        &dir,
        "\
task ci {
    parallel bad slow other
}
task bad {
    fail nope
}
task slow {
    sleep 0.2
    write slow.txt done
}
task other {
    write other.txt done
}
",
        "ci",
    );
    assert!(ran.failed(), "the call should have failed");
    // Every failure is reported, and the siblings that had work to do did it.
    assert!(
        ran.err.contains("parallel: task `bad` failed: nope"),
        "{}",
        ran.err
    );
    assert!(dir.exists("slow.txt"), "the slow sibling was cut short");
    assert!(dir.exists("other.txt"));
}

#[test]
fn every_failure_is_reported_not_just_the_first() {
    let dir = Dir::new("all-failures");
    let ran = run(
        &dir,
        "\
task ci {
    parallel one two
}
task one {
    fail first
}
task two {
    sleep 0.1
    fail second
}
",
        "ci",
    );
    assert!(ran.failed());
    assert!(ran.err.contains("task `one` failed: first"), "{}", ran.err);
    assert!(ran.err.contains("task `two` failed: second"), "{}", ran.err);
}

#[test]
fn fail_fast_stops_the_siblings_at_their_next_statement() {
    let dir = Dir::new("fail-fast");
    let ran = run(
        &dir,
        "\
task ci {
    parallel --fail-fast bad slow
}
task bad {
    fail nope
}
task slow {
    sleep 0.3
    write slow.txt done
}
",
        "ci",
    );
    assert!(ran.failed());
    // The `sleep` already running was left to finish; the statement after it
    // never started.
    assert!(
        !dir.exists("slow.txt"),
        "--fail-fast did not stop the sibling"
    );
    assert!(
        ran.err.contains("parallel: task `slow` stopped early"),
        "{}",
        ran.err
    );
}

#[test]
fn a_task_stopped_by_fail_fast_is_not_recorded_as_having_run() {
    let dir = Dir::new("fail-fast-memo");
    let ran = run(
        &dir,
        "\
task ci {
    try parallel --fail-fast bad slow
    slow
}
task bad {
    fail nope
}
task slow {
    sleep 0.3
    write slow.txt done
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    // Run-once must not skip work that was abandoned half way.
    assert!(dir.exists("slow.txt"));
}

#[test]
fn exit_in_a_sibling_ends_the_whole_run() {
    let dir = Dir::new("exit");
    let ran = run(
        &dir,
        "\
task ci {
    parallel quit other
    write after.txt done
}
task quit {
    exit 3
}
task other {
    write other.txt done
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(3));
    // The siblings were already running and are still waited for; what `exit`
    // stops is everything after the call.
    assert!(dir.exists("other.txt"));
    assert!(!dir.exists("after.txt"));
}

#[test]
fn an_unknown_task_is_an_error_before_anything_runs() {
    let dir = Dir::new("unknown");
    let ran = run(
        &dir,
        "\
task ci {
    parallel a nope
}
task a {
    write a.txt done
}
",
        "ci",
    );
    assert!(ran.code.is_err());
    assert!(!dir.exists("a.txt"), "half the call ran anyway");
}

// ---------------------------------------------------------------------------
// --dry
// ---------------------------------------------------------------------------

#[test]
fn dry_previews_the_tasks_and_runs_nothing() {
    let dir = Dir::new("dry");
    let ran = run_in(
        &dir,
        "\
task ci {
    parallel a b
}
task a {
    write a.txt done
}
task b {
    write b.txt done
}
",
        "ci",
        Mode::Dry,
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    assert_eq!(
        ran.out,
        "\
$ parallel a b
$ write a.txt done
$ write b.txt done
"
    );
    assert!(!dir.exists("a.txt"));
    assert!(!dir.exists("b.txt"));
}

// ---------------------------------------------------------------------------
// The frame a sibling starts in
// ---------------------------------------------------------------------------

#[test]
fn a_sibling_starts_where_the_caller_stands_and_its_cd_dies_with_it() {
    let dir = Dir::new("cwd");
    fs::create_dir_all(dir.path().join("sub")).expect("sub");
    let ran = run(
        &dir,
        "\
task ci {
    parallel a b
    echo $CWD/here.txt > where.txt
}
task a {
    cd sub
    write a.txt done
}
task b {
    write b.txt done
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    // `a` moved itself, not its caller and not its sibling.
    assert!(dir.exists("sub/a.txt"));
    assert!(dir.exists("b.txt"));
    let where_txt = dir.read("where.txt").expect("where");
    assert!(!where_txt.contains("sub"), "{where_txt:?}");
}

// ---------------------------------------------------------------------------
// Nesting, and the cycles it makes possible
// ---------------------------------------------------------------------------

#[test]
fn a_parallel_inside_a_parallel_still_runs_a_shared_task_once() {
    let dir = Dir::new("nested");
    let ran = run(
        &dir,
        "\
task ci {
    parallel outer c
}
task outer {
    parallel a b
}
task a {
    deps
}
task b {
    deps
}
task c {
    deps
}
task deps {
    sleep 0.2
    echo dep >> log.txt
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    assert_eq!(dir.read("log.txt").as_deref(), Some("dep\n"));
}

#[test]
fn two_siblings_that_call_each_other_do_not_deadlock() {
    // A cycle no single-threaded run could hang on must not hang here
    // either: waiting for a task whose owner is waiting for us is answered
    // the way a call already on the stack is answered.
    let dir = Dir::new("cycle");
    let ran = run(
        &dir,
        "\
task ci {
    parallel a b
}
task a {
    sleep 0.1
    b
    echo a >> log.txt
}
task b {
    sleep 0.1
    a
    echo b >> log.txt
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    let log = dir.read("log.txt").expect("log");
    assert!(log.contains('a') && log.contains('b'), "{log:?}");
}

// ---------------------------------------------------------------------------
// env
// ---------------------------------------------------------------------------

#[test]
fn siblings_cannot_see_each_others_env_sets() {
    // Each sibling has its own interpreter and its own copy of the overlay,
    // which is the whole reason `env` never writes the process environment:
    // two threads setting one name there would race, and whichever won would
    // decide what the other's `download` was configured with.
    let dir = Dir::new("env-siblings");
    let ran = run(
        &dir,
        "\
task ci {
    parallel a b
    if env SHARED {
        write leaked.txt yes
    }
}
task a {
    env SHARED a
    sleep 0.1
    write a.txt $(env SHARED)
}
task b {
    env SHARED b
    sleep 0.1
    write b.txt $(env SHARED)
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    assert_eq!(dir.read("a.txt").as_deref(), Some("a\n"));
    assert_eq!(dir.read("b.txt").as_deref(), Some("b\n"));
    // And nothing a sibling set outlives the `parallel`.
    assert!(!dir.exists("leaked.txt"));
}

#[test]
fn a_sibling_starts_with_the_environment_the_parallel_was_called_with() {
    let dir = Dir::new("env-inherited");
    let ran = run(
        &dir,
        "\
task ci {
    env FROM_PARENT yes
    parallel a
}
task a {
    write a.txt $(env FROM_PARENT)
}
",
        "ci",
    );
    assert_eq!(ran.code, Ok(0), "{}", ran.err);
    assert_eq!(dir.read("a.txt").as_deref(), Some("yes\n"));
}
