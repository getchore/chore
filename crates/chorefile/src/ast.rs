//! The shape of a parsed chorefile.

use crate::error::Span;

/// One parsed file, before includes are merged.
#[derive(Debug, Default)]
pub struct File {
    /// The `require` this file states, if it states one. Kept per file rather
    /// than folded into the merge, because an unmet requirement has to name
    /// the file that asked for it, and a merged tree has forgotten which one
    /// that was.
    pub require: Option<Require>,
    /// `include` directives, in source order.
    pub includes: Vec<Include>,
    /// Top-level assignments. Evaluated once before the first task runs, and
    /// never by `list`, `help`, `check` or `spec` — those only need the tree.
    pub globals: Vec<Assign>,
    pub tasks: Vec<Task>,
}

/// `require 1.4.0`: the oldest `chore` that can run this file.
///
/// The version is parsed here rather than kept as text, so that the
/// comparison is numeric per component and cannot be done any other way. See
/// [`require`](crate::require) for what is done with it.
#[derive(Debug)]
pub struct Require {
    pub version: crate::require::Version,
    /// The whole directive, keyword included, so a diagnostic points at the
    /// line that stated the requirement rather than at the number alone.
    pub span: Span,
}

#[derive(Debug)]
pub struct Include {
    /// Path as written, resolved relative to the file doing the including.
    /// A directory means the `chorefile` inside it.
    pub path: String,
    /// `as name`: tasks and globals are exposed as `name::task`. Without it
    /// they merge flat, and any duplicate is a `check` error.
    pub namespace: Option<String>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Task {
    pub name: String,
    /// Declared parameters, bound to `$1`, `$2`, ... in the body.
    ///
    /// A parameter may carry a default (`task deploy env=staging { }`), which
    /// makes it optional. Every parameter without one must be supplied, and
    /// they come first: a required parameter after an optional one could only
    /// be reached by supplying the optional one anyway, so the grammar
    /// refuses it rather than letting a chorefile declare something no caller
    /// can satisfy.
    pub params: Vec<Param>,
    /// The task's one-line description, shown by `list`.
    ///
    /// It is the first non-empty line of the contiguous `#` block directly
    /// above the task — one line, however many the block holds, because that
    /// is what a listing has room for and the summary is what a block leads
    /// with. A blank line ends a block, so a file header is not a description,
    /// and the line is cut at the end of its first sentence.
    pub doc: Option<String>,
    pub body: Block,
    pub span: Span,
}

/// One declared parameter of a task.
#[derive(Debug)]
pub struct Param {
    pub name: String,
    /// The value bound when the caller supplies nothing. Evaluated at call
    /// time, in the called task's scope, so a default can read `$TRIPLE` or
    /// capture a command — and pays for it only when it is actually used.
    pub default: Option<Word>,
    pub span: Span,
}

impl Param {
    pub fn required(&self) -> bool {
        self.default.is_none()
    }
}

/// A `script <command...> { ... }` statement.
#[derive(Debug)]
pub struct Script {
    /// The command and its arguments, expanded like any other command's, so
    /// `script uv run -` and `script python3 -` both work and a `$var` in the
    /// argv still interpolates. Only the *body* is raw.
    pub command: Vec<Word>,
    /// The block, exactly as written, minus the common indentation.
    pub body: String,
    /// The `script` keyword through the closing brace.
    pub span: Span,
    /// Where `body` starts in the source, so a diagnostic about the block can
    /// point into it rather than at the keyword.
    pub body_span: Span,
    /// `> out.txt` and friends. A script block is not a [`Command`], so its
    /// redirects live here rather than there; `span` stops at the closing
    /// brace and each redirect carries its own.
    pub redirects: Vec<Redirect>,
}

pub type Block = Vec<Stmt>;

#[derive(Debug)]
pub enum Stmt {
    Assign(Assign),
    Command(Chain),
    If(If),
    For(For),
    /// `try <cmd>` — run it, ignore a nonzero exit.
    Try(Chain),
    /// `exit [code]` — ends the **whole run**, unwinding every caller with
    /// this code. Nothing after it happens, in this task or in the one that
    /// called it.
    Exit(Option<Word>),

    /// `return [code]` — ends the **enclosing task** and hands control back to
    /// its caller, which carries on with the next statement. The code becomes
    /// the task's exit status, so `&&`, `||`, `try`, an `if` condition and a
    /// `$( )` capture all read it exactly as they read any other command's.
    ///
    /// This is the difference the two statements exist to draw: a `setup` task
    /// that finds its work already done wants to stop *itself*, so that a
    /// `dev` task calling it still gets to run `tauri dev`. `exit` stops the
    /// run and takes the caller down with it; `return` stops one frame
    /// earlier. In the task named on the command line there is no caller, so
    /// `return` ends the run — successfully, unless it names a code.
    ///
    /// It is not a loop control: inside a `for`, `return` leaves the task, not
    /// the loop. There is no `break`.
    Return(Option<Word>),
}

#[derive(Debug)]
pub struct Assign {
    pub name: String,
    pub value: Word,
    pub span: Span,
}

#[derive(Debug)]
pub struct If {
    pub cond: Cond,
    pub then: Block,
    /// `else if` is parsed as an `If` statement inside this block.
    pub otherwise: Option<Block>,
    /// The header — `if <cond>` — and not the body. A diagnostic about the
    /// condition should point at the condition, not at forty lines of block.
    pub span: Span,
}

#[derive(Debug)]
pub struct For {
    pub var: String,
    /// Each word is split on whitespace after interpolation, so
    /// `for f in $(find src *.rs)` iterates every match.
    pub items: Vec<Word>,
    pub body: Block,
    /// The header — `for x in <items>` — and not the body, for the same
    /// reason as [`If::span`].
    pub span: Span,
}

/// A condition. Every form ultimately reduces to a boolean, and a bare
/// command is true when it exits zero.
#[derive(Debug)]
pub enum Cond {
    Compare {
        left: Word,
        op: CompareOp,
        right: Word,
    },
    /// `exists path`, `which name`, or any other command's exit code.
    Command(Chain),
    Not(Box<Cond>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
}

impl Cond {
    /// Where this condition sits in the source.
    ///
    /// Derived from the children rather than stored: every form spans exactly
    /// its operands, so a stored span would be a second copy of the same fact
    /// and free to go stale. The one thing it cannot recover is a prefix
    /// operator — `!cond` reports the span of `cond` — since the `!` leaves no
    /// trace in the tree.
    pub fn span(&self) -> Span {
        match self {
            Self::Compare { left, right, .. } => Span::new(left.span.start, right.span.end),
            Self::Command(chain) => chain.span(),
            Self::Not(inner) => inner.span(),
            Self::And(a, b) | Self::Or(a, b) => Span::new(a.span().start, b.span().end),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Contains,
    StartsWith,
    EndsWith,
}

/// Commands joined by `&&`, `||` or `|`, with redirections.
#[derive(Debug)]
pub enum Chain {
    Single(Command),
    /// `script <command...> { <raw text> }` — hand a block of text to another
    /// interpreter on its stdin.
    ///
    /// It lives in `Chain` rather than in `Stmt` so that it composes like
    /// anything else that runs: `x=$(script uv run - { ... })`, a pipe, a
    /// redirect, `&&`. Computing a value in another language and using it in
    /// the task is the main reason to reach for this, and a form that could
    /// not hand the value back would have sent every author to a temporary
    /// file instead.
    ///
    /// The escape hatch, and deliberately a narrow one. `check` can tell you
    /// `curl` will not work on Windows because it knows every builtin, and
    /// `--dry` can skip effects because every builtin says whether it has any.
    /// Neither is knowable about text handed to another program, so a block is
    /// checked for nothing and never previewed — and `check` says so rather
    /// than letting a reader assume the usual guarantees hold.
    ///
    /// The text is raw: no interpolation, no quoting rules. It reaches the
    /// command on stdin rather than in argv, so a quote or a `$` inside it
    /// means whatever that interpreter says it means.
    Script(Script),
    And(Box<Chain>, Box<Chain>),
    Or(Box<Chain>, Box<Chain>),
    Pipe(Box<Chain>, Box<Chain>),
}

impl Chain {
    /// Where this chain sits in the source, derived from its ends for the
    /// same reason as [`Cond::span`].
    pub fn span(&self) -> Span {
        match self {
            Self::Single(cmd) => cmd.span,
            Self::Script(script) => script.span,
            Self::And(a, b) | Self::Or(a, b) | Self::Pipe(a, b) => {
                Span::new(a.span().start, b.span().end)
            }
        }
    }
}

#[derive(Debug)]
pub struct Command {
    /// Resolution order: task, then builtin, then PATH. A leading `^` forces
    /// PATH and is recorded here, not kept in the name.
    pub name: Word,
    pub force_path: bool,
    pub args: Vec<Word>,
    pub redirects: Vec<Redirect>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Redirect {
    pub kind: RedirectKind,
    /// The file to write to. `None` for [`RedirectKind::StderrToStdout`],
    /// which names a stream rather than a path — the one redirect with
    /// nothing after the operator, which is why this is an `Option` rather
    /// than a `Word` every walk can assume is there.
    pub target: Option<Word>,
    /// The operator and its target together, as in `> out.txt`, or just the
    /// operator where there is no target.
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectKind {
    /// `>`
    Stdout,
    /// `>>`
    StdoutAppend,
    /// `2>`
    Stderr,
    /// `2>&1` — send stderr wherever stdout is going.
    ///
    /// Deliberately *not* sh's general stream dup: `2>&1` is the only
    /// spelling, there is no `1>&2` and no `3>&`, and it is read as a
    /// statement about the command rather than as an operation performed in
    /// written order. `> log 2>&1` and `2>&1 > log` therefore both mean "both
    /// streams into `log`", where sh gives the second one a stderr still
    /// pointed at the terminal. Order-dependence is the part of `2>&1`
    /// everyone gets wrong, and a task runner has nothing to gain from
    /// reproducing it.
    StderrToStdout,
}

/// One argument, before interpolation.
///
/// Whether a word splits into several argv entries is decided here, not at
/// runtime: a quoted word is always exactly one argument, an unquoted word
/// splits on the whitespace its interpolated parts introduce.
#[derive(Debug)]
pub struct Word {
    pub parts: Vec<WordPart>,
    pub quoted: bool,
    pub span: Span,
}

/// One piece of a word, with the source it came from.
///
/// The span is per-part rather than per-word so that a diagnostic about an
/// interpolation points at the `$name` itself: `"$a/$b"` is one word but two
/// places to point at.
#[derive(Debug)]
pub struct WordPart {
    pub kind: PartKind,
    pub span: Span,
}

impl WordPart {
    pub fn new(kind: PartKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug)]
pub enum PartKind {
    Literal(String),
    /// `$name`, `$1`, `$@`, `$#`
    Var(VarRef),
    /// `$(cmd)` — stdout, trimmed. A nonzero exit fails unless wrapped in `try`.
    Capture(Box<Chain>),
}

#[derive(Debug)]
pub enum VarRef {
    Named(String),
    /// `$1`, `$2`, ...
    Positional(usize),
    /// `$@` — every argument, as separate words.
    All,
    /// `$#` — argument count.
    Count,
}
