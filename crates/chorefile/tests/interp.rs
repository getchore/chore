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
    with_params(name, params.iter().map(|p| required(p)).collect(), body)
}

/// A task whose parameters carry defaults.
fn with_params(name: &str, params: Vec<Param>, body: Block) -> Task {
    Task {
        name: name.into(),
        params,
        doc: None,
        body,
        span: sp(),
    }
}

/// A parameter the caller must supply.
fn required(name: &str) -> Param {
    Param {
        name: name.into(),
        default: None,
        span: sp(),
    }
}

/// A parameter with a default, evaluated at call time when it is left off.
fn optional(name: &str, default: Word) -> Param {
    Param {
        name: name.into(),
        default: Some(default),
        span: sp(),
    }
}

fn file(tasks: Vec<Task>) -> File {
    File {
        require: None,
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
        "look" => look,
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

/// `look <NAME...>` prints what the *builtin* sees in the environment, which is
/// the overlay `env` writes rather than the process environment. `download`
/// reads `GITHUB_TOKEN` this way, and `which` reads `PATH`.
fn look(ctx: &mut Ctx<'_>) -> Result<Output> {
    let seen: Vec<String> = ctx
        .rest()
        .iter()
        .map(|name| ctx.env.get(name).unwrap_or_else(|| "<unset>".to_string()))
        .collect();
    writeln!(ctx.out, "{}", seen.join(" "))?;
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

/// The shape the statement exists for: a `setup` task that finds its work
/// already done stops itself, and the `dev` task that called it goes on to the
/// next step. `exit` would have taken `dev` down with it.
#[test]
fn return_ends_the_task_and_the_caller_carries_on() {
    let f = file(vec![
        task(
            "dev",
            &[],
            vec![
                run(cmd("setup", vec![])),
                run(cmd("hit", vec![lit("tauri-dev")])),
            ],
        ),
        task(
            "setup",
            &[],
            vec![
                run(cmd("hit", vec![lit("checked")])),
                Stmt::Return(None),
                run(cmd("hit", vec![lit("downloaded")])),
            ],
        ),
    ]);
    let ran = go(&f, "dev", &[]);
    assert_eq!(ran.result.unwrap(), 0);
    assert_eq!(trace(), ["checked", "tauri-dev"]);
}

/// There is no `break`, and someone will reach for `return` as one: it leaves
/// the task, not the iteration and not just the loop.
#[test]
fn return_inside_a_for_leaves_the_whole_task() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("find-first", vec![])),
                run(cmd("hit", vec![lit("after-call")])),
            ],
        ),
        task(
            "find-first",
            &[],
            vec![
                Stmt::For(For {
                    var: "item".into(),
                    items: vec![lit("a"), lit("b"), lit("c")],
                    body: vec![
                        run(cmd("hit", vec![unquoted(vec![var("item")])])),
                        Stmt::Return(None),
                    ],
                    span: sp(),
                }),
                run(cmd("hit", vec![lit("past-the-loop")])),
            ],
        ),
    ]);
    go(&f, "t", &[]).ok();
    // One iteration, nothing after the loop, and the caller still ran.
    assert_eq!(trace(), ["a", "after-call"]);
}

#[test]
fn return_inside_a_nested_if_leaves_the_task() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("guard", vec![])),
                run(cmd("hit", vec![lit("after-call")])),
            ],
        ),
        task(
            "guard",
            &[],
            vec![
                Stmt::If(If {
                    cond: Cond::Compare {
                        left: lit("yes"),
                        op: CompareOp::Eq,
                        right: lit("yes"),
                    },
                    then: vec![Stmt::If(If {
                        cond: Cond::Command(cmd("status", vec![lit("0")])),
                        then: vec![Stmt::Return(None)],
                        otherwise: None,
                        span: sp(),
                    })],
                    otherwise: None,
                    span: sp(),
                }),
                run(cmd("hit", vec![lit("unreached")])),
            ],
        ),
    ]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["after-call"]);
}

/// A task that returned early still produced whatever it printed first, and
/// that value — not an empty string — is what the capture gets. Run-once
/// records it, so the second capture replays it without running the body.
#[test]
fn a_captured_task_that_returns_early_yields_what_it_printed() {
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
                run(cmd("say", vec![lit("cached")])),
                Stmt::Return(None),
                run(cmd("say", vec![lit("unreached")])),
            ],
        ),
    ]);
    let ran = go(&f, "t", &[]);
    assert_eq!(ran.printed(), ["a=[cached] b=[cached]"]);
    assert_eq!(trace(), ["body"]);
}

/// The code is the task's exit status, so every construct that reads a
/// command's status reads a `return` the same way.
#[test]
fn a_return_code_becomes_the_tasks_status() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                // `||` sees the failure and takes over; `&&` would not run.
                run(Chain::Or(
                    Box::new(cmd("nope", vec![])),
                    Box::new(cmd("hit", vec![lit("fallback")])),
                )),
                Stmt::If(If {
                    cond: Cond::Command(cmd("nope", vec![])),
                    then: vec![run(cmd("hit", vec![lit("unreached")]))],
                    otherwise: Some(vec![run(cmd("hit", vec![lit("condition-false")]))]),
                    span: sp(),
                }),
                // `try` swallows it, the same as any other nonzero command.
                Stmt::Try(cmd("nope", vec![])),
                run(cmd("hit", vec![lit("after-try")])),
            ],
        ),
        task("nope", &[], vec![Stmt::Return(Some(lit("3")))]),
    ]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Always, &root());
    assert_eq!(ran.result.unwrap(), 0);
    assert_eq!(trace(), ["fallback", "condition-false", "after-try"]);
}

/// Outside `try` and the operators, a nonzero `return` stops the caller
/// fail-fast, exactly as a command that exited nonzero would.
#[test]
fn an_unhandled_nonzero_return_stops_the_caller() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("nope", vec![])),
                run(cmd("hit", vec![lit("unreached")])),
            ],
        ),
        task("nope", &[], vec![Stmt::Return(Some(lit("4")))]),
    ]);
    let message = go(&f, "t", &[]).err_text();
    assert!(message.contains("nope"), "{message}");
    assert!(message.contains('4'), "{message}");
    assert!(trace().is_empty(), "{:?}", trace());
}

/// In the task the command line named there is no caller left, so `return`
/// ends the run — successfully unless it named a code.
#[test]
fn return_in_the_top_level_task_ends_the_run() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd("hit", vec![lit("first")])),
            Stmt::Return(None),
            run(cmd("hit", vec![lit("unreached")])),
        ],
    )]);
    let ran = go(&f, "t", &[]);
    assert_eq!(ran.result.unwrap(), 0);
    assert_eq!(trace(), ["first"]);

    let f = file(vec![task("t", &[], vec![Stmt::Return(Some(lit("5")))])]);
    assert_eq!(go(&f, "t", &[]).result.unwrap(), 5);
}

/// A `return` code that is not a number is the author's mistake, and the
/// message names the statement it was written on.
#[test]
fn a_non_numeric_return_code_is_an_error() {
    let f = file(vec![task("t", &[], vec![Stmt::Return(Some(lit("soon")))])]);
    let message = go(&f, "t", &[]).err_text();
    assert!(message.contains("return code"), "{message}");
    assert!(message.contains("soon"), "{message}");
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

// ---------------------------------------------------------------------------
// Optional parameters
// ---------------------------------------------------------------------------

/// `$n`, the way a task body reaches a parameter.
fn pos(n: usize) -> WordPart {
    part(PartKind::Var(VarRef::Positional(n)))
}

/// A word that captures a chain, for a default with a cost.
fn capture(chain: Chain) -> Word {
    unquoted(vec![part(PartKind::Capture(Box::new(chain)))])
}

#[test]
fn a_default_binds_the_parameter_the_caller_left_off() {
    let f = file(vec![with_params(
        "t",
        vec![optional("env", lit("staging"))],
        vec![run(cmd("say", vec![unquoted(vec![pos(1)])]))],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["staging"]);
}

#[test]
fn an_explicit_argument_wins_over_the_default() {
    let f = file(vec![with_params(
        "t",
        vec![optional("env", lit("staging"))],
        vec![run(cmd("say", vec![unquoted(vec![pos(1)])]))],
    )]);
    assert_eq!(go(&f, "t", &["prod"]).printed(), ["prod"]);
}

#[test]
fn a_default_reads_the_callees_scope_not_the_callers() {
    // The caller has a local `TRIPLE` of its own. A default is evaluated in
    // the frame it is filling, so the callee's default sees the global.
    let mut f = file(vec![
        task(
            "caller",
            &[],
            vec![assign("TRIPLE", lit("the-callers")), run(cmd("t", vec![]))],
        ),
        with_params(
            "t",
            vec![optional("target", unquoted(vec![var("TRIPLE")]))],
            vec![run(cmd("say", vec![unquoted(vec![pos(1)])]))],
        ),
    ]);
    f.globals.push(Assign {
        name: "TRIPLE".into(),
        value: lit("aarch64-apple-darwin"),
        span: sp(),
    });
    assert_eq!(go(&f, "caller", &[]).printed(), ["aarch64-apple-darwin"]);
}

#[test]
fn a_default_sees_the_parameters_declared_before_it() {
    // Binding is left to right, so `$1` is already there when `b`'s default
    // is evaluated.
    let f = file(vec![with_params(
        "t",
        vec![required("a"), optional("b", unquoted(vec![pos(1)]))],
        vec![run(cmd("say", vec![unquoted(vec![pos(2)])]))],
    )]);
    assert_eq!(go(&f, "t", &["one"]).printed(), ["one"]);
}

#[test]
fn a_default_runs_only_when_it_is_used() {
    // The whole point of evaluating at call time: `url=$(read .env)` must not
    // read anything when the caller supplied a url. The default records that
    // it ran, and prints a value, so both halves are observable.
    let probe = || {
        capture(Chain::And(
            Box::new(cmd("hit", vec![lit("default")])),
            Box::new(cmd("say", vec![lit("fallback")])),
        ))
    };
    let f = file(vec![with_params(
        "t",
        vec![optional("url", probe())],
        vec![run(cmd("say", vec![unquoted(vec![pos(1)])]))],
    )]);

    let bare = go(&f, "t", &[]);
    assert_eq!(bare.printed(), ["fallback"]);
    assert_eq!(trace(), ["default"]);

    let supplied = go(&f, "t", &["https://example.invalid"]);
    assert_eq!(supplied.printed(), ["https://example.invalid"]);
    assert!(trace().is_empty(), "the default ran anyway: {:?}", trace());
}

#[test]
fn count_and_all_reflect_the_bound_arguments() {
    // A `$1` that exists while `$#` says zero would be incoherent, so the
    // filled-in defaults are part of both.
    let f = file(vec![with_params(
        "t",
        vec![optional("a", lit("x")), optional("b", lit("y"))],
        vec![
            run(cmd(
                "say",
                vec![unquoted(vec![part(PartKind::Var(VarRef::Count))])],
            )),
            run(cmd(
                "count",
                vec![unquoted(vec![part(PartKind::Var(VarRef::All))])],
            )),
            run(cmd(
                "say",
                vec![unquoted(vec![part(PartKind::Var(VarRef::All))])],
            )),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["2", "2", "x y"]);
    assert_eq!(go(&f, "t", &["a"]).printed(), ["2", "2", "a y"]);
}

#[test]
fn extra_arguments_past_the_last_parameter_are_still_allowed() {
    let f = file(vec![with_params(
        "t",
        vec![optional("a", lit("x"))],
        vec![run(cmd(
            "count",
            vec![unquoted(vec![part(PartKind::Var(VarRef::All))])],
        ))],
    )]);
    assert_eq!(go(&f, "t", &["one", "two", "three"]).printed(), ["3"]);
}

#[test]
fn a_default_that_expands_to_two_words_still_fills_one_parameter() {
    // A default is a value, like the right-hand side of an assignment: it
    // fills its own slot and cannot spill into the next parameter's.
    let mut f = file(vec![with_params(
        "t",
        vec![optional("flags", unquoted(vec![var("both")]))],
        vec![
            run(cmd("count", vec![unquoted(vec![pos(1)])])),
            run(cmd(
                "say",
                vec![unquoted(vec![part(PartKind::Var(VarRef::Count))])],
            )),
        ],
    )]);
    f.globals.push(Assign {
        name: "both".into(),
        value: lit("-a -b"),
        span: sp(),
    });
    // `count $1` splits at *that* call site — the word there is an argument —
    // but `$#` shows the two words went into one parameter.
    assert_eq!(go(&f, "t", &[]).printed(), ["2", "1"]);
}

#[test]
fn run_once_keys_on_the_bound_arguments_not_the_written_ones() {
    // `t` and `t staging` name the same work when `staging` is the default,
    // so the body runs once between them.
    let f = file(vec![
        task(
            "caller",
            &[],
            vec![
                run(cmd("t", vec![])),
                run(cmd("t", vec![lit("staging")])),
                run(cmd("t", vec![lit("prod")])),
            ],
        ),
        with_params(
            "t",
            vec![optional("env", lit("staging"))],
            vec![run(cmd("hit", vec![unquoted(vec![pos(1)])]))],
        ),
    ]);
    go(&f, "caller", &[]);
    assert_eq!(trace(), ["staging", "prod"]);
}

#[test]
fn force_still_runs_a_defaulted_task_every_time() {
    let f = file(vec![
        task(
            "caller",
            &[],
            vec![run(cmd("t", vec![])), run(cmd("t", vec![lit("staging")]))],
        ),
        with_params(
            "t",
            vec![optional("env", lit("staging"))],
            vec![run(cmd("hit", vec![unquoted(vec![pos(1)])]))],
        ),
    ]);
    exec(&f, "caller", &[], Mode::Run, Repeat::Always, &root());
    assert_eq!(trace(), ["staging", "staging"]);
}

#[test]
fn a_missing_required_argument_is_named_and_the_optional_one_is_not() {
    let f = file(vec![
        task("t", &[], vec![run(cmd("deploy", vec![]))]),
        with_params(
            "deploy",
            vec![required("url"), optional("dest", lit("build"))],
            vec![],
        ),
    ]);
    let message = go(&f, "t", &[]).err_text();
    assert!(
        message.contains("missing required argument(s) `url`")
            && message.contains("usage: deploy <url> [dest=build]"),
        "{message}"
    );
}

#[test]
fn a_task_with_only_optional_parameters_may_be_called_bare() {
    let f = file(vec![
        task("t", &[], vec![run(cmd("build", vec![]))]),
        with_params(
            "build",
            vec![optional("profile", lit("debug"))],
            vec![run(cmd("say", vec![unquoted(vec![pos(1)])]))],
        ),
    ]);
    assert_eq!(go(&f, "t", &[]).printed(), ["debug"]);
}

#[test]
fn a_default_that_fails_fails_the_call() {
    let f = file(vec![with_params(
        "t",
        vec![optional(
            "x",
            capture(cmd("boom", vec![lit("no such thing")])),
        )],
        vec![run(cmd("say", vec![unquoted(vec![pos(1)])]))],
    )]);
    let message = go(&f, "t", &[]).err_text();
    assert!(message.contains("no such thing"), "{message}");
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
// env
//
// `env NAME value` is per-call, like `cd` and locals, and nothing here — or
// anywhere else — writes the process environment. The per-command form
// `env NAME=value <cmd>` opens the same scope for exactly one command.
// ---------------------------------------------------------------------------

/// A name no real environment has, so a test that passes because the machine
/// happened to export something is impossible.
const UNIQUE: &str = "CHOREFILE_TEST_SCOPED_VAR";

/// `env NAME value`.
fn set(name: &str, value: &str) -> Stmt {
    run(cmd("env", vec![lit(name), lit(value)]))
}

/// `look NAME` — what a builtin running now sees.
fn look_at(name: &str) -> Stmt {
    run(cmd("look", vec![lit(name)]))
}

#[test]
fn a_set_reaches_the_tasks_this_one_calls_and_dies_with_it() {
    let f = file(vec![
        task("t", &[], vec![run(cmd("a", vec![])), run(cmd("c", vec![]))]),
        task("a", &[], vec![set(UNIQUE, "from-a"), run(cmd("b", vec![]))]),
        task("b", &[], vec![look_at(UNIQUE)]),
        task("c", &[], vec![look_at(UNIQUE)]),
    ]);
    // `b` runs inside `a`'s call and sees it; `c` runs after `a` returned and
    // does not. This is the bug the scope exists for: a `run` task that sets
    // `TERRA_SOCKET` must not still be setting it for whatever comes next.
    assert_eq!(go(&f, "t", &[]).printed(), ["from-a", "<unset>"]);
    // And the process itself never learned about it.
    assert!(std::env::var(UNIQUE).is_err());
}

#[test]
fn a_set_at_the_top_of_a_task_is_visible_to_the_rest_of_it() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            look_at(UNIQUE),
            set(UNIQUE, "one"),
            look_at(UNIQUE),
            set(UNIQUE, "two"),
            look_at(UNIQUE),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["<unset>", "one", "two"]);
}

#[test]
fn reading_goes_through_the_overlay() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            set(UNIQUE, "value"),
            run(cmd("env", vec![lit(UNIQUE)])),
            // And into a capture, which is how a chorefile moves it into a
            // variable.
            assign(
                "v",
                unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "env",
                    vec![lit(UNIQUE)],
                ))))]),
            ),
            run(cmd("say", vec![unquoted(vec![var("v")])])),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["value", "value"]);
}

#[test]
fn reading_an_unset_name_is_an_answer_and_not_a_failure() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            if_stmt(
                Cond::Command(cmd("env", vec![lit(UNIQUE)])),
                vec![run(cmd("say", vec![lit("set")]))],
                Some(vec![run(cmd("say", vec![lit("unset")]))]),
            ),
            set(UNIQUE, "now"),
            if_stmt(
                Cond::Command(cmd("env", vec![lit(UNIQUE)])),
                vec![run(cmd("say", vec![lit("set")]))],
                Some(vec![run(cmd("say", vec![lit("unset")]))]),
            ),
        ],
    )]);
    let ran = go(&f, "t", &[]);
    assert_eq!(ran.printed(), ["unset", "set"]);
    assert!(ran.err.contains("is not set"), "{:?}", ran.err);
}

#[test]
fn the_per_command_form_binds_for_one_builtin_only() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            run(cmd(
                "env",
                vec![lit(&format!("{UNIQUE}=once")), lit("look"), lit(UNIQUE)],
            )),
            look_at(UNIQUE),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["once", "<unset>"]);
}

#[test]
fn the_per_command_form_takes_several_bindings() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "env",
            vec![
                lit("CHOREFILE_TEST_A=1"),
                lit("CHOREFILE_TEST_B=2"),
                lit("look"),
                lit("CHOREFILE_TEST_A"),
                lit("CHOREFILE_TEST_B"),
            ],
        ))],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["1 2"]);
}

#[test]
fn the_per_command_form_reaches_a_whole_task_call() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd(
                    "env",
                    vec![lit(&format!("{UNIQUE}=for-the-call")), lit("inner")],
                )),
                look_at(UNIQUE),
            ],
        ),
        // The callee sees it for its whole body, and for anything it calls:
        // the frame it pushes sits inside the scope the binding opened.
        task(
            "inner",
            &[],
            vec![look_at(UNIQUE), run(cmd("deeper", vec![]))],
        ),
        task("deeper", &[], vec![look_at(UNIQUE)]),
    ]);
    assert_eq!(
        go(&f, "t", &[]).printed(),
        ["for-the-call", "for-the-call", "<unset>"]
    );
}

#[test]
fn a_binding_shadows_an_outer_one_and_gives_it_back() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            set(UNIQUE, "outer"),
            run(cmd(
                "env",
                vec![lit(&format!("{UNIQUE}=inner")), lit("look"), lit(UNIQUE)],
            )),
            look_at(UNIQUE),
        ],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), ["inner", "outer"]);
}

#[test]
fn an_empty_value_is_a_value() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "env",
            vec![lit(&format!("{UNIQUE}=")), lit("look"), lit(UNIQUE)],
        ))],
    )]);
    assert_eq!(go(&f, "t", &[]).printed(), [""]);
}

#[test]
fn a_binding_with_no_command_says_how_to_set_one() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd("env", vec![lit(&format!("{UNIQUE}=value"))]))],
    )]);
    let message = go(&f, "t", &[]).err_text();
    assert!(message.contains("needs a command"), "{message}");
    assert!(
        message.contains(&format!("env {UNIQUE} value")),
        "{message}"
    );
}

#[test]
fn a_first_argument_with_an_equals_that_is_not_a_name_is_refused() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd("env", vec![lit("not-a-name=1"), lit("say")]))],
    )]);
    let message = go(&f, "t", &[]).err_text();
    assert!(message.contains("not a NAME=value binding"), "{message}");
}

#[test]
fn a_set_under_dry_is_carried_out_so_a_later_read_is_truthful() {
    // Nothing outside the run changes, so there is no effect to skip — and a
    // preview whose `if env NAME` answered from the developer's own shell
    // would describe branches the run will not take.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            set(UNIQUE, "previewed"),
            look_at(UNIQUE),
            if_stmt(
                Cond::Command(cmd("env", vec![lit(UNIQUE)])),
                vec![run(cmd("say", vec![lit("branch taken")]))],
                None,
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    assert_eq!(ran.printed(), ["previewed", "branch taken"]);
    // The value is one the chorefile named, not one the preview invented, so
    // no note is printed about it.
    assert!(!ran.err.contains("invented"), "{:?}", ran.err);
    assert!(std::env::var(UNIQUE).is_err());
}

#[test]
fn the_per_command_form_is_echoed_with_its_bindings() {
    let f = file(vec![task(
        "t",
        &[],
        vec![run(cmd(
            "env",
            vec![lit("CHOREFILE_TEST_A=1"), lit("say"), lit("hi")],
        ))],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    assert_eq!(ran.echoed(), ["$ env CHOREFILE_TEST_A=1 say hi"]);
}

/// A real child process, which is the only proof that the overlay reaches the
/// other side of a `spawn` — every other test here reads it inside chore.
#[cfg(unix)]
#[test]
fn a_spawned_program_is_given_the_bindings() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            set(UNIQUE, "from-the-task"),
            // `sh` prints what it was handed, and the capture brings it back.
            assign(
                "outer",
                unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "sh",
                    vec![
                        lit("-c"),
                        quoted(vec![text(&format!("printf %s \"${UNIQUE}\""))]),
                    ],
                ))))]),
            ),
            run(cmd("say", vec![unquoted(vec![var("outer")])])),
            assign(
                "inner",
                unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "env",
                    vec![
                        lit(&format!("{UNIQUE}=for-one-command")),
                        lit("sh"),
                        lit("-c"),
                        quoted(vec![text(&format!("printf %s \"${UNIQUE}\""))]),
                    ],
                ))))]),
            ),
            run(cmd("say", vec![unquoted(vec![var("inner")])])),
        ],
    )]);
    assert_eq!(
        go(&f, "t", &[]).printed(),
        ["from-the-task", "for-one-command"]
    );
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
// --dry: a value the preview invented
//
// A capture the preview could not evaluate binds the empty string, and every
// later decision on that variable is decidable — and wrong. The preview does
// not take a different branch, because there is nothing to decide on; it says
// the decision was made on a value it made up. See the interpreter's module
// docs.
// ---------------------------------------------------------------------------

/// `v=$(read <missing>)` — the plainest capture `--dry` cannot evaluate, and
/// the one that needs no `PATH` program.
fn missing_capture(var: &str, path: &str) -> Stmt {
    assign(
        var,
        unquoted(vec![part(PartKind::Capture(Box::new(cmd(
            "read",
            vec![lit(path)],
        ))))]),
    )
}

#[test]
fn dry_explains_a_decision_made_on_an_invented_value() {
    // The shape from the bug report: the capture is skipped, `$size` is empty
    // only because of that, and `if $size == ""` is a perfectly decidable
    // comparison that walks into `fail`.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            missing_capture("size", "wasm/out.wasm"),
            if_stmt(
                compare(unquoted(vec![var("size")]), CompareOp::Eq, quoted(vec![])),
                vec![run(cmd("fail", vec![lit("wasm missing or empty")]))],
                None,
            ),
            run(cmd("hit", vec![lit("unreached")])),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());

    // The branch is exactly the one the preview took before: `fail` still
    // aborts, and nothing after it previews.
    assert!(ran.result.is_err(), "`fail` is still a hard stop");
    assert_eq!(trace(), Vec::<String>::new());

    assert!(
        ran.err.contains(
            "--dry: took the `then` branch on `$size`, a value this preview invented \
             because it could not evaluate `read wasm/out.wasm`; a real run may go the other way"
        ),
        "{}",
        ran.err
    );
    // And the `fail` says it was walked into rather than chosen.
    assert!(
        ran.err.contains(
            "--dry: this `fail` is inside the `then` branch, which was chosen on `$size` \
             — a value this preview invented, so a real run may never reach it"
        ),
        "{}",
        ran.err
    );
}

#[test]
fn dry_notes_go_to_stderr_never_stdout() {
    // stdout may be a capture's value; a note is a fact about the preview.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            missing_capture("size", "out.wasm"),
            if_stmt(
                compare(unquoted(vec![var("size")]), CompareOp::Eq, quoted(vec![])),
                vec![run(cmd("say", vec![lit("empty")]))],
                None,
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(ran.printed(), ["empty"]);
    assert!(!ran.out.contains("--dry: took"), "{}", ran.out);
    assert!(
        ran.err.contains("--dry: took the `then` branch"),
        "{}",
        ran.err
    );
}

#[test]
fn the_mark_travels_through_a_second_assignment() {
    // `label` never touched the capture; it only read something that did.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            missing_capture("size", "out.wasm"),
            assign("label", quoted(vec![text("size is "), var("size")])),
            if_stmt(
                compare(
                    unquoted(vec![var("label")]),
                    CompareOp::Eq,
                    quoted(vec![text("size is ")]),
                ),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["then"]);
    // Named for the variable the condition read, and blamed on the command
    // whose answer is missing two assignments back.
    assert!(
        ran.err.contains(
            "took the `then` branch on `$label`, a value this preview invented \
             because it could not evaluate `read out.wasm`"
        ),
        "{}",
        ran.err
    );
}

#[test]
fn an_ordinary_assignment_clears_the_mark() {
    // The mark describes the value the variable holds, not the history of the
    // name.
    let f = file(vec![task(
        "t",
        &[],
        vec![
            missing_capture("size", "out.wasm"),
            assign("size", lit("4096")),
            if_stmt(
                compare(unquoted(vec![var("size")]), CompareOp::Eq, quoted(vec![])),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["else"]);
    assert!(!ran.err.contains("took the"), "{}", ran.err);
}

#[test]
fn a_condition_on_a_value_that_was_never_invented_says_nothing() {
    let dir = Temp::new("dry-untainted");
    std::fs::write(dir.path().join("size.txt"), "4096").unwrap();
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign(
                "size",
                unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "read",
                    vec![lit("size.txt")],
                ))))]),
            ),
            if_stmt(
                compare(unquoted(vec![var("size")]), CompareOp::Eq, quoted(vec![])),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, dir.path());
    ran.ok();
    assert_eq!(trace(), ["else"]);
    assert_eq!(ran.err, "");
}

#[test]
fn run_mode_carries_no_mark_at_all() {
    let dir = Temp::new("dry-run-mode");
    std::fs::write(dir.path().join("size.txt"), "4096").unwrap();
    let f = file(vec![task(
        "t",
        &[],
        vec![
            assign(
                "size",
                unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                    "read",
                    vec![lit("size.txt")],
                ))))]),
            ),
            assign("label", quoted(vec![text("size is "), var("size")])),
            if_stmt(
                compare(unquoted(vec![var("label")]), CompareOp::Eq, quoted(vec![])),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    ran.ok();
    assert_eq!(trace(), ["else"]);
    assert_eq!(ran.err, "", "a run has nothing to explain");
    assert!(!ran.out.contains("--dry"), "{}", ran.out);
}

#[test]
fn the_note_is_printed_once_however_often_the_decision_repeats() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            missing_capture("size", "out.wasm"),
            Stmt::For(For {
                var: "item".into(),
                items: vec![lit("a"), lit("b"), lit("c")],
                body: vec![if_stmt(
                    compare(unquoted(vec![var("size")]), CompareOp::Eq, quoted(vec![])),
                    vec![run(cmd("hit", vec![unquoted(vec![var("item")])]))],
                    None,
                )],
                span: sp(),
            }),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    // The loop still runs three times; only the explanation is said once.
    assert_eq!(trace(), ["a", "b", "c"]);
    assert_eq!(
        ran.err.matches("--dry: took the `then` branch").count(),
        1,
        "{}",
        ran.err
    );
}

#[test]
fn a_loop_variable_taken_from_an_invented_list_is_marked_too() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            missing_capture("names", "names.txt"),
            Stmt::For(For {
                var: "item".into(),
                items: vec![unquoted(vec![var("names")])],
                body: vec![if_stmt(
                    compare(unquoted(vec![var("item")]), CompareOp::Eq, lit("x")),
                    vec![run(cmd("hit", vec![lit("then")]))],
                    Some(vec![run(cmd("hit", vec![lit("else")]))]),
                )],
                span: sp(),
            }),
            // The list was empty, so the body never ran: the `for` header
            // itself is where the invented value showed.
            if_stmt(
                compare(unquoted(vec![var("names")]), CompareOp::Eq, quoted(vec![])),
                vec![run(cmd("hit", vec![lit("empty")]))],
                None,
            ),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["empty"]);
    assert!(
        ran.err.contains("took the `then` branch on `$names`"),
        "{}",
        ran.err
    );
}

#[test]
fn the_mark_reaches_a_called_task_through_its_arguments() {
    // The callee never saw the capture; it was handed the value.
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                missing_capture("size", "out.wasm"),
                run(cmd("check", vec![unquoted(vec![var("size")])])),
            ],
        ),
        with_params(
            "check",
            vec![optional("s", lit(""))],
            vec![if_stmt(
                compare(unquoted(vec![pos(1)]), CompareOp::Eq, quoted(vec![])),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            )],
        ),
    ]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["then"]);
    assert!(
        ran.err.contains(
            "took the `then` branch on `$1`, a value this preview invented \
             because it could not evaluate `read out.wasm`"
        ),
        "{}",
        ran.err
    );
}

#[test]
fn the_mark_reaches_a_called_task_through_dollar_at() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                missing_capture("size", "out.wasm"),
                run(cmd("check", vec![unquoted(vec![var("size")])])),
            ],
        ),
        task(
            "check",
            &[],
            vec![if_stmt(
                compare(
                    unquoted(vec![part(PartKind::Var(VarRef::All))]),
                    CompareOp::Eq,
                    quoted(vec![]),
                ),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            )],
        ),
    ]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["then"]);
    assert!(ran.err.contains("branch on `$@`"), "{}", ran.err);
}

#[test]
fn a_marked_variable_does_not_leak_out_of_the_task_that_made_it() {
    // The mark lives in the frame, so it dies with it: the caller's `$size`
    // is its own.
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                assign("size", lit("4096")),
                run(cmd("inner", vec![])),
                if_stmt(
                    compare(unquoted(vec![var("size")]), CompareOp::Eq, lit("4096")),
                    vec![run(cmd("hit", vec![lit("then")]))],
                    None,
                ),
            ],
        ),
        task("inner", &[], vec![missing_capture("size", "out.wasm")]),
    ]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    ran.ok();
    assert_eq!(trace(), ["then"]);
    assert!(!ran.err.contains("took the"), "{}", ran.err);
}

#[test]
fn a_fail_the_author_chose_gets_no_extra_note() {
    let f = file(vec![task(
        "t",
        &[],
        vec![
            missing_capture("size", "out.wasm"),
            if_stmt(
                compare(unquoted(vec![var("size")]), CompareOp::Ne, quoted(vec![])),
                vec![run(cmd("hit", vec![lit("unreached")]))],
                None,
            ),
            // Outside the branch chosen on `$size`: this one is the author's.
            run(cmd("fail", vec![lit("deliberate")])),
        ],
    )]);
    let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
    assert!(ran.result.is_err());
    assert!(ran.err.contains("took no branch on `$size`"), "{}", ran.err);
    assert!(!ran.err.contains("this `fail` is inside"), "{}", ran.err);
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

// ---------------------------------------------------------------------------
// Included tasks (`include ... as`)
// ---------------------------------------------------------------------------
//
// `resolve` merges every included file into one tree before the interpreter
// sees it, so a task pulled in under `include libs/chorefile as libs` arrives
// here simply *named* `libs::build`. These build that merged tree by hand —
// the interpreter is the unit under test, and it must not need `resolve` to
// exist to be tested. What is being pinned down is that the whole namespaced
// name, and nothing shorter, is what the interpreter looks up, keys and
// reports.

#[test]
fn a_namespaced_task_runs_as_a_statement() {
    let f = file(vec![
        task("t", &[], vec![run(cmd("libs::build", vec![]))]),
        task(
            "libs::build",
            &[],
            vec![run(cmd("hit", vec![lit("built")]))],
        ),
    ]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["built"]);
}

#[test]
fn a_namespaced_task_is_callable_from_the_command_line() {
    // `chore libs::build` hands the whole name to `run_task`.
    let f = file(vec![task(
        "libs::build",
        &[],
        vec![run(cmd("hit", vec![lit("built")]))],
    )]);
    go(&f, "libs::build", &[]).ok();
    assert_eq!(trace(), ["built"]);
}

#[test]
fn a_namespaced_task_captures_pipes_and_redirects() {
    let dir = Temp::new("ns-capture");
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                // $( ... )
                assign(
                    "v",
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                        "libs::version",
                        vec![],
                    ))))]),
                ),
                run(cmd("say", vec![unquoted(vec![text("got "), var("v")])])),
                // A pipe.
                run(Chain::Pipe(
                    Box::new(cmd("libs::version", vec![])),
                    Box::new(cmd("upper", vec![])),
                )),
                // A redirect.
                run(cmd_with(
                    "libs::version",
                    vec![],
                    vec![redirect(RedirectKind::Stdout, "v.txt")],
                )),
                run(cmd("read", vec![lit("v.txt")])),
            ],
        ),
        task(
            "libs::version",
            &[],
            vec![run(cmd("say", vec![lit("1.2.3")]))],
        ),
    ]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    assert_eq!(ran.printed(), ["got 1.2.3", "1.2.3", "1.2.3"]);
}

#[test]
fn same_named_tasks_in_different_namespaces_both_run() {
    // The very collision `as` exists to prevent: two `build`s that must stay
    // two tasks. A key that dropped the namespace would run one and skip the
    // other, silently.
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("libs::build", vec![])),
                run(cmd("tools::build", vec![])),
                run(cmd("build", vec![])),
            ],
        ),
        task("libs::build", &[], vec![run(cmd("hit", vec![lit("libs")]))]),
        task(
            "tools::build",
            &[],
            vec![run(cmd("hit", vec![lit("tools")]))],
        ),
        task("build", &[], vec![run(cmd("hit", vec![lit("bare")]))]),
    ]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["libs", "tools", "bare"]);
}

#[test]
fn run_once_is_keyed_per_namespace() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("libs::build", vec![])),
                run(cmd("tools::build", vec![])),
                // Each is already done; neither may run a second time, and
                // the second call must not be answered by the first's record.
                run(cmd("libs::build", vec![])),
                run(cmd("tools::build", vec![])),
            ],
        ),
        task("libs::build", &[], vec![run(cmd("hit", vec![lit("libs")]))]),
        task(
            "tools::build",
            &[],
            vec![run(cmd("hit", vec![lit("tools")]))],
        ),
    ]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["libs", "tools"]);
}

#[test]
fn a_replayed_capture_is_keyed_per_namespace_too() {
    // Two namespaced tasks used as functions: the remembered value belongs to
    // the one that printed it.
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                assign(
                    "a",
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                        "libs::id",
                        vec![],
                    ))))]),
                ),
                assign(
                    "b",
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                        "tools::id",
                        vec![],
                    ))))]),
                ),
                assign(
                    "a2",
                    unquoted(vec![part(PartKind::Capture(Box::new(cmd(
                        "libs::id",
                        vec![],
                    ))))]),
                ),
                run(cmd(
                    "say",
                    vec![unquoted(vec![
                        var("a"),
                        text(" "),
                        var("b"),
                        text(" "),
                        var("a2"),
                    ])],
                )),
            ],
        ),
        task("libs::id", &[], vec![run(cmd("say", vec![lit("libs")]))]),
        task("tools::id", &[], vec![run(cmd("say", vec![lit("tools")]))]),
    ]);
    let ran = go(&f, "t", &[]);
    assert_eq!(ran.printed(), ["libs tools libs"]);
}

#[test]
fn task_variable_holds_the_namespaced_name() {
    // The name `chore list` shows and the name that has to be typed to run
    // it. Answering `build` would name something uncallable.
    let f = file(vec![
        task("t", &[], vec![run(cmd("libs::build", vec![]))]),
        task(
            "libs::build",
            &[],
            vec![run(cmd("say", vec![unquoted(vec![var("TASK")])]))],
        ),
    ]);
    assert_eq!(go(&f, "t", &[]).printed(), ["libs::build"]);
}

#[test]
fn a_namespaced_task_calls_a_bare_sibling() {
    // Merging is flat, so a task in a namespace reaches a top-level task by
    // its plain name — and its own name is no prefix on the lookup.
    let f = file(vec![
        task(
            "libs::build",
            &[],
            vec![
                run(cmd("prepare", vec![])),
                run(cmd("hit", vec![lit("built")])),
            ],
        ),
        task("prepare", &[], vec![run(cmd("hit", vec![lit("prepared")]))]),
    ]);
    go(&f, "libs::build", &[]).ok();
    assert_eq!(trace(), ["prepared", "built"]);
}

#[test]
fn a_namespaced_task_takes_arguments_and_reports_a_missing_one_by_full_name() {
    let f = file(vec![
        task(
            "t",
            &[],
            vec![run(cmd("libs::build", vec![lit("release")]))],
        ),
        task(
            "libs::build",
            &["profile"],
            vec![run(cmd(
                "hit",
                vec![unquoted(vec![part(PartKind::Var(VarRef::Positional(1)))])],
            ))],
        ),
    ]);
    go(&f, "t", &[]).ok();
    assert_eq!(trace(), ["release"]);

    let missing = file(vec![
        task("t", &[], vec![run(cmd("libs::build", vec![]))]),
        task("libs::build", &["profile"], vec![]),
    ]);
    let message = go(&missing, "t", &[]).err_text();
    assert!(message.contains("libs::build"), "{message}");
}

#[test]
fn an_unknown_namespaced_task_is_reported_by_full_name() {
    let f = file(vec![task("t", &[], vec![])]);
    let message = go(&f, "libs::build", &[]).err_text();
    assert!(message.contains("unknown task `libs::build`"), "{message}");
}

#[test]
fn a_namespaced_task_recursing_still_hits_the_depth_guard() {
    // On a stack the size `chore` itself uses. The guard allows 128 nested
    // calls, which does not fit the stack a test harness thread gets on
    // Windows -- the process overflowed before the guard could report. The
    // binary spawns its work on a 32 MB stack for exactly this reason, so a
    // test of the guard has to stand where the binary stands.
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            // The guard counts frames, so the shape of the name cannot slip
            // past it.
            let f = file(vec![task(
                "libs::build",
                &[],
                vec![assign("n", lit("x")), run(cmd("libs::build", vec![]))],
            )]);
            // --force, or run-once would stop the second call before the
            // guard could.
            let ran = exec(&f, "libs::build", &[], Mode::Run, Repeat::Always, &root());
            ran.err_text()
        })
        .expect("spawn");
    let message = handle
        .join()
        .expect("the guard should report, not overflow");
    assert!(message.contains("libs::build"), "{message}");
    assert!(message.contains("recursed"), "{message}");
}

#[test]
fn root_stays_the_top_level_directory_inside_an_included_task() {
    // The rule `include` is most likely to break by accident: a relative path
    // written in an included file belongs under the *project's* root, not
    // beside the file that happened to contain it. The interpreter is handed
    // one root for the run and every task sees it, wherever it came from.
    let dir = Temp::new("ns-root");
    let f = file(vec![
        task(
            "t",
            &[],
            vec![
                run(cmd("say", vec![unquoted(vec![var("ROOT")])])),
                run(cmd("libs::fetch", vec![])),
            ],
        ),
        task(
            "libs::fetch",
            &[],
            vec![
                run(cmd("say", vec![unquoted(vec![var("ROOT")])])),
                // A relative path in the included task resolves against the
                // run's directory, which starts at that same root.
                run(cmd("touch", vec![unquoted(vec![text("third_party.txt")])])),
            ],
        ),
    ]);
    let ran = exec(&f, "t", &[], Mode::Run, Repeat::Once, dir.path());
    let root = chorefile::vars::display(dir.path());
    assert_eq!(ran.printed(), [root.clone(), root]);
    assert!(dir.path().join("third_party.txt").is_file());
}

#[test]
fn root_cannot_be_reassigned_by_a_global_or_a_task() {
    // A merged tree carries every included file's globals, so an included
    // `ROOT=...` would otherwise move the root for the whole run — and only
    // for `$ROOT`, since the builtins read the interpreter's own field. One
    // root per invocation, and nothing in the file may move it.
    let dir = Temp::new("ns-root-fixed");
    let f = File {
        require: None,
        includes: Vec::new(),
        globals: vec![Assign {
            name: "ROOT".into(),
            value: lit("/somewhere/else"),
            span: sp(),
        }],
        tasks: vec![task(
            "libs::fetch",
            &[],
            vec![
                run(cmd("say", vec![unquoted(vec![var("ROOT")])])),
                assign("ROOT", lit("/elsewhere")),
                run(cmd("say", vec![unquoted(vec![var("ROOT")])])),
            ],
        )],
    };
    let ran = exec(&f, "libs::fetch", &[], Mode::Run, Repeat::Once, dir.path());
    let root = chorefile::vars::display(dir.path());
    assert_eq!(ran.printed(), [root.clone(), root]);
}

#[test]
fn a_namespaced_name_echoes_unquoted() {
    // `::` is not whitespace and not a quote, so the echo line reads as the
    // command the author wrote.
    let f = file(vec![
        task(
            "t",
            &[],
            vec![run(cmd("libs::build", vec![lit("release")]))],
        ),
        task("libs::build", &[], vec![]),
    ]);
    assert_eq!(go(&f, "t", &[]).echoed(), ["$ libs::build release"]);
}

// ---------------------------------------------------------------------------
// `script` blocks
//
// The command is a real program on `PATH` — there is no way to hand a block of
// text to a builtin — so these tests need one that is guaranteed present and
// reads its input from stdin. On unix that is `sh` and `cat`; there is no
// equivalent pair to rely on elsewhere, so the section is unix-only.
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod script_blocks {
    use super::*;

    fn script_chain(command: Vec<Word>, body: &str) -> Chain {
        script_with(command, body, Vec::new())
    }

    fn script_with(command: Vec<Word>, body: &str, redirects: Vec<Redirect>) -> Chain {
        Chain::Script(Script {
            command,
            body: body.into(),
            redirects,
            span: sp(),
            body_span: sp(),
        })
    }

    /// A block on a line of its own — the plainest of the places one can now
    /// appear, and a statement like any other command statement.
    fn script(command: Vec<Word>, body: &str) -> Stmt {
        run(script_chain(command, body))
    }

    /// `$( <chain> )` as a word part, for capturing a block.
    fn capture(chain: Chain) -> WordPart {
        part(PartKind::Capture(Box::new(chain)))
    }

    /// `sh -c "cat > <path>"`: the block reaches `cat` on stdin and lands in a
    /// file, which is the only way a test can read bytes a streamed command
    /// wrote.
    fn write_body_to(path: &Path) -> Vec<Word> {
        vec![
            lit("sh"),
            lit("-c"),
            quoted(vec![text(&format!(
                "cat > {}",
                chorefile::vars::display(path)
            ))]),
        ]
    }

    /// The block's own text is the program `sh` runs, so whatever it prints is
    /// the block's stdout — which is what a capture, a pipe or a `>` reads.
    fn sh() -> Vec<Word> {
        vec![lit("sh")]
    }

    fn shown(path: &Path) -> String {
        chorefile::vars::display(path)
    }

    // -- the block reaches the interpreter ---------------------------------

    /// The documented way a chore value reaches a block: `env` sets it, and
    /// the block's interpreter is spawned with it. The set is per-call now, so
    /// this is the test that the call it is inside is the one that spawns.
    #[test]
    fn a_block_is_spawned_with_what_env_set() {
        let dir = Temp::new("script-env");
        let out = dir.path().join("target.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![
                set("CHOREFILE_TEST_TARGET", "aarch64-apple-darwin"),
                script(
                    sh(),
                    &format!("printf %s \"$CHOREFILE_TEST_TARGET\" > {}\n", shown(&out)),
                ),
            ],
        )]);
        go(&f, "t", &[]).ok();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "aarch64-apple-darwin"
        );
    }

    #[test]
    fn the_body_reaches_the_interpreter_on_stdin() {
        let dir = Temp::new("script-stdin");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![script(write_body_to(&out), "hello from the block\n")],
        )]);
        let ran = go(&f, "t", &[]);
        ran.ok();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "hello from the block\n"
        );
    }

    #[test]
    fn the_body_arrives_byte_for_byte_with_no_interpolation() {
        // Quotes, `$var`, `$(...)` and a backslash: every one of them means
        // something to chore in a word and nothing at all in a block. If any
        // of it were expanded on the way out, the block would reach the
        // interpreter saying something its author did not write.
        let dir = Temp::new("script-verbatim");
        let out = dir.path().join("out.txt");
        let body = "print(\"$HOME\", '$(rm -rf /)', $count, \"a\\tb\")\n#\u{00e9}\n";
        let f = file(vec![task(
            "t",
            &[],
            vec![script(write_body_to(&out), body)],
        )]);
        go(&f, "t", &[]).ok();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), body);
    }

    #[test]
    fn the_echo_names_the_command_and_counts_the_lines_but_never_shows_them() {
        let dir = Temp::new("script-echo");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![script(write_body_to(&out), "one\ntwo\nthree\n")],
        )]);
        let ran = go(&f, "t", &[]);
        let echoed = ran.echoed();
        assert_eq!(echoed.len(), 1);
        assert!(echoed[0].starts_with("$ script sh -c "), "{echoed:?}");
        assert!(echoed[0].ends_with("(3 lines on stdin)"), "{echoed:?}");
        assert!(!ran.ok().contains("two"), "the body must not be echoed");
    }

    #[test]
    fn the_command_is_expanded_like_any_other_command() {
        // `$var` in the argv still interpolates: only the body is raw.
        let dir = Temp::new("script-expand");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &["dest"],
            vec![
                assign("SH", lit("sh")),
                script(
                    vec![
                        unquoted(vec![var("SH")]),
                        lit("-c"),
                        quoted(vec![
                            text("cat > "),
                            part(PartKind::Var(VarRef::Positional(1))),
                        ]),
                    ],
                    "expanded\n",
                ),
            ],
        )]);
        let ran = exec(&f, "t", &[&shown(&out)], Mode::Run, Repeat::Once, &root());
        ran.ok();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "expanded\n");
    }

    #[test]
    fn a_missing_interpreter_says_it_came_from_a_script_block() {
        let f = file(vec![task(
            "t",
            &[],
            vec![script(
                vec![lit("chore-no-such-interpreter"), lit("run")],
                "print(1)\n",
            )],
        )]);
        let message = go(&f, "t", &[]).err_text();
        assert!(message.contains("chore-no-such-interpreter"), "{message}");
        assert!(message.contains("script"), "{message}");
        assert!(message.contains("PATH"), "{message}");
    }

    #[test]
    fn a_large_body_does_not_hang() {
        // The child writes as it reads, so a body bigger than a pipe buffer
        // deadlocks anything that writes the whole of it before waiting.
        let dir = Temp::new("script-large");
        let out = dir.path().join("out.txt");
        let body = "0123456789abcdef".repeat(24 * 1024); // ~384 KiB
        let f = file(vec![task(
            "t",
            &[],
            vec![script(
                vec![
                    lit("sh"),
                    lit("-c"),
                    quoted(vec![text(&format!("cat | cat > {}", shown(&out)))]),
                ],
                &body,
            )],
        )]);
        go(&f, "t", &[]).ok();
        assert_eq!(std::fs::metadata(&out).unwrap().len() as usize, body.len());
    }

    // -- captured ----------------------------------------------------------

    #[test]
    fn a_capture_binds_what_the_block_printed() {
        // The headline case: compute a value in another language and use it in
        // the task. Trimmed like any other capture, and echoed like any other
        // capture — which is to say not at all, because a capture is machinery
        // rather than a step of the recipe.
        let f = file(vec![task(
            "t",
            &[],
            vec![
                assign("v", unquoted(vec![capture(script_chain(sh(), "echo 7\n"))])),
                run(cmd("say", vec![quoted(vec![text("v="), var("v")])])),
            ],
        )]);
        let ran = go(&f, "t", &[]);
        assert_eq!(ran.printed(), ["v=7"]);
        assert_eq!(ran.echoed(), ["$ say v=7"]);
    }

    #[test]
    fn a_captured_body_is_still_raw() {
        // The same guarantee as on a bare statement, in the position where it
        // is easiest to lose: a `$HOME` here is not a chore variable, and a
        // chore that expanded it would fail on the undefined name rather than
        // hand the text to `sh`. The single quotes are `sh`'s, so `sh` does not
        // expand it either — what comes back is exactly what was written.
        let f = file(vec![task(
            "t",
            &[],
            vec![
                assign(
                    "v",
                    unquoted(vec![capture(script_chain(
                        sh(),
                        "printf '%s' '$HOME \"quoted\" $(rm -rf /)'\n",
                    ))]),
                ),
                run(cmd("say", vec![quoted(vec![var("v")])])),
            ],
        )]);
        assert_eq!(go(&f, "t", &[]).printed(), ["$HOME \"quoted\" $(rm -rf /)"]);
    }

    #[test]
    fn a_capture_of_a_failing_block_fails_the_run() {
        // `Mode::Run`: an empty value here would be wrong rather than merely
        // unknown, the same as for any other capture.
        let f = file(vec![task(
            "t",
            &[],
            vec![assign(
                "v",
                unquoted(vec![capture(script_chain(sh(), "exit 3\n"))]),
            )],
        )]);
        let message = go(&f, "t", &[]).err_text();
        assert!(message.contains("capture failed"), "{message}");
    }

    // -- piped -------------------------------------------------------------

    #[test]
    fn a_block_pipes_its_stdout_on() {
        let f = file(vec![task(
            "t",
            &[],
            vec![run(Chain::Pipe(
                Box::new(script_chain(sh(), "echo hi\n")),
                Box::new(cmd("upper", vec![])),
            ))],
        )]);
        assert_eq!(go(&f, "t", &[]).printed(), ["HI"]);
    }

    #[test]
    fn a_pipe_into_a_block_is_refused_before_its_left_side_runs() {
        // Two candidate stdins, one slot. The block wins — it is the program
        // being run — which leaves the pipe's bytes nowhere to go, so the
        // pipeline is refused rather than half-honoured. Refused *early*: the
        // left side has not run, so nothing was done and then thrown away.
        let dir = Temp::new("script-pipe-in");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![run(Chain::Pipe(
                Box::new(cmd("hit", vec![lit("left")])),
                Box::new(script_chain(write_body_to(&out), "x\n")),
            ))],
        )]);
        let message = go(&f, "t", &[]).err_text();
        assert!(message.contains("right of a `|`"), "{message}");
        assert!(message.contains("script sh"), "{message}");
        assert!(trace().is_empty(), "the left side of the pipe must not run");
        assert!(!out.exists(), "the block must not run either");
    }

    #[test]
    fn the_refusal_finds_the_block_that_would_have_been_fed() {
        // A pipe's bytes go to the leftmost thing that runs on the right, so
        // `a | script { } && b` is refused and `a | b && script { }` is not:
        // the block in the second is not the command reading the pipe.
        let fed = file(vec![task(
            "t",
            &[],
            vec![run(Chain::Pipe(
                Box::new(cmd("hit", vec![lit("left")])),
                Box::new(Chain::And(
                    Box::new(script_chain(sh(), "true\n")),
                    Box::new(cmd("hit", vec![lit("after")])),
                )),
            ))],
        )]);
        assert!(go(&fed, "t", &[]).err_text().contains("right of a `|`"));

        // The block runs, and its stdin is its own body — the pipe's bytes
        // reached `upper` and stopped there. It streams to the real stdout, so
        // the file is what the test can see.
        let dir = Temp::new("script-unfed");
        let out = dir.path().join("out.txt");
        let unfed = file(vec![task(
            "t",
            &[],
            vec![run(Chain::Pipe(
                Box::new(cmd("say", vec![lit("left")])),
                Box::new(Chain::And(
                    Box::new(cmd("upper", vec![])),
                    Box::new(script_chain(write_body_to(&out), "after\n")),
                )),
            ))],
        )]);
        assert_eq!(go(&unfed, "t", &[]).printed(), ["LEFT"]);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "after\n");
    }

    // -- redirected --------------------------------------------------------

    #[test]
    fn a_block_redirects_its_stdout() {
        let dir = Temp::new("script-redirect");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![run(script_with(
                sh(),
                "echo one\n",
                vec![redirect(RedirectKind::Stdout, &shown(&out))],
            ))],
        )]);
        let ran = go(&f, "t", &[]);
        ran.ok();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "one\n");
        // The redirection is echoed, and the stdin note stays last.
        let echoed = ran.echoed();
        assert!(
            echoed[0].contains(&format!("> {}", shown(&out))),
            "{echoed:?}"
        );
        assert!(echoed[0].ends_with("(1 line on stdin)"), "{echoed:?}");
    }

    #[test]
    fn a_block_appends_with_a_double_arrow() {
        let dir = Temp::new("script-append");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![
                run(script_with(
                    sh(),
                    "echo one\n",
                    vec![redirect(RedirectKind::Stdout, &shown(&out))],
                )),
                run(script_with(
                    sh(),
                    "echo two\n",
                    vec![redirect(RedirectKind::StdoutAppend, &shown(&out))],
                )),
            ],
        )]);
        go(&f, "t", &[]).ok();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn a_block_redirects_its_stderr() {
        let dir = Temp::new("script-stderr");
        let err = dir.path().join("err.txt");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![run(script_with(
                sh(),
                "echo kept; echo diagnostic >&2\n",
                vec![
                    redirect(RedirectKind::Stdout, &shown(&out)),
                    redirect(RedirectKind::Stderr, &shown(&err)),
                ],
            ))],
        )]);
        go(&f, "t", &[]).ok();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "kept\n");
        assert_eq!(std::fs::read_to_string(&err).unwrap(), "diagnostic\n");
    }

    // -- chained -----------------------------------------------------------

    #[test]
    fn a_nonzero_interpreter_fails_the_run() {
        let f = file(vec![task(
            "t",
            &[],
            vec![
                script(sh(), "exit 3\n"),
                run(cmd("hit", vec![lit("unreachable")])),
            ],
        )]);
        let ran = go(&f, "t", &[]);
        let message = ran.err_text();
        assert!(message.contains("exited with code 3"), "{message}");
        assert!(message.contains("script sh"), "{message}");
        assert!(trace().is_empty(), "the statement after it must not run");
    }

    #[test]
    fn and_carries_on_only_when_the_block_succeeded() {
        let ok = file(vec![task(
            "t",
            &[],
            vec![run(Chain::And(
                Box::new(script_chain(sh(), "exit 0\n")),
                Box::new(cmd("hit", vec![lit("after")])),
            ))],
        )]);
        go(&ok, "t", &[]).ok();
        assert_eq!(trace(), ["after"]);

        let bad = file(vec![task(
            "t",
            &[],
            vec![run(Chain::And(
                Box::new(script_chain(sh(), "exit 3\n")),
                Box::new(cmd("hit", vec![lit("after")])),
            ))],
        )]);
        let message = go(&bad, "t", &[]).err_text();
        assert!(message.contains("exited with code 3"), "{message}");
        assert!(trace().is_empty(), "`&&` must not reach the right side");
    }

    #[test]
    fn or_takes_over_when_the_block_failed() {
        let f = file(vec![task(
            "t",
            &[],
            vec![run(Chain::Or(
                Box::new(script_chain(sh(), "exit 3\n")),
                Box::new(cmd("hit", vec![lit("fallback")])),
            ))],
        )]);
        go(&f, "t", &[]).ok();
        assert_eq!(trace(), ["fallback"]);
    }

    #[test]
    fn try_swallows_a_failing_block() {
        let f = file(vec![task(
            "t",
            &[],
            vec![
                Stmt::Try(script_chain(sh(), "exit 3\n")),
                run(cmd("say", vec![lit("carried on")])),
            ],
        )]);
        assert_eq!(go(&f, "t", &[]).printed(), ["carried on"]);
    }

    #[test]
    fn try_around_the_task_swallows_a_failing_block() {
        // The other way round: a block inside a task the caller forgives. It
        // works for the same reason `try` directly around the block does — the
        // block fails the way any command does.
        let f = file(vec![
            task(
                "t",
                &[],
                vec![
                    Stmt::Try(cmd("risky", vec![])),
                    run(cmd("say", vec![lit("carried on")])),
                ],
            ),
            task("risky", &[], vec![script(sh(), "exit 3\n")]),
        ]);
        let ran = go(&f, "t", &[]);
        assert_eq!(ran.printed(), ["carried on"]);
    }

    #[test]
    fn a_block_is_an_if_condition() {
        let build = |body: &str| {
            file(vec![task(
                "t",
                &[],
                vec![if_stmt(
                    Cond::Command(script_chain(sh(), body)),
                    vec![run(cmd("hit", vec![lit("then")]))],
                    Some(vec![run(cmd("hit", vec![lit("else")]))]),
                )],
            )])
        };

        let yes = build("exit 0\n");
        let ran = go(&yes, "t", &[]);
        ran.ok();
        assert_eq!(trace(), ["then"]);
        // A condition is machinery: the block echoes no more there than
        // `if which cargo` does.
        let echoed = ran.echoed();
        assert!(
            !echoed.iter().any(|l| l.starts_with("$ script")),
            "{echoed:?}"
        );

        let no = build("exit 3\n");
        go(&no, "t", &[]).ok();
        assert_eq!(trace(), ["else"]);
    }

    // -- `--dry` -----------------------------------------------------------

    #[test]
    fn dry_reports_the_block_and_does_not_run_it() {
        let dir = Temp::new("script-dry");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![script(write_body_to(&out), "one\ntwo\n")],
        )]);
        let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
        ran.ok();
        assert!(!out.exists(), "--dry must not run a script block");
        assert_eq!(ran.echoed().len(), 1);
        let note = ran.printed().join("\n");
        assert!(note.contains("skipped by --dry"), "{note}");
    }

    #[test]
    fn a_dry_capture_of_a_block_is_the_empty_string() {
        // The existing rule for a capture a preview could not evaluate: the
        // empty string and a note, never an abort. The note goes to stderr
        // here rather than stdout, because a capture's stdout is a value
        // somebody is about to read.
        let dir = Temp::new("script-dry-capture");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![
                assign(
                    "v",
                    unquoted(vec![capture(script_chain(write_body_to(&out), "echo 7\n"))]),
                ),
                run(cmd(
                    "say",
                    vec![quoted(vec![text("v=["), var("v"), text("]")])],
                )),
            ],
        )]);
        let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
        ran.ok();
        assert_eq!(ran.printed(), ["v=[]"]);
        assert!(!out.exists(), "--dry must not run a captured block either");
        assert!(ran.err.contains("skipped"), "{}", ran.err);
        assert!(
            !ran.ok().contains("skipped"),
            "the note belongs on stderr here: {}",
            ran.ok()
        );
    }

    #[test]
    fn a_dry_redirect_of_a_block_writes_nothing() {
        let dir = Temp::new("script-dry-redirect");
        let out = dir.path().join("out.txt");
        let f = file(vec![task(
            "t",
            &[],
            vec![run(script_with(
                sh(),
                "echo one\n",
                vec![redirect(RedirectKind::Stdout, &shown(&out))],
            ))],
        )]);
        let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
        ran.ok();
        assert!(
            !out.exists(),
            "a skipped block must not create its `>` file"
        );
        assert!(
            ran.echoed()[0].contains(&format!("> {}", shown(&out))),
            "{:?}",
            ran.echoed()
        );
    }

    #[test]
    fn a_dry_condition_on_a_block_previews_the_then_branch() {
        // The block never ran, so there is no verdict: previewing the work the
        // author wrote beats previewing nothing.
        let f = file(vec![task(
            "t",
            &[],
            vec![if_stmt(
                Cond::Command(script_chain(sh(), "exit 3\n")),
                vec![run(cmd("hit", vec![lit("then")]))],
                Some(vec![run(cmd("hit", vec![lit("else")]))]),
            )],
        )]);
        exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root()).ok();
        assert_eq!(trace(), ["then"]);
    }

    #[test]
    fn a_decision_on_a_skipped_blocks_value_is_explained_not_guessed() {
        // The shape from the bug report, whole: the block is skipped, `$size`
        // is empty only because of that, and `if $size == ""` is a decidable
        // comparison of a variable that walks straight into `fail`. The
        // preview keeps that branch — there is nothing to decide on, and the
        // undecided rule would land in the same place — and says where the
        // value came from instead.
        let f = file(vec![task(
            "t",
            &[],
            vec![
                assign(
                    "size",
                    unquoted(vec![capture(script_chain(sh(), "echo 4096\n"))]),
                ),
                if_stmt(
                    compare(unquoted(vec![var("size")]), CompareOp::Eq, quoted(vec![])),
                    vec![run(cmd("fail", vec![lit("wasm missing or empty")]))],
                    None,
                ),
            ],
        )]);
        let ran = exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root());
        assert!(ran.result.is_err(), "`fail` is still a hard stop");
        assert!(
            ran.err.contains(
                "--dry: took the `then` branch on `$size`, a value this preview invented \
                 because it could not evaluate `script sh { ... }`; a real run may go the \
                 other way"
            ),
            "{}",
            ran.err
        );
        assert!(
            ran.err.contains("this `fail` is inside the `then` branch"),
            "{}",
            ran.err
        );
    }

    #[test]
    fn a_dry_block_does_not_stop_the_preview_around_it() {
        let f = file(vec![task(
            "t",
            &[],
            vec![
                run(Chain::And(
                    Box::new(script_chain(sh(), "exit 3\n")),
                    Box::new(cmd("hit", vec![lit("after")])),
                )),
                run(cmd("hit", vec![lit("last")])),
            ],
        )]);
        exec(&f, "t", &[], Mode::Dry, Repeat::Once, &root()).ok();
        // A skipped block answers like a skipped program on `PATH`, so the
        // rest of the recipe is still previewed.
        assert_eq!(trace(), ["after", "last"]);
    }
}
