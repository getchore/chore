//! Interpreter tests.
//!
//! The AST is built by hand rather than parsed: the interpreter is the unit
//! under test, and the tests stay meaningful while the parser is in flight.
//! Commands resolve through a test builtin table, so nothing here depends on
//! what happens to be installed on `PATH`.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chorefile::ast::*;
use chorefile::error::{Error, Result, Span};
use chorefile::exec::{Builtin, Ctx, Output};
use chorefile::interp::{Interpreter, Mode, Repeat};

// ---------------------------------------------------------------------------
// AST helpers
// ---------------------------------------------------------------------------

fn sp() -> Span {
    Span::new(0, 0)
}

/// An unquoted word made of literal text.
fn lit(text: &str) -> Word {
    word(vec![part(PartKind::Literal(text.into()))], false)
}

/// A quoted word: always exactly one argument.
fn quoted(parts: Vec<WordPart>) -> Word {
    word(parts, true)
}

fn unquoted(parts: Vec<WordPart>) -> Word {
    word(parts, false)
}

fn word(parts: Vec<WordPart>, quoted: bool) -> Word {
    Word {
        parts,
        quoted,
        span: sp(),
    }
}

/// The interpreter never reads a span, so every hand-built node carries an
/// empty one.
fn part(kind: PartKind) -> WordPart {
    WordPart::new(kind, sp())
}

fn var(name: &str) -> WordPart {
    part(PartKind::Var(VarRef::Named(name.into())))
}

fn text(s: &str) -> WordPart {
    part(PartKind::Literal(s.into()))
}

fn cmd(name: &str, args: Vec<Word>) -> Chain {
    Chain::Single(Command {
        name: lit(name),
        force_path: false,
        args,
        redirects: Vec::new(),
        span: sp(),
    })
}

fn cmd_with(name: &str, args: Vec<Word>, redirects: Vec<Redirect>) -> Chain {
    Chain::Single(Command {
        name: lit(name),
        force_path: false,
        args,
        redirects,
        span: sp(),
    })
}

fn redirect(kind: RedirectKind, target: &str) -> Redirect {
    Redirect {
        kind,
        target: lit(target),
        span: sp(),
    }
}

fn run(chain: Chain) -> Stmt {
    Stmt::Command(chain)
}

fn assign(name: &str, value: Word) -> Stmt {
    Stmt::Assign(Assign {
        name: name.into(),
        value,
        span: sp(),
    })
}

fn task(name: &str, params: &[&str], body: Block) -> Task {
    Task {
        name: name.into(),
        params: params.iter().map(|p| p.to_string()).collect(),
        doc: None,
        body,
        span: sp(),
    }
}

fn file(tasks: Vec<Task>) -> File {
    File {
        includes: Vec::new(),
        globals: Vec::new(),
        tasks,
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Test builtins
// ---------------------------------------------------------------------------

thread_local! {
    /// What every `hit` recorded, in order. Thread-local because the test
    /// harness runs tests in parallel and each gets its own thread.
    static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn trace() -> Vec<String> {
    TRACE.with(|t| t.borrow().clone())
}

fn reset() {
    TRACE.with(|t| t.borrow_mut().clear());
}

fn table(name: &str) -> Option<Builtin> {
    Some(match name {
        "say" => say,
        "count" => count,
        "hit" => hit,
        "status" => status,
        "upper" => upper,
        "here" => here,
        "touch" => touch,
        "warn" => warn,
        "tty" => tty,
        "emit" => emit,
        "boom" => boom,
        "exists" => exists,
        "read" => read,
        // The real `fail`, close enough for the interpreter: it errors out.
        // The name matters — `--dry` softens every builtin failure but this
        // one.
        "fail" => boom,
        _ => return None,
    })
}

/// Prints its arguments, space separated. No effects, so it runs under `--dry`.
fn say(ctx: &mut Ctx<'_>) -> Result<Output> {
    writeln!(ctx.out, "{}", ctx.rest().join(" "))?;
    Ok(Output::ok())
}

/// Prints how many argv entries it received — the word-splitting probe.
fn count(ctx: &mut Ctx<'_>) -> Result<Output> {
    writeln!(ctx.out, "{}", ctx.rest().len())?;
    Ok(Output::ok())
}

/// `hit <tag>` records a tag, so what ran — and how often — is observable.
fn hit(ctx: &mut Ctx<'_>) -> Result<Output> {
    TRACE.with(|t| t.borrow_mut().push(ctx.rest().join(" ")));
    Ok(Output::ok())
}

/// `status <n>` exits with `n`.
fn status(ctx: &mut Ctx<'_>) -> Result<Output> {
    let code = ctx.rest().first().map_or(1, |c| c.parse().unwrap_or(1));
    Ok(Output::failed(code))
}

/// Uppercases piped input, for `|`.
fn upper(ctx: &mut Ctx<'_>) -> Result<Output> {
    let input = String::from_utf8_lossy(ctx.stdin.unwrap_or_default()).to_uppercase();
    write!(ctx.out, "{input}")?;
    Ok(Output::ok())
}

/// Prints the interpreter's current directory.
fn here(ctx: &mut Ctx<'_>) -> Result<Output> {
    writeln!(ctx.out, "{}", chorefile::vars::display(ctx.cwd))?;
    Ok(Output::ok())
}

/// The one builtin with an effect: it honours `dry`.
fn touch(ctx: &mut Ctx<'_>) -> Result<Output> {
    if !ctx.dry {
        let path = ctx.path(&ctx.rest()[0]);
        std::fs::write(path, b"")?;
    }
    Ok(Output::ok())
}

/// Writes to both streams, so a test can see which one a redirect caught.
fn warn(ctx: &mut Ctx<'_>) -> Result<Output> {
    writeln!(ctx.out, "to stdout")?;
    writeln!(ctx.err, "{}", ctx.rest().join(" "))?;
    Ok(Output::ok())
}

/// Returns its arguments in `Output::stdout` instead of writing to `ctx.out`
/// — the other convention a builtin may use, and the one that shows whether
/// bytes bound for a `>` file leak back into the caller.
fn emit(ctx: &mut Ctx<'_>) -> Result<Output> {
    Ok(Output {
        stdout: format!("{}\n", ctx.rest().join(" ")).into_bytes(),
        ..Output::ok()
    })
}

/// Fails hard, the way `read` does on a missing file: the message is the
/// diagnostic, and it never reaches `ctx.err`.
fn boom(ctx: &mut Ctx<'_>) -> Result<Output> {
    Err(Error::Run {
        message: ctx.rest().join(" "),
    })
}

/// `exists <path>` — the real one in miniature: a miss is an *answer*, a
/// nonzero exit, and never a failure. One of the three builtins that cannot
/// fail (`exists`, `which`, `env <NAME>`).
fn exists(ctx: &mut Ctx<'_>) -> Result<Output> {
    if ctx.path(&ctx.rest()[0]).exists() {
        Ok(Output::ok())
    } else {
        Ok(Output::failed(1))
    }
}

/// `read <path>` — the real one in miniature: a miss is a *failure*, like
/// every builtin outside those three.
fn read(ctx: &mut Ctx<'_>) -> Result<Output> {
    let path = ctx.path(&ctx.rest()[0]);
    let text = std::fs::read_to_string(&path).map_err(|_| Error::Run {
        message: format!("read: cannot read {}", chorefile::vars::display(&path)),
    })?;
    write!(ctx.out, "{text}")?;
    Ok(Output::ok())
}

/// Prints whether the builtin believes it is writing to a terminal.
fn tty(ctx: &mut Ctx<'_>) -> Result<Output> {
    writeln!(ctx.out, "{}", ctx.interactive)?;
    Ok(Output::ok())
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A writer the test can read back.
#[derive(Clone, Default)]
struct Log(Rc<RefCell<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Log {
    fn text(&self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}

struct Ran {
    result: Result<i32>,
    out: String,
    /// Everything the run's builtins reported as diagnostics.
    err: String,
}

impl Ran {
    fn ok(&self) -> &str {
        assert!(self.result.is_ok(), "run failed: {}", self.err_text());
        &self.out
    }

    fn err_text(&self) -> String {
        match &self.result {
            Err(e) => e.to_string(),
            Ok(code) => format!("(succeeded with code {code})"),
        }
    }

    /// Everything the run printed that was not an echo line.
    fn printed(&self) -> Vec<String> {
        self.ok()
            .lines()
            .filter(|l| !l.starts_with("$ "))
            .map(String::from)
            .collect()
    }

    fn echoed(&self) -> Vec<String> {
        self.ok()
            .lines()
            .filter(|l| l.starts_with("$ "))
            .map(String::from)
            .collect()
    }
}

fn exec(file: &File, name: &str, argv: &[&str], mode: Mode, repeat: Repeat, root: &Path) -> Ran {
    reset();
    let log = Log::default();
    let errlog = Log::default();
    let mut interp = Interpreter::new(file, root, mode, repeat)
        .with_builtins(table)
        .with_output(Box::new(log.clone()))
        .with_error_output(Box::new(errlog.clone()));
    let result = interp.run_task(name, &args(argv));
    Ran {
        result,
        out: log.text(),
        err: errlog.text(),
    }
}

fn go(file: &File, name: &str, argv: &[&str]) -> Ran {
    exec(file, name, argv, Mode::Run, Repeat::Once, &root())
}

fn root() -> PathBuf {
    std::env::temp_dir()
}

/// A throwaway directory, removed when the test ends.
struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "chore-interp-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Word expansion
// ---------------------------------------------------------------------------

#[test]
fn unquoted_interpolation_splits_but_quoted_does_not() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign("flags", quoted(vec![text("-a -b -c")])),
            run(cmd("count", vec![unquoted(vec![var("flags")])])),
            run(cmd("count", vec![quoted(vec![var("flags")])])),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["3", "1"]);
}

#[test]
fn literal_whitespace_in_a_word_never_splits() {
    // The source wrote it as one word, so it stays one argument.
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "count",
            vec![quoted(vec![text("MinGW Makefiles")])],
        ))],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["1"]);
}

#[test]
fn interpolation_joins_with_adjacent_literals() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign("dir", lit("build")),
            run(cmd(
                "say",
                vec![unquoted(vec![text("--out="), var("dir"), text("/bin")])],
            )),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["--out=build/bin"]);
}

#[test]
fn empty_variable_contributes_no_argument() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign("nothing", quoted(vec![])),
            run(cmd(
                "count",
                vec![unquoted(vec![var("nothing")]), lit("kept")],
            )),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["1"]);
}

#[test]
fn positional_args_and_all_and_count() {
    let f = file(vec![task(
        "t",
        &["first", "second"],
        vec![
            run(cmd(
                "say",
                vec![unquoted(vec![part(PartKind::Var(VarRef::Positional(2)))])],
            )),
            run(cmd(
                "count",
                vec![unquoted(vec![part(PartKind::Var(VarRef::All))])],
            )),
            run(cmd(
                "say",
                vec![unquoted(vec![part(PartKind::Var(VarRef::Count))])],
            )),
        ],
    )]);
    assert_eq!(go(&f, "t", &["a", "b", "c"]).printed(), ["b", "3", "3"]);
}

#[test]
fn all_args_keep_their_argv_boundaries() {
    // An argument that arrived with a space in it is still one argument.
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "count",
            vec![unquoted(vec![part(PartKind::Var(VarRef::All))])],
        ))],
    )]);
    assert_eq!(go(&f, "t", &["one two", "three"]).printed(), ["2"]);
}

#[test]
fn undefined_variable_is_a_run_error_naming_it() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd("say", vec![unquoted(vec![var("missing")])]))],
    )]);
    let ran = go(&f, "t", &[]);
    let message = ran.err_text();
    assert!(message.contains("$missing"), "{message}");
    assert!(matches!(ran.result, Err(Error::Run { .. })));
}

#[test]
fn missing_positional_is_a_run_error() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "say",
            vec![unquoted(vec![part(PartKind::Var(VarRef::Positional(2)))])],
        ))],
    )]);
    assert!(go(&f, "t", &["only"]).err_text().contains("$2"));
}

#[test]
fn builtin_variables_are_set() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd("say", vec![unquoted(vec![var("TASK")])])),
            run(cmd("say", vec![unquoted(vec![var("PLATFORM")])])),
        ],
    )]);
    let printed = go(&f, "t", &[]).printed();
    assert_eq!(printed[0], "t");
    assert_eq!(
        printed[1],
        format!("{}-{}", chorefile::vars::OS, chorefile::vars::ARCH)
    );
}

#[test]
fn now_is_an_iso_timestamp() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd("say", vec![unquoted(vec![var("NOW")])]))],
    )]);
    let now = go(&f, "t", &[]).printed().remove(0);
    assert_eq!(now.len(), 20, "{now}");
    assert!(now.ends_with('Z') && now.contains('T'), "{now}");
    let year: i32 = now[..4].parse().unwrap();
    assert!(year >= 2024, "{now}");
}

// ---------------------------------------------------------------------------
// Captures
// ---------------------------------------------------------------------------

#[test]
fn capture_is_trimmed_and_splits_when_unquoted() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd(
                "count",
                vec![unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "say",
                    vec![lit("a"), lit("b")],
                ))))])],
            )),
            run(cmd(
                "count",
                vec![quoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "say",
                    vec![lit("a"), lit("b")],
                ))))])],
            )),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["2", "1"]);
}

#[test]
fn a_failed_capture_fails_the_run() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "say",
            vec![unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                "status",
                vec![lit("3")],
            ))))])],
        ))],
    )]);
    assert!(go(&f, "t", &[]).result.is_err());
}

#[test]
fn a_task_can_be_captured() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![run(cmd(
                "say",
                vec![
                    lit("got"),
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                        "inner",
                        vec![],
                    ))))]),
                ],
            ))],
        ),
        task("inner", &[], vec![run(cmd("say", vec![lit("value")]))]),
    ]);
    assert_eq!(go(&f, "t", &[]).printed(), ["got value"]);
}

// ---------------------------------------------------------------------------
// Chains and redirection
// ---------------------------------------------------------------------------

#[test]
fn and_or_short_circuit() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(Chain::And(
                Box::new(cmd("hit", vec![lit("first")])),
                Box::new(cmd("hit", vec![lit("second")])),
            )),
            Stmt::Try(Chain::And(
                Box::new(cmd("status", vec![lit("1")])),
                Box::new(cmd("hit", vec![lit("skipped")])),
            )),
            run(Chain::Or(
                Box::new(cmd("status", vec![lit("1")])),
                Box::new(cmd("hit", vec![lit("fallback")])),
            )),
            run(Chain::Or(
                Box::new(cmd("hit", vec![lit("taken")])),
                Box::new(cmd("hit", vec![lit("unreached")])),
            )),
        ],
    )]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["first", "second", "fallback", "taken"]);
}

#[test]
fn pipe_feeds_stdout_into_stdin() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(Chain::Pipe(
            Box::new(cmd("say", vec![lit("hello")])),
            Box::new(cmd("upper", vec![])),
        ))],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["HELLO"]);
}

#[test]
fn pipe_status_is_the_last_commands() {
    let f = file(vec![task(
        "t",
        &[],
        vec![Stmt::Try(Chain::Pipe(
            Box::new(cmd("say", vec![lit("x")])),
            Box::new(cmd("status", vec![lit("2")])),
        ))],
    )]);
    // The statement is wrapped in `try`, so a nonzero right side is swallowed.
    assert!(go(&f, "t", &[]).result.is_ok());
}

#[test]
fn redirects_write_and_append() {
    let dir = Temp::new("redirect");
    let path = dir.path().join("out.txt");
    let target = chorefile::vars::display(&path);
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd_with(
                "say",
                vec![lit("one")],
                vec![redirect(RedirectKind::Stdout, &target)],
            )),
            run(cmd_with(
                "say",
                vec![lit("two")],
                vec![redirect(RedirectKind::StdoutAppend, &target)],
            )),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    assert_eq!(ran.printed(), Vec::<String>::new());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
}

#[test]
fn a_relative_redirect_lands_in_the_interpreters_directory() {
    let dir = Temp::new("relative");
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd_with(
            "say",
            vec![lit("here")],
            vec![redirect(RedirectKind::Stdout, "log.txt")],
        ))],
    )]);
    exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path()).ok();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("log.txt")).unwrap(),
        "here\n"
    );
}

#[test]
fn a_builtins_stderr_lands_in_its_redirect_file() {
    let dir = Temp::new("stderr-redirect");
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd_with(
            "warn",
            vec![lit("careful")],
            vec![redirect(RedirectKind::Stderr, "err.txt")],
        ))],
    )]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    // stdout is untouched by `2>`, and the file holds what the builtin wrote
    // to `ctx.err` — not an empty file created for show.
    assert_eq!(ran.printed(), ["to stdout"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("err.txt")).unwrap(),
        "careful\n"
    );
}

#[test]
fn a_builtins_stderr_stays_out_of_a_capture() {
    // Without `2>` diagnostics stream, as in sh: what `$(...)` yields is
    // stdout alone.
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "count",
            vec![unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                "warn",
                vec![lit("careful")],
            ))))])],
        ))],
    )]);
    let ran = go(&f, "t", &[]);
    assert_eq!(ran.printed(), ["2"]);
    assert_eq!(ran.err, "careful\n");
}

#[test]
fn a_captured_or_redirected_builtin_is_not_interactive() {
    let dir = Temp::new("interactive");
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd(
                "say",
                vec![quoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "tty",
                    vec![],
                ))))])],
            )),
            run(cmd_with(
                "tty",
                vec![],
                vec![redirect(RedirectKind::Stdout, "tty.txt")],
            )),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    assert_eq!(ran.printed(), ["false"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("tty.txt")).unwrap(),
        "false\n"
    );
}

#[test]
fn a_task_stderr_redirect_catches_what_its_commands_report() {
    let dir = Temp::new("task-stderr");
    let f = file(vec![
        task("inner", &[], vec![run(cmd("warn", vec![lit("careful")]))]),
        task(
            "t",
            &[],
            vec![run(cmd_with(
                "inner",
                vec![],
                vec![redirect(RedirectKind::Stderr, "err.txt")],
            ))],
        ),
    ]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    // Diverted for the length of the call: nothing reached the terminal, and
    // stdout is untouched.
    assert_eq!(ran.printed(), ["to stdout"]);
    assert_eq!(ran.err, "");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("err.txt")).unwrap(),
        "careful\n"
    );
}

#[test]
fn a_redirected_builtin_leaves_nothing_behind_for_the_next_command() {
    // `a > f && b` captured: only `b`'s output is the capture. The bytes `a`
    // wrote belong to the file alone.
    let dir = Temp::new("redirect-leak");
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "say",
            vec![quoted(vec![part(PartKind::Capture(Box::new(Chain::And(
                Box::new(cmd_with(
                    "emit",
                    vec![lit("one")],
                    vec![redirect(RedirectKind::Stdout, "out.txt")],
                )),
                Box::new(cmd("say", vec![lit("two")])),
            ))))])],
        ))],
    )]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    assert_eq!(ran.printed(), ["two"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn a_stderr_redirect_catches_a_builtin_that_fails_hard() {
    let dir = Temp::new("stderr-error");
    let f = file(vec![task(
        "t",
        &[],
        vec![
            Stmt::Try(cmd_with(
                "boom",
                vec![lit("no"), lit("such"), lit("file")],
                vec![redirect(RedirectKind::Stderr, "err.txt")],
            )),
            run(cmd("say", vec![lit("done")])),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    // The failure is the diagnostic, so the redirect must catch it rather than
    // unwind past the file: `2>` works everywhere else even when there is
    // nothing to catch.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("err.txt")).unwrap(),
        "no such file\n"
    );
    assert_eq!(ran.err, "");
    // `try` still sees the failure, and the run carries on.
    assert_eq!(ran.printed(), ["done"]);
}

// ---------------------------------------------------------------------------
// Conditions and control flow
// ---------------------------------------------------------------------------

fn compare(left: Word, op: CompareOp, right: Word) -> Cond {
    Cond::Compare { left, op, right }
}

fn if_stmt(cond: Cond, then: Vec<Stmt>, otherwise: Option<Vec<Stmt>>) -> Stmt {
    Stmt::If(If {
        cond,
        then,
        otherwise,
        span: sp(),
    })
}

#[test]
fn comparison_operators() {
    let cases = [
        (CompareOp::Eq, "abc", true),
        (CompareOp::Ne, "abc", false),
        (CompareOp::Contains, "b", true),
        (CompareOp::StartsWith, "ab", true),
        (CompareOp::EndsWith, "bc", true),
        (CompareOp::StartsWith, "bc", false),
    ];
    for (op, right, expected) in cases {
        let f = file(vec![task(
            "t",
            &[],
            vec![if_stmt(
                compare(lit("abc"), op, lit(right)),
                vec![run(cmd("say", vec![lit("yes")]))],
                Some(vec![run(cmd("say", vec![lit("no")]))]),
            )],
        )]);
        let want = if expected { "yes" } else { "no" };
        assert_eq!(go(&f, "t", &[]).printed(), [want], "{op:?} {right}");
    }
}

#[test]
fn exit_code_not_and_boolean_conditions() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            if_stmt(
                Cond::Command(cmd("status", vec![lit("0")])),
                vec![run(cmd("hit", vec![lit("zero-is-true")]))],
                None,
            ),
            if_stmt(
                Cond::Command(cmd("status", vec![lit("1")])),
                vec![run(cmd("hit", vec![lit("unreached")]))],
                None,
            ),
            if_stmt(
                Cond::Not(Box::new(Cond::Command(cmd("status", vec![lit("1")])))),
                vec![run(cmd("hit", vec![lit("negated")]))],
                None,
            ),
            if_stmt(
                Cond::And(
                    Box::new(compare(lit("a"), CompareOp::Eq, lit("a"))),
                    Box::new(Cond::Or(
                        Box::new(compare(lit("a"), CompareOp::Eq, lit("b"))),
                        Box::new(Cond::Command(cmd("status", vec![lit("0")]))),
                    )),
                ),
                vec![run(cmd("hit", vec![lit("combined")]))],
                None,
            ),
        ],
    )]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["zero-is-true", "negated", "combined"]);
}

#[test]
fn a_condition_does_not_print_or_abort_the_run() {
    let f = file(vec![task(
        "t",
        &[],
        vec![if_stmt(
            Cond::Command(cmd("say", vec![lit("noise")])),
            vec![run(cmd("say", vec![lit("ok")]))],
            None,
        )],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["ok"]);
}

#[test]
fn else_if_chains() {
    let f = file(vec![task(
        "t",
        &["value"],
        vec![if_stmt(
            compare(
                unquoted(vec![part(PartKind::Var(VarRef::Positional(1)))]),
                CompareOp::Eq,
                lit("a"),
            ),
            vec![run(cmd("say", vec![lit("first")]))],
            Some(vec![if_stmt(
                compare(
                    unquoted(vec![part(PartKind::Var(VarRef::Positional(1)))]),
                    CompareOp::Eq,
                    lit("b"),
                ),
                vec![run(cmd("say", vec![lit("second")]))],
                Some(vec![run(cmd("say", vec![lit("otherwise")]))]),
            )]),
        )],
    )]);
    assert_eq!(go(&f, "t", &["b"]).printed(), ["second"]);
    assert_eq!(go(&f, "t", &["z"]).printed(), ["otherwise"]);
}

#[test]
fn for_splits_items_after_interpolation() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign("list", quoted(vec![text("x y")])),
            Stmt::For(For {
                var: "item".into(),
                items: vec![
                    lit("a"),
                    unquoted(vec![var("list")]),
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                        "say",
                        vec![lit("p"), lit("q")],
                    ))))]),
                ],
                body: vec![run(cmd("hit", vec![unquoted(vec![var("item")])]))],
                span: sp(),
            }),
        ],
    )]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["a", "x", "y", "p", "q"]);
}

#[test]
fn try_swallows_a_nonzero_exit() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            Stmt::Try(cmd("status", vec![lit("7")])),
            run(cmd("hit", vec![lit("after")])),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).result.unwrap(), 0);
    assert_eq!(trace(), ["after"]);
}

#[test]
fn a_nonzero_exit_outside_try_aborts_and_names_the_command() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd("status", vec![lit("4")])),
            run(cmd("hit", vec![lit("unreached")])),
        ],
    )]);
    let ran = go(&f, "t", &[]);
    let message = ran.err_text();
    assert!(message.contains("status"), "{message}");
    assert!(message.contains('4'), "{message}");
    assert!(trace().is_empty(), "{:?}", trace());
}

#[test]
fn exit_stops_the_run_and_sets_the_code() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("inner", vec![])),
                run(cmd("hit", vec![lit("unreached")])),
            ],
        ),
        task(
            "inner",
            &[],
            vec![
                Stmt::Exit(Some(lit("3"))),
                run(cmd("hit", vec![lit("also-unreached")])),
            ],
        ),
    ]);
    let ran = go(&f, "t", &[]);
    assert_eq!(ran.result.unwrap(), 3);
    assert!(trace().is_empty(), "{:?}", trace());
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

#[test]
fn run_once_is_keyed_on_name_and_arguments() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("build", vec![lit("debug")])),
                run(cmd("build", vec![lit("debug")])),
                run(cmd("build", vec![lit("release")])),
            ],
        ),
        task(
            "build",
            &["profile"],
            vec![run(cmd(
                "hit",
                vec![unquoted(vec![part(PartKind::Var(VarRef::Positional(1)))])],
            ))],
        ),
    ]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["debug", "release"]);

    // --force runs it every time.
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Always, &root());
    ran.ok();
    assert_eq!(trace(), ["debug", "debug", "release"]);
}

#[test]
fn capturing_a_task_twice_yields_the_same_value_and_runs_it_once() {
    // A task used as a function — the obvious thing to do in a language with
    // no functions. The second `$(id)` used to come back empty.
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                assign(
                    "a",
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd("id", vec![]))))]),
                ),
                assign(
                    "b",
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd("id", vec![]))))]),
                ),
                run(cmd(
                    "say",
                    vec![
                        unquoted(vec![text("a=["), var("a"), text("]")]),
                        unquoted(vec![text("b=["), var("b"), text("]")]),
                    ],
                )),
            ],
        ),
        task(
            "id",
            &[],
            vec![
                run(cmd("hit", vec![lit("body")])),
                run(cmd("say", vec![lit("hello")])),
            ],
        ),
    ]);

    let ran = go(&f, "t", &[]);
    assert_eq!(ran.printed(), ["a=[hello] b=[hello]"]);
    // The value was replayed, not recomputed: the body ran exactly once.
    assert_eq!(trace(), ["body"]);

    // --force runs it again, and both answers are still right.
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Always, &root());
    assert_eq!(ran.printed(), ["a=[hello] b=[hello]"]);
    assert_eq!(trace(), ["body", "body"]);
}

#[test]
fn a_captured_value_is_replayed_through_a_pipe_and_a_redirect_too() {
    let dir = Temp::new("replay");
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd_with(
                    "id",
                    vec![],
                    vec![redirect(RedirectKind::Stdout, "first.txt")],
                )),
                run(Chain::Pipe(
                    Box::new(cmd("id", vec![])),
                    Box::new(cmd("upper", vec![])),
                )),
            ],
        ),
        task("id", &[], vec![run(cmd("say", vec![lit("hello")]))]),
    ]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("first.txt")).unwrap(),
        "hello\n"
    );
    // The pipe read the recorded value rather than an empty buffer.
    assert_eq!(ran.printed(), ["HELLO"]);
}

#[test]
fn a_task_that_only_streamed_runs_again_when_a_value_is_finally_wanted() {
    // Nothing recorded what it printed the first time, so the honest way to
    // answer the capture is to run it — an empty string would be interpolated
    // into a path.
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("id", vec![])),
                assign(
                    "a",
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd("id", vec![]))))]),
                ),
                run(cmd(
                    "say",
                    vec![unquoted(vec![text("["), var("a"), text("]")])],
                )),
            ],
        ),
        task(
            "id",
            &[],
            vec![
                run(cmd("hit", vec![lit("body")])),
                run(cmd("say", vec![lit("hello")])),
            ],
        ),
    ]);
    let ran = go(&f, "t", &[]);
    assert_eq!(ran.printed(), ["hello", "[hello]"]);
    assert_eq!(trace(), ["body", "body"]);
}

#[test]
fn calling_a_task_with_too_few_arguments_is_an_error() {
    let f = file(vec![
        task("t", &[], vec![run(cmd("build", vec![]))]),
        task("build", &["profile"], vec![]),
    ]);
    let message = go(&f, "t", &[]).err_text();
    assert!(
        message.contains("build") && message.contains("profile"),
        "{message}"
    );
}

#[test]
fn unknown_task_and_unknown_command_are_named() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd("chore-does-not-exist-anywhere", vec![]))],
    )]);
    let message = go(&f, "t", &[]).err_text();
    assert!(
        message.contains("chore-does-not-exist-anywhere"),
        "{message}"
    );
}

#[test]
fn locals_do_not_escape_a_task_but_globals_are_shared() {
    let mut f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("inner", vec![])),
                run(cmd("say", vec![unquoted(vec![var("shared")])])),
            ],
        ),
        task(
            "inner",
            &[],
            vec![assign("shared", lit("local")), assign("other", lit("x"))],
        ),
    ]);
    f.globals.push(Assign {
        name: "shared".into(),
        value: lit("global"),
        span: sp(),
    });
    assert_eq!(go(&f, "t", &[]).printed(), ["global"]);
}

#[test]
fn cd_moves_the_interpreter_and_does_not_leak_between_tasks() {
    let dir = Temp::new("cd");
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    let before = std::env::current_dir().unwrap();

    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("inner", vec![])),
                run(cmd("here", vec![])),
                run(cmd("touch", vec![lit("root.txt")])),
            ],
        ),
        task(
            "inner",
            &[],
            vec![run(cmd("cd", vec![lit("sub")])), run(cmd("here", vec![]))],
        ),
    ]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    let printed = ran.printed();
    assert!(printed[0].ends_with("/sub"), "{printed:?}");
    assert!(!printed[1].ends_with("/sub"), "{printed:?}");
    // The process itself never moved, and a relative path still resolves
    // against the interpreter's directory.
    assert_eq!(std::env::current_dir().unwrap(), before);
    assert!(dir.path().join("root.txt").exists());
}

// ---------------------------------------------------------------------------
// Echo and --dry
// ---------------------------------------------------------------------------

#[test]
fn every_command_is_echoed_with_a_dollar_prefix() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign("flag", quoted(vec![text("MinGW Makefiles")])),
            run(cmd_with(
                "say",
                vec![lit("-G"), quoted(vec![var("flag")])],
                vec![redirect(RedirectKind::StdoutAppend, "log.txt")],
            )),
        ],
    )]);
    let dir = Temp::new("echo");
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    assert_eq!(ran.echoed(), [r#"$ say -G "MinGW Makefiles" >> log.txt"#]);
}

#[test]
fn dry_skips_effects_but_still_echoes() {
    let dir = Temp::new("dry");
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd("touch", vec![lit("made.txt")]))],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, dir.path());
    assert_eq!(ran.echoed(), ["$ touch made.txt"]);
    assert!(!dir.path().join("made.txt").exists());
}

#[test]
fn dry_still_runs_captures_and_conditions() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign(
                "name",
                unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "say",
                    vec![lit("computed")],
                ))))]),
            ),
            if_stmt(
                Cond::Command(cmd("status", vec![lit("0")])),
                vec![run(cmd("say", vec![unquoted(vec![var("name")])]))],
                Some(vec![run(cmd("hit", vec![lit("unreached")]))]),
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    // The capture ran, so the echoed preview names the real path.
    assert_eq!(ran.echoed(), ["$ say computed"]);
}

#[test]
fn dry_does_not_spawn_a_path_program_but_a_capture_does() {
    let missing = "chore-no-such-program-xyz";
    let f = file(vec![task("t", &[], vec![run(cmd(missing, vec![]))])]);
    // Nothing spawns, so a program that does not exist is not an error here.
    assert!(
        exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root())
            .result
            .is_ok()
    );

    let captured = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "say",
            vec![unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                missing,
                vec![],
            ))))])],
        ))],
    )]);
    // A capture must run even under --dry, so this one does reach the OS —
    // and the spawn failure is reported without ending the preview, because
    // the program may be the one a skipped `download` installs.
    let ran = exec(&captured, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert!(ran.err.contains(missing), "{}", ran.err);
}

// ---------------------------------------------------------------------------
// --dry: a command that cannot run does not end the preview
// ---------------------------------------------------------------------------

#[test]
fn dry_reports_a_failing_builtin_and_keeps_previewing() {
    // `boom` stands in for `read` of a file the skipped `download` would have
    // fetched: under --dry it cannot succeed, and the steps after it are
    // exactly what the author wanted to see.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd("boom", vec![lit("cannot read manifest.json")])),
            run(cmd("hit", vec![lit("after")])),
        ],
    )]);

    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["after"]);
    assert!(ran.err.contains("cannot read manifest.json"), "{}", ran.err);

    // Mode::Run is untouched: there the failure is the run's.
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, &root());
    assert!(ran.result.is_err(), "{}", ran.out);
    assert!(trace().is_empty(), "{:?}", trace());
}

#[test]
fn dry_still_stops_at_fail() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd("fail", vec![lit("nope")])),
            run(cmd("hit", vec![lit("after")])),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    assert!(ran.result.is_err(), "{}", ran.out);
    assert!(trace().is_empty(), "{:?}", trace());
}

#[test]
fn dry_takes_the_then_branch_of_a_condition_it_cannot_evaluate() {
    let build = |cond: Cond| {
        file(vec![task(
            "t",
            &[],
            vec![if_stmt(
                cond,
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            )],
        )])
    };

    // Unanswerable: previewing the work beats previewing nothing.
    let f = build(Cond::Command(cmd("boom", vec![lit("not a directory")])));
    exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root()).ok();
    assert_eq!(trace(), ["then"]);

    // The choice is made above the condition, so `not` does not flip it.
    let f = build(Cond::Not(Box::new(Cond::Command(cmd(
        "boom",
        vec![lit("not a directory")],
    )))));
    exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root()).ok();
    assert_eq!(trace(), ["then"]);

    // A condition that *did* answer is believed, nonzero and all.
    let f = build(Cond::Command(cmd("status", vec![lit("1")])));
    exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root()).ok();
    assert_eq!(trace(), ["else"]);

    // ...and Mode::Run answers false, as it always did.
    let f = build(Cond::Command(cmd("boom", vec![lit("no")])));
    exec(&f, "t", &[], Mode::Run, Repeat::Once, &root()).ok();
    assert_eq!(trace(), ["else"]);
}

/// Under `--dry` a condition is believed only when its command *answered*.
/// `exists` answers a miss with a nonzero exit, so its `else` branch is
/// previewed; `read` reports a miss as a failure, which leaves the condition
/// undecided, and an undecided condition previews the `then` branch. The rule
/// is positional — where the command sits, not which command it is.
#[test]
fn dry_believes_a_condition_that_answered_and_previews_one_that_failed() {
    let dir = Temp::new("decided");
    let branches = |cond: Cond| {
        file(vec![task(
            "t",
            &[],
            vec![if_stmt(
                cond,
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            )],
        )])
    };
    let preview = |cond: Cond| {
        let f = branches(cond);
        exec(&f, "t", &[], Mode::Dry, Repeat::Once, dir.path()).ok();
        trace()
    };
    let missing = || lit("build/version.txt");

    // The two lines the rule has to tell apart, on a file that is not there.
    assert_eq!(
        preview(Cond::Command(cmd("exists", vec![missing()]))),
        ["else"]
    );
    assert_eq!(
        preview(Cond::Command(cmd("read", vec![missing()]))),
        ["then"]
    );

    // `not` cannot flip an undecided condition: there is no truth value to
    // negate, and the branch was chosen above the condition.
    assert_eq!(
        preview(Cond::Not(Box::new(Cond::Command(cmd(
            "read",
            vec![missing()]
        ))))),
        ["then"]
    );
    // ...but it does flip one that answered.
    assert_eq!(
        preview(Cond::Not(Box::new(Cond::Command(cmd(
            "exists",
            vec![missing()]
        ))))),
        ["then"]
    );

    // Composition: a failure anywhere leaves the whole condition undecided,
    // while short-circuiting is respected — `exists` answered "no" and
    // decided the condition before `read` ever ran.
    assert_eq!(
        preview(Cond::And(
            Box::new(Cond::Command(cmd("exists", vec![missing()]))),
            Box::new(Cond::Command(cmd("read", vec![missing()]))),
        )),
        ["else"]
    );
    assert_eq!(
        preview(Cond::Or(
            Box::new(Cond::Command(cmd("read", vec![missing()]))),
            Box::new(Cond::Command(cmd("exists", vec![missing()]))),
        )),
        ["then"]
    );

    // Once the file is there, both answer, and both answer the same way.
    std::fs::create_dir_all(dir.path().join("build")).unwrap();
    std::fs::write(dir.path().join("build/version.txt"), "1\n").unwrap();
    assert_eq!(
        preview(Cond::Command(cmd("exists", vec![missing()]))),
        ["then"]
    );
    assert_eq!(
        preview(Cond::Command(cmd("read", vec![missing()]))),
        ["then"]
    );

    // Mode::Run is untouched: a failing condition there is simply false.
    let f = branches(Cond::Command(cmd("read", vec![lit("build/absent.txt")])));
    exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path()).ok();
    assert_eq!(trace(), ["else"]);
}

/// A program on `PATH` inside a condition really runs under `--dry`, so its
/// nonzero exit is an answer and is believed. Only one that cannot be spawned
/// at all leaves the condition undecided.
#[test]
#[cfg(unix)]
fn dry_believes_a_path_program_that_ran_and_not_one_that_could_not() {
    let branches = |name: &str| {
        file(vec![task(
            "t",
            &[],
            vec![if_stmt(
                Cond::Command(cmd(name, vec![])),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            )],
        )])
    };

    let f = branches("/usr/bin/false");
    exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root()).ok();
    assert_eq!(trace(), ["else"]);

    let f = branches("chore-no-such-program-xyz");
    exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root()).ok();
    assert_eq!(trace(), ["then"]);
}

#[test]
fn dry_gives_a_failed_capture_the_empty_string_and_carries_on() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign(
                "v",
                unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "boom",
                    vec![lit("no manifest")],
                ))))]),
            ),
            run(cmd("hit", vec![lit("after")])),
        ],
    )]);

    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["after"]);
    assert!(ran.err.contains("no manifest"), "{}", ran.err);

    // Mode::Run still refuses to interpolate a value it does not have.
    assert!(
        exec(&f, "t", &[], Mode::Run, Repeat::Once, &root())
            .result
            .is_err()
    );
}

#[test]
fn dry_branches_on_a_softened_failure_the_way_a_shell_would() {
    // `||` sees a failed left side and runs its right; `&&` does not.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(Chain::Or(
                Box::new(cmd("boom", vec![lit("missing")])),
                Box::new(cmd("hit", vec![lit("fallback")])),
            )),
            run(Chain::And(
                Box::new(cmd("boom", vec![lit("missing")])),
                Box::new(cmd("hit", vec![lit("unreached")])),
            )),
            run(cmd("hit", vec![lit("end")])),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["fallback", "end"]);
}

// ---------------------------------------------------------------------------
// PATH
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn a_path_program_receives_argv_without_re_quoting() {
    use std::os::unix::fs::PermissionsExt;

    let dir = Temp::new("argv");
    let script = dir.path().join("show.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nfor a in \"$@\"; do echo \"[$a]\"; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Captured, so the test can read what the program actually received.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign("arg", quoted(vec![text("two words")])),
            run(cmd(
                "say",
                vec![quoted(vec![part(PartKind::Capture(Box::new(cmd(
                    &chorefile::vars::display(&script),
                    vec![quoted(vec![var("arg")]), lit("plain")],
                ))))])],
            )),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    // One quoted word stayed one argument all the way to the OS.
    assert!(ran.ok().ends_with("[two words]\n[plain]\n"), "{}", ran.ok());
}

#[test]
fn tasks_are_listed_in_source_order() {
    let f = file(vec![task("a", &[], vec![]), task("b", &[], vec![])]);
    let interp = Interpreter::new(&f, root(), Mode::Run, Repeat::Once);
    let names: Vec<_> = interp.tasks().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert!(interp.task("b").is_some());
    assert!(interp.task("c").is_none());
}
