//! The shape of a parsed chorefile.

use crate::error::Span;

/// One parsed file, before includes are merged.
#[derive(Debug, Default)]
pub struct File {
    /// `include` directives, in source order.
    pub includes: Vec<Include>,
    /// Top-level assignments. Evaluated once before the first task runs, and
    /// never by `list`, `help`, `check` or `spec` — those only need the tree.
    pub globals: Vec<Assign>,
    pub tasks: Vec<Task>,
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
    pub params: Vec<String>,
    /// The comment line directly above the task, shown by `list`.
    pub doc: Option<String>,
    pub body: Block,
    pub span: Span,
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
    pub target: Word,
    /// The operator and its target together, as in `> out.txt`.
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
