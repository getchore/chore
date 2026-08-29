//! End-to-end parser tests.
//!
//! Most assertions go through [`render`], which prints a parsed node back as
//! source-like text with the structure made explicit: quoted words keep their
//! quotes, and every `&&`, `||`, `!` and comparison is parenthesised, so a
//! single string assertion pins down both the shape and the precedence.

use std::path::Path;

use chorefile::ast::*;
use chorefile::error::Error;
use chorefile::parse::parse;

fn file(source: &str) -> File {
    match parse(source, Path::new("chorefile")) {
        Ok(f) => f,
        Err(e) => panic!("expected a parse, got error: {e}"),
    }
}

/// The message of the syntax error `source` must produce.
fn error(source: &str) -> String {
    match parse(source, Path::new("chorefile")) {
        Ok(f) => panic!("expected a syntax error, parsed {f:?}"),
        Err(Error::Syntax { message, at }) => {
            assert_eq!(at.file, Path::new("chorefile"), "error lost its file");
            assert!(at.span.start <= at.span.end, "backwards span in {message}");
            assert!(
                at.span.end <= source.len(),
                "span {:?} outside source for {message}",
                at.span
            );
            message
        }
        Err(other) => panic!("expected a syntax error, got {other:?}"),
    }
}

fn body(source: &str) -> String {
    let f = file(source);
    let task = f.tasks.first().expect("no task parsed");
    render_block(&task.body)
}

// --- rendering ----------------------------------------------------------

fn render_block(block: &Block) -> String {
    let stmts: Vec<String> = block.iter().map(render_stmt).collect();
    format!("{{ {} }}", stmts.join("; "))
}

fn render_stmt(stmt: &Stmt) -> String {
    match stmt {
        Stmt::Assign(a) => format!("{}={}", a.name, render_word(&a.value)),
        Stmt::Command(c) => render_chain(c),
        Stmt::Try(c) => format!("try {}", render_chain(c)),
        Stmt::Exit(None) => "exit".into(),
        Stmt::Exit(Some(w)) => format!("exit {}", render_word(w)),
        Stmt::Return(None) => "return".into(),
        Stmt::Return(Some(w)) => format!("return {}", render_word(w)),
        Stmt::For(f) => {
            let items: Vec<String> = f.items.iter().map(render_word).collect();
            format!(
                "for {} in {} {}",
                f.var,
                items.join(" "),
                render_block(&f.body)
            )
        }
        Stmt::If(i) => render_if(i),
    }
}

/// The first `script` block anywhere in a block: inside `if` and `for`, and
/// inside a chain or a `$( )` capture, which is where one may now sit.
fn find_script(block: &Block) -> Option<&Script> {
    block.iter().find_map(script_in_stmt)
}

fn script_in_stmt(stmt: &Stmt) -> Option<&Script> {
    match stmt {
        Stmt::Assign(a) => script_in_word(&a.value),
        Stmt::Command(c) | Stmt::Try(c) => script_in_chain(c),
        Stmt::Exit(w) | Stmt::Return(w) => w.as_ref().and_then(script_in_word),
        Stmt::For(f) => f
            .items
            .iter()
            .find_map(script_in_word)
            .or_else(|| find_script(&f.body)),
        Stmt::If(i) => script_in_cond(&i.cond)
            .or_else(|| find_script(&i.then))
            .or_else(|| {
                i.otherwise
                    .as_ref()
                    .and_then(|otherwise| find_script(otherwise))
            }),
    }
}

fn script_in_chain(chain: &Chain) -> Option<&Script> {
    match chain {
        Chain::Script(s) => Some(s),
        Chain::Single(c) => std::iter::once(&c.name)
            .chain(&c.args)
            .chain(c.redirects.iter().map(|r| &r.target))
            .find_map(script_in_word),
        Chain::And(a, b) | Chain::Or(a, b) | Chain::Pipe(a, b) => {
            script_in_chain(a).or_else(|| script_in_chain(b))
        }
    }
}

fn script_in_cond(cond: &Cond) -> Option<&Script> {
    match cond {
        Cond::Compare { left, right, .. } => script_in_word(left).or_else(|| script_in_word(right)),
        Cond::Command(c) => script_in_chain(c),
        Cond::Not(c) => script_in_cond(c),
        Cond::And(a, b) | Cond::Or(a, b) => script_in_cond(a).or_else(|| script_in_cond(b)),
    }
}

fn script_in_word(word: &Word) -> Option<&Script> {
    word.parts.iter().find_map(|part| match &part.kind {
        PartKind::Capture(chain) => script_in_chain(chain),
        _ => None,
    })
}

fn render_if(i: &If) -> String {
    let head = format!("if {} {}", render_cond(&i.cond), render_block(&i.then));
    match &i.otherwise {
        Some(block) => format!("{head} else {}", render_block(block)),
        None => head,
    }
}

fn render_cond(cond: &Cond) -> String {
    match cond {
        Cond::Compare { left, op, right } => {
            let op = match op {
                CompareOp::Eq => "==",
                CompareOp::Ne => "!=",
                CompareOp::Contains => "contains",
                CompareOp::StartsWith => "starts-with",
                CompareOp::EndsWith => "ends-with",
            };
            format!("{} {op} {}", render_word(left), render_word(right))
        }
        Cond::Command(c) => render_chain(c),
        Cond::Not(c) => format!("!({})", render_cond(c)),
        Cond::And(a, b) => format!("({} && {})", render_cond(a), render_cond(b)),
        Cond::Or(a, b) => format!("({} || {})", render_cond(a), render_cond(b)),
    }
}

fn render_chain(chain: &Chain) -> String {
    match chain {
        Chain::Single(c) => render_command(c),
        Chain::Script(s) => render_script(s),
        Chain::And(a, b) => format!("({} && {})", render_chain(a), render_chain(b)),
        Chain::Or(a, b) => format!("({} || {})", render_chain(a), render_chain(b)),
        Chain::Pipe(a, b) => format!("({} | {})", render_chain(a), render_chain(b)),
    }
}

fn render_command(cmd: &Command) -> String {
    let mut out = String::new();
    if cmd.force_path {
        out.push('^');
    }
    out.push_str(&render_word(&cmd.name));
    for arg in &cmd.args {
        out.push(' ');
        out.push_str(&render_word(arg));
    }
    out.push_str(&render_redirects(&cmd.redirects));
    out
}

fn render_script(s: &Script) -> String {
    let command: Vec<String> = s.command.iter().map(render_word).collect();
    // The body is debug-printed, so a block's newlines stay on one line of
    // assertion and every space in them is visible.
    format!(
        "script {} {:?}{}",
        command.join(" "),
        s.body,
        render_redirects(&s.redirects)
    )
}

fn render_redirects(redirects: &[Redirect]) -> String {
    let mut out = String::new();
    for r in redirects {
        let op = match r.kind {
            RedirectKind::Stdout => ">",
            RedirectKind::StdoutAppend => ">>",
            RedirectKind::Stderr => "2>",
        };
        out.push_str(&format!(" {op} {}", render_word(&r.target)));
    }
    out
}

fn render_word(word: &Word) -> String {
    let mut out = String::new();
    for part in &word.parts {
        match &part.kind {
            PartKind::Literal(text) => out.push_str(text),
            PartKind::Var(VarRef::Named(name)) => out.push_str(&format!("${name}")),
            PartKind::Var(VarRef::Positional(n)) => out.push_str(&format!("${n}")),
            PartKind::Var(VarRef::All) => out.push_str("$@"),
            PartKind::Var(VarRef::Count) => out.push_str("$#"),
            PartKind::Capture(chain) => out.push_str(&format!("$({})", render_chain(chain))),
        }
    }
    if word.quoted {
        format!("\"{out}\"")
    } else {
        out
    }
}

/// A task's parameters as written: `who`, or `env=staging`.
fn render_params(task: &Task) -> Vec<String> {
    task.params
        .iter()
        .map(|p| match &p.default {
            Some(d) => format!("{}={}", p.name, render_word(d)),
            None => p.name.clone(),
        })
        .collect()
}

// --- the sona chorefile -------------------------------------------------

const SONA: &str = r#"ggml=$(read .ggml-version)

# Fetch ggml headers pinned to .ggml-version
task fetch-headers {
  for h in ggml ggml-alloc ggml-backend ggml-cpu gguf {
    download "https://raw.githubusercontent.com/ggml-org/ggml/$ggml/include/$h.h" third_party/include/
  }
}

# Build ggml from source
task build-libs {
  git clone --depth 1 --branch $ggml --recurse-submodules https://github.com/ggml-org/ggml ggml-src
  cd ggml-src
  flags="-DCMAKE_BUILD_TYPE=Release"
  if $OS == macos { flags="$flags -DGGML_METAL=ON" }
  else            { flags="$flags -DGGML_VULKAN=ON" }
  if $OS == windows && $ENV == gnu { cmake -B build $flags -G "MinGW Makefiles" }
  else                             { cmake -B build $flags }
}
"#;

#[test]
fn sona_chorefile() {
    let f = file(SONA);

    assert!(f.includes.is_empty());
    assert_eq!(f.globals.len(), 1);
    assert_eq!(f.globals[0].name, "ggml");
    assert_eq!(render_word(&f.globals[0].value), "$(read .ggml-version)");

    let names: Vec<&str> = f.tasks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["fetch-headers", "build-libs"]);
    assert_eq!(
        f.tasks[0].doc.as_deref(),
        Some("Fetch ggml headers pinned to .ggml-version")
    );
    assert_eq!(f.tasks[1].doc.as_deref(), Some("Build ggml from source"));
    assert!(f.tasks.iter().all(|t| t.params.is_empty()));

    assert_eq!(
        render_block(&f.tasks[0].body),
        "{ for h in ggml ggml-alloc ggml-backend ggml-cpu gguf \
         { download \"https://raw.githubusercontent.com/ggml-org/ggml/$ggml/include/$h.h\" \
         third_party/include/ } }"
    );
    assert_eq!(
        render_block(&f.tasks[1].body),
        "{ git clone --depth 1 --branch $ggml --recurse-submodules \
         https://github.com/ggml-org/ggml ggml-src; \
         cd ggml-src; \
         flags=\"-DCMAKE_BUILD_TYPE=Release\"; \
         if $OS == macos { flags=\"$flags -DGGML_METAL=ON\" } \
         else { flags=\"$flags -DGGML_VULKAN=ON\" }; \
         if ($OS == windows && $ENV == gnu) { cmake -B build $flags -G \"MinGW Makefiles\" } \
         else { cmake -B build $flags } }"
    );

    // Every span must slice back out of the source it came from.
    for task in &f.tasks {
        let text = &SONA[task.span.range()];
        assert!(text.starts_with("task "), "task span starts at {text:?}");
        assert!(text.ends_with('}'), "task span ends at {text:?}");
    }
    assert_eq!(
        &SONA[f.globals[0].span.range()],
        "ggml=$(read .ggml-version)"
    );
}

// --- statements ---------------------------------------------------------

#[test]
fn top_level_holds_globals_includes_and_tasks() {
    let f = file("a=1\ninclude other.chore\ntask t { echo }\n");
    assert_eq!(f.globals.len(), 1);
    assert_eq!(f.includes.len(), 1);
    assert_eq!(f.tasks.len(), 1);
}

#[test]
fn assignments() {
    assert_eq!(
        body("task t {\n x=1\n y=\"a b\"\n z=\n w=$x/lib\n}"),
        "{ x=1; y=\"a b\"; z=\"\"; w=$x/lib }"
    );
}

#[test]
fn assignment_only_splits_at_a_statement_start() {
    // `-DFOO=ON` and `a=b=c` keep their `=`; only the leading name is a target.
    assert_eq!(
        body("task t {\n cmake -DFOO=ON\n a=b=c\n}"),
        "{ cmake -DFOO=ON; a=b=c }"
    );
}

#[test]
fn a_binding_in_envs_argument_list_is_a_plain_word() {
    // `env`'s per-command form leans on this: only a *statement*'s first word
    // can be an assignment, so `CGO_ENABLED=0` inside an argument list arrives
    // as one word for the interpreter to split.
    assert_eq!(
        body("task t {\n env CGO_ENABLED=0 go build\n}"),
        "{ env CGO_ENABLED=0 go build }"
    );
    assert_eq!(
        body("task t {\n env A=1 B=$x go build\n}"),
        "{ env A=1 B=$x go build }"
    );
    // And a `^` still belongs to a statement's command name only, which is
    // what keeps the per-command form from having to interpret one.
    assert!(
        error("task t {\n env A=1 ^go build\n}").contains("`^` may only prefix"),
        "a caret in an argument is a syntax error"
    );
}

#[test]
fn chaining_and_redirects() {
    assert_eq!(
        body("task t {\n a && b || c\n a | b | c\n a > out\n b >> out\n c 2> err\n ^find . x\n}"),
        "{ ((a && b) || c); ((a | b) | c); a > out; b >> out; c 2> err; ^find . x }"
    );
}

#[test]
fn pipes_bind_tighter_than_and() {
    assert_eq!(body("task t {\n a | b && c\n}"), "{ ((a | b) && c) }");
}

#[test]
fn if_else_if_else_nests_in_otherwise() {
    let f = file("task t {\n if a { x } else if b { y } else { z }\n}");
    let Stmt::If(outer) = &f.tasks[0].body[0] else {
        panic!("expected an if");
    };
    let otherwise = outer.otherwise.as_ref().expect("missing else");
    assert_eq!(otherwise.len(), 1);
    assert!(
        matches!(otherwise[0], Stmt::If(_)),
        "`else if` must be a nested If, got {:?}",
        otherwise[0]
    );
    assert_eq!(
        render_stmt(&f.tasks[0].body[0]),
        "if a { x } else { if b { y } else { z } }"
    );
}

#[test]
fn one_line_if_else() {
    assert_eq!(
        body("task t { if a { x } else { y } }"),
        "{ if a { x } else { y } }"
    );
}

#[test]
fn else_on_the_next_line() {
    assert_eq!(
        body("task t {\n if a { x }\n else { y }\n}"),
        "{ if a { x } else { y } }"
    );
}

#[test]
fn if_without_else_does_not_swallow_the_next_statement() {
    assert_eq!(
        body("task t {\n if a { x }\n echo done\n}"),
        "{ if a { x }; echo done }"
    );
}

#[test]
fn nested_blocks() {
    assert_eq!(
        body("task t {\n for f in a b {\n  if $f == a {\n   for g in $f { echo $g }\n  }\n }\n}"),
        "{ for f in a b { if $f == a { for g in $f { echo $g } } } }"
    );
}

#[test]
fn for_over_a_capture() {
    assert_eq!(
        body("task t {\n for f in $(find src *.rs) { echo $f }\n}"),
        "{ for f in $(find src *.rs) { echo $f } }"
    );
}

#[test]
fn try_and_exit() {
    assert_eq!(
        body("task t {\n try which cargo\n try a && b\n exit\n exit 2\n}"),
        "{ try which cargo; try (a && b); exit; exit 2 }"
    );
}

#[test]
fn return_with_and_without_a_code() {
    assert_eq!(
        body("task t {\n return\n return 2\n return $code\n}"),
        "{ return; return 2; return $code }"
    );
}

/// `return` ends the task it sits in, wherever that is, so it parses as a
/// statement anywhere a statement is allowed — a `for` body and a nested `if`
/// included.
#[test]
fn return_parses_inside_a_loop_and_a_nested_if() {
    assert_eq!(
        body("task t {\n for f in a b {\n  if $f == a { return 0 }\n }\n exit 1\n}"),
        "{ for f in a b { if $f == a { return 0 } }; exit 1 }"
    );
}

#[test]
fn task_parameters_and_positionals() {
    let f = file("task greet who when {\n echo $1 $2 $@ $#\n}");
    assert_eq!(render_params(&f.tasks[0]), ["who", "when"]);
    assert!(f.tasks[0].params.iter().all(Param::required));
    assert_eq!(render_block(&f.tasks[0].body), "{ echo $1 $2 $@ $# }");
}

#[test]
fn a_task_without_parameters_declares_none() {
    let f = file("task build {\n cargo build\n}");
    assert!(f.tasks[0].params.is_empty());
}

#[test]
fn a_default_makes_a_parameter_optional() {
    let f = file("task deploy env=staging {\n echo $1\n}");
    assert_eq!(render_params(&f.tasks[0]), ["env=staging"]);
    assert!(!f.tasks[0].params[0].required());
}

#[test]
fn required_parameters_may_be_followed_by_optional_ones() {
    let f = file("task fetch url dest=build {\n echo $1 $2\n}");
    assert_eq!(render_params(&f.tasks[0]), ["url", "dest=build"]);
    let required: Vec<bool> = f.tasks[0].params.iter().map(Param::required).collect();
    assert_eq!(required, [true, false]);
}

#[test]
fn a_default_interpolates_and_captures_like_any_other_word() {
    let f = file("task setup target=$TRIPLE {\n echo $1\n}");
    assert_eq!(render_params(&f.tasks[0]), ["target=$TRIPLE"]);

    let f = file("task ship to=$(read .env) {\n echo $1\n}");
    assert_eq!(render_params(&f.tasks[0]), ["to=$(read .env)"]);
}

#[test]
fn a_quoted_default_is_one_word_even_with_spaces() {
    let f = file("task greet who=\"good morning\" {\n echo $1\n}");
    assert_eq!(render_params(&f.tasks[0]), ["who=\"good morning\""]);
    let default = f.tasks[0].params[0].default.as_ref().expect("no default");
    assert!(default.quoted, "a quoted default must stay one argument");
}

#[test]
fn a_default_may_hold_the_equals_signs_after_the_first() {
    // Only the first `=` separates the name from the value, exactly as in an
    // assignment, so a `-D` flag survives being a default.
    let f = file("task build flags=-DGGML_METAL=ON {\n cmake $1\n}");
    assert_eq!(render_params(&f.tasks[0]), ["flags=-DGGML_METAL=ON"]);
}

#[test]
fn an_empty_default_is_the_empty_string() {
    // `env=` means the same here as it does in the assignment `env=`.
    let f = file("task deploy env= {\n echo $1\n}");
    assert_eq!(render_params(&f.tasks[0]), ["env=\"\""]);
    assert!(!f.tasks[0].params[0].required());
}

#[test]
fn a_required_parameter_cannot_follow_an_optional_one() {
    let message = error("task deploy env=staging target {\n echo $1\n}");
    assert!(
        message.contains("required parameter `target`")
            && message.contains("optional parameter `env`"),
        "unhelpful message: {message}"
    );
    assert!(
        message.contains("positional"),
        "message should say why: {message}"
    );
}

#[test]
fn a_parameter_span_covers_the_name_and_the_default() {
    let src = "task deploy host env=staging {\n echo $1 $2\n}";
    let f = file(src);
    let params = &f.tasks[0].params;
    assert_eq!(&src[params[0].span.range()], "host");
    assert_eq!(&src[params[1].span.range()], "env=staging");
    let default = params[1].default.as_ref().expect("no default");
    assert_eq!(&src[default.span.range()], "staging");
}

#[test]
fn a_parameter_name_must_still_be_a_name() {
    assert!(error("task t 1st { }").contains("must be a name"));
    assert!(error("task t \"a b\" { }").contains("must be a name"));
}

// --- conditions ---------------------------------------------------------

#[test]
fn condition_forms() {
    assert_eq!(
        body(
            "task t {\n if $a == $b { x }\n if $a != $b { x }\n if $a contains x { x }\n \
             if $a starts-with x { x }\n if $a ends-with x { x }\n if $a == \"\" { x }\n \
             if exists path { x }\n}"
        ),
        "{ if $a == $b { x }; if $a != $b { x }; if $a contains x { x }; \
         if $a starts-with x { x }; if $a ends-with x { x }; if $a == \"\" { x }; \
         if exists path { x } }"
    );
}

#[test]
fn condition_precedence() {
    // `!` binds tightest, then `&&`, then `||`.
    assert_eq!(
        body("task t {\n if !a && b || c { x }\n}"),
        "{ if ((!(a) && b) || c) { x } }"
    );
    assert_eq!(
        body("task t {\n if a || b && !c { x }\n}"),
        "{ if (a || (b && !(c))) { x } }"
    );
    assert_eq!(
        body("task t {\n if !(a || b) { x }\n}"),
        "{ if !((a || b)) { x } }"
    );
}

#[test]
fn condition_command_keeps_its_pipe_but_not_its_and() {
    assert_eq!(
        body("task t {\n if a | b && c { x }\n}"),
        "{ if ((a | b) && c) { x } }"
    );
}

// --- words --------------------------------------------------------------

#[test]
fn quoting_decides_splitting() {
    let f = file("task t {\n echo bare \"two words\" 'raw $x' a\"b c\"\n}");
    let Stmt::Command(Chain::Single(cmd)) = &f.tasks[0].body[0] else {
        panic!("expected a command");
    };
    let quoted: Vec<bool> = cmd.args.iter().map(|a| a.quoted).collect();
    assert_eq!(quoted, [false, true, true, true]);
    // Single quotes are literal: `$x` inside them is not a variable.
    assert_eq!(render_word(&cmd.args[2]), "\"raw $x\"");
    assert_eq!(render_word(&cmd.args[3]), "\"ab c\"");
}

#[test]
fn interpolation_inside_a_quoted_word() {
    let f = file("task t {\n echo \"$x/lib/$name-$1.$@\"\n}");
    let Stmt::Command(Chain::Single(cmd)) = &f.tasks[0].body[0] else {
        panic!("expected a command");
    };
    assert_eq!(render_word(&cmd.args[0]), "\"$x/lib/$name-$1.$@\"");
    assert!(matches!(
        cmd.args[0].parts[0].kind,
        PartKind::Var(VarRef::Named(_))
    ));
}

#[test]
fn braced_variables() {
    assert_eq!(
        body("task t {\n echo ${x}y \"${x}\"\n}"),
        "{ echo $xy \"$x\" }"
    );
}

#[test]
fn capture_inside_a_string_with_a_nested_chain() {
    assert_eq!(
        body("task t {\n echo \"v=$(git tag | head -1 && echo ok)/end\"\n}"),
        "{ echo \"v=$(((git tag | head -1) && echo ok))/end\" }"
    );
}

#[test]
fn capture_containing_quotes_and_a_nested_capture() {
    assert_eq!(
        body("task t {\n x=$(echo \"a b $(read f)\")\n}"),
        "{ x=$(echo \"a b $(read f)\") }"
    );
}

#[test]
fn escapes_in_a_double_quoted_word() {
    assert_eq!(
        body("task t {\n echo \"a \\\"b\\\" \\$x \\\\ \\d\"\n}"),
        "{ echo \"a \"b\" $x \\ \\d\" }"
    );
}

#[test]
fn word_spans_point_at_the_source() {
    let src = "task t {\n echo hello \"wide world\"\n}";
    let f = file(src);
    let Stmt::Command(Chain::Single(cmd)) = &f.tasks[0].body[0] else {
        panic!("expected a command");
    };
    assert_eq!(&src[cmd.name.span.range()], "echo");
    assert_eq!(&src[cmd.args[0].span.range()], "hello");
    assert_eq!(&src[cmd.args[1].span.range()], "\"wide world\"");
    assert_eq!(&src[cmd.span.range()], "echo hello \"wide world\"");
}

#[test]
fn part_spans_point_at_each_interpolation() {
    let src = "task t {\n echo \"$a/$b\" ${c}x $1\n}";
    let f = file(src);
    let Stmt::Command(Chain::Single(cmd)) = &f.tasks[0].body[0] else {
        panic!("expected a command");
    };
    // Two variables in one word, each locatable on its own.
    let parts = &cmd.args[0].parts;
    assert_eq!(&src[parts[0].span.range()], "$a");
    assert_eq!(&src[parts[1].span.range()], "/");
    assert_eq!(&src[parts[2].span.range()], "$b");

    let parts = &cmd.args[1].parts;
    assert_eq!(&src[parts[0].span.range()], "${c}");
    assert_eq!(&src[parts[1].span.range()], "x");

    assert_eq!(&src[cmd.args[2].parts[0].span.range()], "$1");
}

#[test]
fn capture_and_escape_spans() {
    let src = "task t {\n echo \"pre$(git rev-parse HEAD)\\$post\"\n}";
    let f = file(src);
    let Stmt::Command(Chain::Single(cmd)) = &f.tasks[0].body[0] else {
        panic!("expected a command");
    };
    let parts = &cmd.args[0].parts;
    assert_eq!(&src[parts[0].span.range()], "pre");
    assert_eq!(&src[parts[1].span.range()], "$(git rev-parse HEAD)");
    // An escaped `$` is one character of text and two bytes of source, and
    // the span is the source it came from.
    assert_eq!(&src[parts[2].span.range()], "\\$post");
}

#[test]
fn an_if_span_covers_the_header_only() {
    let src = "task t {\n if $OS == macos && exists Makefile {\n  echo hi\n }\n}";
    let f = file(src);
    let Stmt::If(node) = &f.tasks[0].body[0] else {
        panic!("expected an if");
    };
    assert_eq!(
        &src[node.span.range()],
        "if $OS == macos && exists Makefile"
    );
    assert_eq!(
        &src[node.cond.span().range()],
        "$OS == macos && exists Makefile"
    );
}

#[test]
fn an_if_span_keeps_the_parentheses_the_tree_drops() {
    let src = "task t {\n if ($OS == macos) { echo hi }\n}";
    let f = file(src);
    let Stmt::If(node) = &f.tasks[0].body[0] else {
        panic!("expected an if");
    };
    assert_eq!(&src[node.span.range()], "if ($OS == macos)");
}

#[test]
fn a_for_span_covers_the_header_only() {
    let src = "task t {\n for f in a $(find src *.rs) {\n  echo $f\n }\n}";
    let f = file(src);
    let Stmt::For(node) = &f.tasks[0].body[0] else {
        panic!("expected a for");
    };
    assert_eq!(&src[node.span.range()], "for f in a $(find src *.rs)");
}

#[test]
fn a_return_code_keeps_the_span_of_the_word_it_was_written_as() {
    let src = "task t {\n return $code\n}";
    let f = file(src);
    let Stmt::Return(Some(code)) = &f.tasks[0].body[0] else {
        panic!("expected a return with a code");
    };
    assert_eq!(&src[code.span.range()], "$code");
    assert_eq!(&src[code.parts[0].span.range()], "$code");
}

#[test]
fn a_redirect_span_covers_the_operator_and_its_target() {
    let src = "task t {\n echo hi >> out.log\n}";
    let f = file(src);
    let Stmt::Command(Chain::Single(cmd)) = &f.tasks[0].body[0] else {
        panic!("expected a command");
    };
    assert_eq!(&src[cmd.redirects[0].span.range()], ">> out.log");
}

#[test]
fn chain_spans_run_end_to_end() {
    let src = "task t {\n a && b | c\n}";
    let f = file(src);
    let Stmt::Command(chain) = &f.tasks[0].body[0] else {
        panic!("expected a command");
    };
    assert_eq!(&src[chain.span().range()], "a && b | c");
    let Chain::And(_, right) = chain else {
        panic!("expected `&&` at the top");
    };
    assert_eq!(&src[right.span().range()], "b | c");
}

// --- comments and docs --------------------------------------------------

#[test]
fn doc_comment_attaches_to_the_task_directly_below() {
    let f = file("# Build it\ntask build { x }\n");
    assert_eq!(f.tasks[0].doc.as_deref(), Some("Build it"));
}

#[test]
fn a_blank_line_breaks_the_doc_association() {
    let f = file("# Not a doc\n\ntask build { x }\n");
    assert_eq!(f.tasks[0].doc, None);
}

#[test]
fn the_last_comment_line_wins() {
    let f = file("# first\n# second\ntask build { x }\n");
    assert_eq!(f.tasks[0].doc.as_deref(), Some("second"));
}

#[test]
fn a_statement_between_breaks_the_doc_association() {
    let f = file("# a doc\nx=1\ntask build { y }\n");
    assert_eq!(f.tasks[0].doc, None);
}

#[test]
fn only_one_leading_space_is_stripped() {
    let f = file("#  padded\ntask a { x }\n#---\ntask b { x }\n");
    assert_eq!(f.tasks[0].doc.as_deref(), Some(" padded"));
    assert_eq!(f.tasks[1].doc.as_deref(), Some("---"));
}

#[test]
fn comments_end_at_the_line_but_not_inside_a_string() {
    assert_eq!(
        body("task t {\n echo hi # trailing\n # whole line\n echo \"a # b\"\n}"),
        "{ echo hi; echo \"a # b\" }"
    );
}

// --- includes -----------------------------------------------------------

#[test]
fn includes_are_recorded_not_followed() {
    let src = "include other.chore\ninclude libs/chorefile as libs\n";
    let f = file(src);
    assert_eq!(f.includes[0].path, "other.chore");
    assert_eq!(f.includes[0].namespace, None);
    assert_eq!(f.includes[1].path, "libs/chorefile");
    assert_eq!(f.includes[1].namespace.as_deref(), Some("libs"));
    assert_eq!(
        &src[f.includes[1].span.range()],
        "include libs/chorefile as libs"
    );
}

// --- errors -------------------------------------------------------------

#[test]
fn error_unclosed_block() {
    assert!(error("task t {\n echo hi\n").contains("unclosed `{`"));
}

#[test]
fn error_unterminated_string() {
    assert!(
        error("task t {\n echo \"oops\n}").contains("unterminated double-quoted string"),
        "message was: {}",
        error("task t {\n echo \"oops\n}")
    );
    assert!(error("task t {\n echo 'oops\n}").contains("unterminated single-quoted string"));
}

#[test]
fn error_unterminated_capture() {
    assert!(error("x=$(read f\n").contains("unterminated `$(`"));
}

#[test]
fn error_empty_capture() {
    assert!(error("x=$()\n").contains("empty `$( )`"));
}

#[test]
fn error_command_at_the_top_level() {
    let message = error("echo hi\n");
    assert!(message.contains("top level"), "message was: {message}");
    assert!(message.contains("`echo`"), "message was: {message}");
}

#[test]
fn error_return_outside_a_task() {
    // There is no task to leave at the top level, and the message has to say
    // which statement the author wanted instead.
    let message = error("return\n");
    assert!(
        message.contains("only valid inside a task"),
        "message was: {message}"
    );
    assert!(message.contains("`exit`"), "message was: {message}");
}

#[test]
fn error_missing_in_after_for() {
    let message = error("task t {\n for x a b { echo }\n}");
    assert!(message.contains("expected `in`"), "message was: {message}");
}

#[test]
fn error_bad_task_parameter() {
    let message = error("task t bad-name { echo }\n");
    assert!(message.contains("must be a name"), "message was: {message}");
}

#[test]
fn error_missing_block() {
    let message = error("task t\n");
    assert!(message.contains("expected `{`"), "message was: {message}");
}

#[test]
fn error_two_statements_on_one_line() {
    let message = error("task t {\n echo a } echo b\n}");
    assert!(
        message.contains("separated by newlines"),
        "message was: {message}"
    );
}

#[test]
fn error_redirect_without_a_target() {
    let message = error("task t {\n echo hi >\n}");
    assert!(
        message.contains("expected a file to redirect to"),
        "message was: {message}"
    );
}

#[test]
fn error_zero_is_not_a_parameter() {
    let message = error("task t {\n echo $0\n}");
    assert!(
        message.contains("numbered from `$1`"),
        "message was: {message}"
    );
}

#[test]
fn error_stdin_redirect_is_not_supported() {
    let message = error("task t {\n echo < in\n}");
    assert!(
        message.contains("`<` is not supported"),
        "message was: {message}"
    );
}

#[test]
fn error_single_ampersand() {
    let message = error("task t {\n a & b\n}");
    assert!(message.contains("use `&&`"), "message was: {message}");
}

#[test]
fn error_include_path_must_be_literal() {
    let message = error("include $dir/other.chore\n");
    assert!(
        message.contains("without interpolation"),
        "message was: {message}"
    );
}

#[test]
fn error_missing_command_after_operator() {
    let message = error("task t {\n echo a &&\n}");
    assert!(
        message.contains("expected a command"),
        "message was: {message}"
    );
}

// --- require ------------------------------------------------------------

#[test]
fn require_states_a_version() {
    let f = file("require 1.4.0\n\ntask build { echo hi }\n");
    let require = f.require.expect("no require parsed");
    assert_eq!(require.version.to_string(), "1.4.0");
    // The span covers the keyword too, so a diagnostic points at the line
    // rather than at the number on its own.
    assert_eq!(require.span.start, 0);
    assert_eq!(require.span.end, "require 1.4.0".len());
    assert_eq!(f.tasks.len(), 1);
}

#[test]
fn a_file_without_require_states_none() {
    assert!(file("task build { echo hi }\n").require.is_none());
}

#[test]
fn error_require_is_not_a_bare_triple() {
    // Every spelling someone might reach for, and none of them accepted: the
    // version is a floor, so a range has nothing to mean here.
    for bad in ["v1.4.0", "1.4", "^1.4.0", ">=1.4.0", "1.4.0-rc1", "latest"] {
        let message = error(&format!("require {bad}\n"));
        assert!(
            message.contains("<major>.<minor>.<patch>"),
            "`{bad}` gave: {message}"
        );
        assert!(
            message.contains("`require 1.4.0`"),
            "`{bad}` gave: {message}"
        );
    }
}

#[test]
fn error_require_without_a_version() {
    // Nothing at all is wrong in the same way `^1.4.0` is wrong, and gets the
    // same answer: here is the shape.
    let message = error("require\n");
    assert!(
        message.contains("<major>.<minor>.<patch>"),
        "message was: {message}"
    );
}

#[test]
fn error_two_requires_name_both() {
    let message = error("require 1.0.0\nrequire 1.2.0\n");
    assert!(
        message.contains("only one `require`"),
        "message was: {message}"
    );
    assert!(message.contains("1.0.0"), "message was: {message}");
    assert!(message.contains("1.2.0"), "message was: {message}");
}

#[test]
fn error_require_inside_a_task() {
    // A requirement about the file cannot be stated by one task, and the
    // message says where it belongs.
    let message = error("task build {\n    require 1.4.0\n}\n");
    assert!(
        message.contains("only valid at the top level"),
        "message was: {message}"
    );
}

#[test]
fn an_unknown_top_level_word_hints_at_a_stale_binary() {
    // The one failure `require` cannot report for itself: a binary too old to
    // know the keyword sees a stray word, so that message carries the hint.
    let message = error("newkeyword 1.0.0\n");
    assert!(message.contains("may be too old"), "message was: {message}");
    // A token that is not a word is not a candidate for a keyword.
    let message = error("{ echo hi }\n");
    assert!(
        !message.contains("may be too old"),
        "message was: {message}"
    );
}

// --- script blocks ------------------------------------------------------

const DEPS: &str = r#"task deps {
    script uv run - {
        import tomllib, pathlib
        data = tomllib.loads(pathlib.Path("Cargo.toml").read_text())
        print(data["workspace"]["package"]["version"])
    }
}
"#;

#[test]
fn script_hands_a_raw_body_to_another_interpreter() {
    let f = file(DEPS);
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");

    let command: Vec<String> = s.command.iter().map(render_word).collect();
    assert_eq!(command, ["uv", "run", "-"]);
    // Verbatim, minus the indentation the task put there: the quotes and
    // brackets are Python's business, and chore has not touched them.
    assert_eq!(
        s.body,
        "import tomllib, pathlib\n\
         data = tomllib.loads(pathlib.Path(\"Cargo.toml\").read_text())\n\
         print(data[\"workspace\"][\"package\"][\"version\"])\n"
    );
}

#[test]
fn a_script_command_is_expanded_but_its_body_is_not() {
    let src = r#"task t {
    script $PYTHON $(which -q flags) - {
        print("$PYTHON is not interpolated, nor is $1 or $@")
        path = "C:\Users\a"  # backslashes and quotes are Python's
    }
}
"#;
    let f = file(src);
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");

    // argv goes through the ordinary word rules,
    let command: Vec<String> = s.command.iter().map(render_word).collect();
    assert_eq!(command, ["$PYTHON", "$(which -q flags)", "-"]);
    // and the body goes through none of them: it is still one flat string,
    // `$` and all.
    assert_eq!(
        s.body,
        "print(\"$PYTHON is not interpolated, nor is $1 or $@\")\n\
         path = \"C:\\Users\\a\"  # backslashes and quotes are Python's\n"
    );
}

#[test]
fn a_script_body_may_hold_braces_on_their_own_lines() {
    // The case that decides the termination rule. A body that closes a dict on
    // its own line has a `}` alone on a line — dedented, even — and the block
    // still runs to the `}` that lines up with `script`.
    let src = r#"task t {
    script node - {
        const data = {
            "shape": "{}",
        }
        if (data) {
            console.log("}")
        }
    }
    echo after
}
"#;
    let f = file(src);
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");
    assert_eq!(
        s.body,
        "const data = {\n    \"shape\": \"{}\",\n}\nif (data) {\n    console.log(\"}\")\n}\n"
    );
    // The statement after the block is still seen, so the block ended where it
    // should have and not at the file's end.
    assert_eq!(
        render_block(&f.tasks[0].body),
        format!("{{ {}; echo after }}", render_stmt(&f.tasks[0].body[0]))
    );
}

#[test]
fn a_script_body_loses_only_the_indentation_every_line_shares() {
    let src = "task t {\n\
               \x20   script sh - {\n\
               \x20       if true; then\n\
               \x20           echo deep\n\
               \n\
               \x20       fi\n\
               \x20   }\n\
               }\n";
    let f = file(src);
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");
    // Eight spaces come off every line; the four that make `echo deep` a body
    // stay, and the blank line stays blank.
    assert_eq!(s.body, "if true; then\n    echo deep\n\nfi\n");
}

#[test]
fn a_less_indented_first_line_sets_the_common_indentation() {
    // The prefix is what every line shares, so an outdented line is the one
    // that decides it and the rest keep the difference.
    let src = "task t {\n\
               \x20   script sh - {\n\
               \x20     echo first\n\
               \x20       echo second\n\
               \x20   }\n\
               }\n";
    let f = file(src);
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");
    assert_eq!(s.body, "echo first\n  echo second\n");
}

#[test]
fn script_spans_slice_back_out_of_the_source() {
    let f = file(DEPS);
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");

    let text = &DEPS[s.span.range()];
    assert!(text.starts_with("script uv run - {"), "span is {text:?}");
    assert!(text.ends_with('}'), "span is {text:?}");

    // The body span is the source the body was made from — indentation
    // included, since that is what is actually written there.
    let raw = &DEPS[s.body_span.range()];
    assert!(
        raw.starts_with("        import tomllib"),
        "body span is {raw:?}"
    );
    assert!(raw.ends_with("\"version\"])\n"), "body span is {raw:?}");
    // It starts at the first byte of the body, which is the line after the
    // `{`, and stops before the line the closing `}` is on.
    assert_eq!(DEPS.as_bytes()[s.body_span.start - 1], b'\n');
    assert_eq!(&DEPS[s.body_span.end..s.body_span.end + 5], "    }");
}

#[test]
fn an_empty_script_block_is_an_empty_body() {
    let f = file("task t {\n    script sh - {\n    }\n}\n");
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");
    assert_eq!(s.body, "");
    assert_eq!(s.body_span.start, s.body_span.end);
}

#[test]
fn script_blocks_nest_inside_if_and_for() {
    let src = r#"task t {
    if $OS == macos {
        script python3 - {
            print({"os": "macos"})
        }
    }
    for f in a b {
        script sh - {
            echo one }
            echo two
        }
    }
}
"#;
    let f = file(src);
    let block = &f.tasks[0].body;
    let Stmt::If(i) = &block[0] else {
        panic!("expected an if, got {:?}", block[0]);
    };
    assert_eq!(
        render_stmt(&i.then[0]),
        "script python3 - \"print({\\\"os\\\": \\\"macos\\\"})\\n\""
    );
    let Stmt::For(loop_) = &block[1] else {
        panic!("expected a for, got {:?}", block[1]);
    };
    // A trailing `}` on a body line is body text, not the end of the block:
    // only a `}` at the keyword's own indentation closes it.
    assert_eq!(
        render_stmt(&loop_.body[0]),
        "script sh - \"echo one }\\necho two\\n\""
    );
}

// --- script blocks compose ----------------------------------------------

#[test]
fn a_script_block_is_captured_by_a_dollar_paren() {
    // The reason the block is a chain element and not a statement: computing a
    // value in another language and using it here.
    let src = r#"task version {
    version=$(script uv run - {
        import tomllib, pathlib
        print(tomllib.loads(pathlib.Path("Cargo.toml").read_text())["workspace"]["package"]["version"])
    })
    echo $version
}
"#;
    let f = file(src);
    let block = &f.tasks[0].body;
    let Stmt::Assign(a) = &block[0] else {
        panic!("expected an assignment, got {:?}", block[0]);
    };
    // The whole value is one capture, and the capture is the block itself.
    let [part] = a.value.parts.as_slice() else {
        panic!("expected one part, got {:?}", a.value.parts);
    };
    let PartKind::Capture(chain) = &part.kind else {
        panic!("expected a capture, got {:?}", part.kind);
    };
    let Chain::Script(s) = chain.as_ref() else {
        panic!("expected a script block, got {chain:?}");
    };
    let command: Vec<String> = s.command.iter().map(render_word).collect();
    assert_eq!(command, ["uv", "run", "-"]);
    assert_eq!(
        s.body,
        "import tomllib, pathlib\n\
         print(tomllib.loads(pathlib.Path(\"Cargo.toml\").read_text())\
         [\"workspace\"][\"package\"][\"version\"])\n"
    );
    // The block ended where the `})` is, so the statement after it is seen.
    assert_eq!(render_stmt(&block[1]), "echo $version");
}

#[test]
fn a_captured_block_ends_at_the_indentation_of_the_line_it_opened_on() {
    // The `script` does not start its line here, so the rule is the line's
    // indentation, not the keyword's column: the block closes at the `})`
    // written where `version=` was, which is what an author would write and
    // an editor would indent to.
    let src = "task t {\n\
               \x20   version=$(script sh - {\n\
               \x20       echo 1\n\
               \x20   })\n\
               \x20   echo $version\n\
               }\n";
    let f = file(src);
    let s = find_script(&f.tasks[0].body).expect("no script block parsed");
    assert_eq!(s.body, "echo 1\n");
    assert_eq!(render_stmt(&f.tasks[0].body[1]), "echo $version");
}

#[test]
fn a_captured_block_closes_at_a_line_starting_with_the_paren() {
    // A global at column zero: the line the `script` sits on has no
    // indentation, so the `})` is written at the start of a line.
    let src = "version=$(script uv run - {\n\
               \x20   print(1)\n\
               })\n\
               task t {\n\
               \x20   echo $version\n\
               }\n";
    let f = file(src);
    let s = script_in_word(&f.globals[0].value).expect("no script block parsed");
    assert_eq!(s.body, "print(1)\n");
    // Everything after the capture still parses as itself.
    assert_eq!(f.globals[0].name, "version");
    assert_eq!(f.tasks[0].name, "t");
}

#[test]
fn a_script_block_takes_either_side_of_a_pipe() {
    let src = "task t {\n\
               \x20   script node - {\n\
               \x20       console.log(\"hi\")\n\
               \x20   } | wc -l\n\
               \x20   cat in.txt | script sh - {\n\
               \x20       tr a-z A-Z\n\
               \x20   }\n\
               }\n";
    let block = &file(src).tasks[0].body;
    assert_eq!(
        render_stmt(&block[0]),
        "(script node - \"console.log(\\\"hi\\\")\\n\" | wc -l)"
    );
    assert_eq!(
        render_stmt(&block[1]),
        "(cat in.txt | script sh - \"tr a-z A-Z\\n\")"
    );
}

#[test]
fn a_script_block_takes_a_redirect() {
    let src = "task t {\n\
               \x20   script node - {\n\
               \x20       console.log(\"hi\")\n\
               \x20   } > out.txt 2> err.txt\n\
               }\n";
    let block = &file(src).tasks[0].body;
    assert_eq!(
        render_stmt(&block[0]),
        "script node - \"console.log(\\\"hi\\\")\\n\" > out.txt 2> err.txt"
    );
    // The block's own span still stops at its closing brace: the redirect is
    // written after the block, not part of it.
    let s = find_script(block).expect("no script block parsed");
    assert!(src[s.span.range()].ends_with('}'), "span is {:?}", s.span);
}

#[test]
fn a_script_block_joins_with_and_and_or() {
    let src = "task t {\n\
               \x20   script uv run - {\n\
               \x20       check()\n\
               \x20   } && echo ok || echo failed\n\
               }\n";
    let block = &file(src).tasks[0].body;
    assert_eq!(
        render_stmt(&block[0]),
        "((script uv run - \"check()\\n\" && echo ok) || echo failed)"
    );
}

#[test]
fn a_script_block_runs_under_try() {
    let src = "task t {\n\
               \x20   try script sh - {\n\
               \x20       exit 1\n\
               \x20   }\n\
               }\n";
    let block = &file(src).tasks[0].body;
    assert_eq!(render_stmt(&block[0]), "try script sh - \"exit 1\\n\"");
}

#[test]
fn a_script_block_is_a_condition() {
    // A block exits like anything else, so it reads as a condition. The `}`
    // that lines up with the `if` closes the block; the `{` after it opens the
    // branch, as it would after any other command.
    let src = "task t {\n\
               \x20   if script sh - {\n\
               \x20       test -f x\n\
               \x20   } {\n\
               \x20       echo yes\n\
               \x20   }\n\
               }\n";
    let block = &file(src).tasks[0].body;
    assert_eq!(
        render_stmt(&block[0]),
        "if script sh - \"test -f x\\n\" { echo yes }"
    );
}

#[test]
fn error_unterminated_script_block_inside_a_capture() {
    // The `})` is at column zero and the `script` sits on a line indented by
    // four, so nothing closes the block. The message names the line the block
    // opened on — the line in the file, not the capture's first line, which is
    // all a fragment could have offered.
    let message = error("task t {\n    v=$(script sh - {\n        echo hi\n})\n}\n");
    assert!(
        message.contains("unterminated script block, opened at line 2"),
        "message was: {message}"
    );
    // Column 5 is where the line's indentation ends, which is where the
    // closing `}` had to be written.
    assert!(message.contains("column 5"), "message was: {message}");
}

#[test]
fn error_unterminated_script_block() {
    // The task's `}` is at column 0 and the `script` is indented, so nothing
    // closes the block — and the message names the line it opened on rather
    // than complaining about the end of the file.
    let message = error("task t {\n    script sh - {\n        echo hi\n}\n");
    assert!(
        message.contains("unterminated script block, opened at line 2"),
        "message was: {message}"
    );
    assert!(message.contains("column 5"), "message was: {message}");
}

#[test]
fn error_script_without_a_command() {
    // Nothing to feed the block to. The braces stay ordinary here, so the
    // message is about the missing command and not about the body.
    let message = error("task t {\n    script {\n    }\n}\n");
    assert!(
        message.contains("`script` needs the command"),
        "message was: {message}"
    );
    assert!(
        message.contains("script python3 -"),
        "message was: {message}"
    );
}

#[test]
fn error_script_body_must_start_on_its_own_line() {
    let message = error("task t {\n    script sh - { echo hi }\n}\n");
    assert!(
        message.contains("starts on the line after `{`"),
        "message was: {message}"
    );
}

#[test]
fn error_script_at_the_top_level() {
    let message = error("script sh - {\n    echo hi\n}\n");
    assert!(
        message.contains("only valid inside a task"),
        "message was: {message}"
    );
}
