//! Source text to tokens.
//!
//! The lexer never looks inside a word: `"$x/lib"` and `$(cmd a b)` arrive as
//! a single [`Token::Word`] holding the verbatim source text, quotes and all.
//! The parser re-reads that text to build the interpolation parts, and because
//! the text is a byte-for-byte copy of the source, every part it produces can
//! still be given an accurate span.

use std::path::PathBuf;

use crate::error::{Error, Location, Result, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A bare word, or a quoted string. Interpolation is resolved later, by
    /// the parser, which needs the quoting to decide argument splitting.
    Word {
        text: String,
        quoted: bool,
    },
    /// A `#` comment. Kept, because the one directly above a task is its
    /// description.
    Comment(String),
    /// The raw text between the braces of a `script` block, verbatim and
    /// undecoded: the span is exactly the bytes it came from. It always sits
    /// between the [`LBrace`](Self::LBrace) and [`RBrace`](Self::RBrace) of
    /// the block, so the parser reads the three as one shape.
    ScriptBody(String),
    Assign,
    LBrace,
    RBrace,
    LParen,
    RParen,
    AndAnd,
    OrOr,
    Pipe,
    Gt,
    GtGt,
    ErrGt,
    Bang,
    Caret,
    Newline,
    Eof,
}

impl Token {
    /// The token as it would be written, for error messages.
    pub fn describe(&self) -> String {
        match self {
            Self::Word { text, .. } => format!("`{text}`"),
            Self::Comment(_) => "a comment".into(),
            Self::ScriptBody(_) => "a script block".into(),
            Self::Assign => "`=`".into(),
            Self::LBrace => "`{`".into(),
            Self::RBrace => "`}`".into(),
            Self::LParen => "`(`".into(),
            Self::RParen => "`)`".into(),
            Self::AndAnd => "`&&`".into(),
            Self::OrOr => "`||`".into(),
            Self::Pipe => "`|`".into(),
            Self::Gt => "`>`".into(),
            Self::GtGt => "`>>`".into(),
            Self::ErrGt => "`2>`".into(),
            Self::Bang => "`!`".into(),
            Self::Caret => "`^`".into(),
            Self::Newline => "end of line".into(),
            Self::Eof => "end of file".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Spanned {
    pub token: Token,
    pub span: Span,
}

/// Tokenize a whole file.
///
/// Errors carry an empty path; [`parse`](crate::parse::parse) is what knows
/// which file the source came from and fills it in.
pub fn lex(source: &str) -> Result<Vec<Spanned>> {
    lex_offset(source, source, 0)
}

/// Tokenize a fragment whose first byte sits at `base` in the enclosing file,
/// so tokens inside a `$(...)` still point at the original source.
///
/// `full` is that enclosing file. A fragment is enough to cut into tokens, but
/// not to measure a `script` block's indentation by: the fragment inside
/// `$(script uv run - {` begins part way through a line, and its own first
/// column is not the line's. Everything about a block that is a property of
/// the *line* — the indentation its closing `}` must match, the line number an
/// unterminated block is reported at — is read out of `full` instead.
pub(crate) fn lex_offset(source: &str, full: &str, base: usize) -> Result<Vec<Spanned>> {
    Lexer {
        src: source,
        full,
        base,
        i: 0,
        out: Vec::new(),
        stmt_start: true,
        cmd_start: true,
        task_header: false,
        script_kw: None,
        script_argv: false,
        depth: 0,
    }
    .run()
}

pub(crate) fn syntax<T>(message: impl Into<String>, span: Span) -> Result<T> {
    Err(Error::Syntax {
        message: message.into(),
        at: Location {
            file: PathBuf::new(),
            span,
        },
    })
}

struct Lexer<'a> {
    src: &'a str,
    /// The whole file `src` is a fragment of; `src` is `full[base..]` clipped
    /// to the fragment. Only the `script` rules read it — see [`lex_offset`].
    full: &'a str,
    base: usize,
    i: usize,
    out: Vec<Spanned>,
    /// Whether the next word could start a statement. `name=value` only splits
    /// into `Word` `Assign` `Word` there, so `cmake -DFOO=ON` and `a=b=c` keep
    /// their `=` inside the word.
    stmt_start: bool,
    /// Whether the next word could begin a command. Every statement start is
    /// one, and so is the position after `|`, `&&`, `||`, `(` and `!`, because
    /// a command may follow each of them.
    ///
    /// It is a second flag rather than a wider [`stmt_start`](Self::stmt_start)
    /// because the two questions differ: `a=b` splits into an assignment only
    /// at a statement start, while `script` is the keyword anywhere a command
    /// can be written — including inside a `$( )`, which is lexed as its own
    /// fragment and so starts in command position anyway.
    cmd_start: bool,
    /// Whether we are between `task` and the `{` that opens its body. A task
    /// header is the one other place `name=value` splits, because that is how
    /// a parameter declares a default. Each word of the header gets the split
    /// offered to it once — the flag re-arms [`stmt_start`](Self::stmt_start)
    /// at every space — so `target=$TRIPLE` splits but `a=b=c` still keeps its
    /// second `=` inside the value.
    task_header: bool,
    /// Where the `script` keyword of the statement being lexed starts, if we
    /// are in one. The body of a `script` block is raw text that no other rule
    /// in this lexer applies to, so the block has to be recognised here, on
    /// the way past — by the time the parser saw it, the body would already
    /// have been chopped into words.
    ///
    /// It holds the keyword's offset rather than a flag because the
    /// terminating `}` is found by indentation, and the indentation that
    /// counts is that of the line the keyword sits on. See
    /// [`Lexer::script_block`].
    script_kw: Option<usize>,
    /// Whether a word has followed that `script` yet. Without one there is no
    /// command to feed the block to, and reading `script { }` as a block would
    /// answer a missing command with a complaint about the body; leaving the
    /// braces ordinary lets the parser say what is actually missing.
    script_argv: bool,
    /// Open `{` blocks, so that only a `task` at the top level opens a header.
    /// Inside a body, `task` is an ordinary command name.
    depth: usize,
}

impl Lexer<'_> {
    fn run(mut self) -> Result<Vec<Spanned>> {
        while self.i < self.src.len() {
            match self.byte(self.i) {
                b' ' | b'\t' | b'\r' => {
                    self.i += 1;
                    // Every word of a task header may declare a default, so
                    // the space between them re-offers the `=` split.
                    self.stmt_start |= self.task_header;
                }
                b'\n' => {
                    self.punct(Token::Newline, 1);
                    self.stmt_start = true;
                    self.cmd_start = true;
                }
                b'#' => self.comment(),
                b'{' if self.script_argv => {
                    // `script cmd... {` — the brace opens raw text, not a block
                    // of statements.
                    let kw = self.script_kw.unwrap_or(self.i);
                    self.script_block(kw)?;
                }
                b'{' => {
                    self.punct(Token::LBrace, 1);
                    self.stmt_start = true;
                    self.cmd_start = true;
                    self.depth += 1;
                }
                b'}' => {
                    self.punct(Token::RBrace, 1);
                    self.stmt_start = true;
                    self.cmd_start = true;
                    self.depth = self.depth.saturating_sub(1);
                }
                b'(' => self.punct(Token::LParen, 1),
                b')' => self.punct(Token::RParen, 1),
                b'&' if self.peek(1) == Some(b'&') => self.punct(Token::AndAnd, 2),
                b'|' if self.peek(1) == Some(b'|') => self.punct(Token::OrOr, 2),
                b'|' => self.punct(Token::Pipe, 1),
                b'>' if self.peek(1) == Some(b'>') => self.punct(Token::GtGt, 2),
                b'>' => self.punct(Token::Gt, 1),
                b'2' if self.peek(1) == Some(b'>') => self.punct(Token::ErrGt, 2),
                b'!' if self.peek(1) != Some(b'=') => self.punct(Token::Bang, 1),
                b'^' => self.punct(Token::Caret, 1),
                b'&' => return syntax("`&` is not supported; use `&&`", self.span(self.i, 1)),
                b'<' => {
                    return syntax(
                        "`<` is not supported; only `>`, `>>` and `2>` redirect",
                        self.span(self.i, 1),
                    );
                }
                _ => self.word()?,
            }
        }
        self.push(Token::Eof, self.src.len(), self.src.len());
        Ok(self.out)
    }

    fn byte(&self, i: usize) -> u8 {
        self.src.as_bytes()[i]
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.src.as_bytes().get(self.i + ahead).copied()
    }

    fn span(&self, start: usize, len: usize) -> Span {
        Span::new(self.base + start, self.base + start + len)
    }

    fn push(&mut self, token: Token, start: usize, end: usize) {
        self.out.push(Spanned {
            token,
            span: Span::new(self.base + start, self.base + end),
        });
    }

    fn punct(&mut self, token: Token, len: usize) {
        let start = self.i;
        self.i += len;
        // The operators that join commands leave a command expected, so the
        // word after them may still be the `script` keyword.
        self.cmd_start = matches!(
            token,
            Token::AndAnd | Token::OrOr | Token::Pipe | Token::LParen | Token::Bang
        );
        self.push(token, start, start + len);
        self.stmt_start = false;
        self.task_header = false;
        // A `script` header runs from the keyword to the `{`, and holds only
        // words: anything else — a newline, an operator — ends it, and the
        // parser reports whatever that made ungrammatical.
        self.script_kw = None;
        self.script_argv = false;
    }

    fn comment(&mut self) {
        let start = self.i;
        let end = self.src[start..]
            .find('\n')
            .map_or(self.src.len(), |n| start + n);
        let text = self.src[start + 1..end].trim_end().to_string();
        self.push(Token::Comment(text), start, end);
        self.task_header = false;
        self.i = end;
    }

    /// Scan one word, stopping at whitespace or an operator. Quoted runs and
    /// `$(...)` are consumed whole, so the spaces inside them do not end it.
    fn word(&mut self) -> Result<()> {
        let start = self.i;
        let mut quoted = false;
        while self.i < self.src.len() {
            match self.byte(self.i) {
                b' ' | b'\t' | b'\r' | b'\n' => break,
                b'{' | b'}' | b'(' | b')' | b'|' | b'&' | b'>' | b'<' => break,
                b'\'' | b'"' => {
                    self.quoted_run()?;
                    quoted = true;
                }
                b'$' if self.peek(1) == Some(b'(') => self.capture_run()?,
                b'$' if self.peek(1) == Some(b'{') => self.braced_run()?,
                b'=' if self.stmt_start && !quoted && is_ident(&self.src[start..self.i]) => {
                    let text = self.src[start..self.i].to_string();
                    self.push(Token::Word { text, quoted }, start, self.i);
                    self.push(Token::Assign, self.i, self.i + 1);
                    self.i += 1;
                    // The value that follows is one word, `=` and all — and a
                    // word, not a command: the `script` in `x=$(script ...)`
                    // is found when that capture is lexed as its own fragment.
                    self.stmt_start = false;
                    self.cmd_start = false;
                    return Ok(());
                }
                _ => self.i += 1,
            }
        }
        let text = self.src[start..self.i].to_string();
        // A top-level `task` opens a header, where the words that follow are
        // parameters and may carry `=` defaults. It stays open until the `{`.
        self.task_header |= self.stmt_start && self.depth == 0 && text == "task" && !quoted;
        // `script` opens one too, and this word is either that keyword or one
        // of the argv words after it.
        match self.script_kw {
            Some(_) => self.script_argv = true,
            None if self.cmd_start && text == "script" && !quoted => {
                self.script_kw = Some(start);
            }
            None => {}
        }
        // A word ends the command position, except for the two keywords that
        // take a command of their own: `try script sh - { }` and `if script
        // sh - { }` both still open a block. Only a keyword that was itself in
        // command position counts, so `echo try` is an argument and a word.
        self.cmd_start = self.cmd_start && !quoted && matches!(text.as_str(), "try" | "if");
        self.push(Token::Word { text, quoted }, start, self.i);
        self.stmt_start = false;
        Ok(())
    }

    /// Consume the `{`, the raw body and the `}` of a `script` block, given
    /// the offset of the `script` keyword that opened it.
    ///
    /// The body is emitted as one [`Token::ScriptBody`] holding the source
    /// verbatim, so nothing in it is ever a token: quotes stay unbalanced, `<`
    /// and `&` stay ordinary characters, and a `#` is a comment in whatever
    /// language the block is written in rather than in this one. The rule that
    /// finds the closing brace is documented on
    /// [`Parser::script`](crate::parse) — it is the subtle part.
    fn script_block(&mut self, kw: usize) -> Result<()> {
        self.punct(Token::LBrace, 1);
        // The body starts on the next line, so that the closing `}` can be
        // recognised by its indentation and the first line by nothing at all.
        let Some(nl) = self.src[self.i..].find('\n').map(|n| self.i + n) else {
            return self.unterminated_script(kw);
        };
        if !self.src[self.i..nl].trim().is_empty() {
            return syntax(
                "a `script` block's body starts on the line after `{`; \
                 nothing else may follow the brace",
                self.span(self.i, nl - self.i),
            );
        }
        // Measured in the whole file, not in the fragment being lexed: see
        // [`Parser::script`](crate::parse) for why the line is the unit.
        let indent = line_indent(self.full, self.base + kw);
        let body_start = nl + 1;
        let mut pos = body_start;
        loop {
            if pos >= self.src.len() {
                return self.unterminated_script(kw);
            }
            let end = self.src[pos..]
                .find('\n')
                .map_or(self.src.len(), |n| pos + n);
            // The closing brace is the first `}` sitting at exactly the
            // keyword's indentation. A `}` the body owns is indented past it.
            if let Some(rest) = self.src[pos..end].strip_prefix(indent)
                && rest.starts_with('}')
            {
                let text = self.src[body_start..pos].to_string();
                self.push(Token::ScriptBody(text), body_start, pos);
                self.i = pos + indent.len();
                self.punct(Token::RBrace, 1);
                return Ok(());
            }
            pos = end + 1;
        }
    }

    fn unterminated_script<T>(&self, kw: usize) -> Result<T> {
        // Both are facts about the file, so a block opened inside a `$( )`
        // names the line it was written on rather than a line of the capture.
        let at = self.base + kw;
        let line = self.full[..at].bytes().filter(|&b| b == b'\n').count() + 1;
        let column = line_indent(self.full, at).chars().count() + 1;
        syntax(
            format!(
                "unterminated script block, opened at line {line}; it ends at the first \
                 `}}` written at column {column}, indented exactly like its `script`"
            ),
            self.span(kw, "script".len()),
        )
    }

    /// Consume a `'...'` or `"..."` run, including the delimiters.
    fn quoted_run(&mut self) -> Result<()> {
        let open = self.byte(self.i);
        let start = self.i;
        self.i += 1;
        while self.i < self.src.len() {
            match self.byte(self.i) {
                c if c == open => {
                    self.i += 1;
                    return Ok(());
                }
                b'\\' if open == b'"' => self.i += 2,
                b'$' if open == b'"' && self.peek(1) == Some(b'(') => self.capture_run()?,
                _ => self.i += 1,
            }
        }
        let kind = if open == b'"' { "double" } else { "single" };
        syntax(
            format!("unterminated {kind}-quoted string"),
            self.span(start, 1),
        )
    }

    /// Consume `$(...)`, tracking nested parentheses and quotes so a capture
    /// may itself contain captures.
    fn capture_run(&mut self) -> Result<()> {
        let start = self.i;
        self.i += 2;
        let mut depth = 1usize;
        while self.i < self.src.len() {
            match self.byte(self.i) {
                b'(' => {
                    depth += 1;
                    self.i += 1;
                }
                b')' => {
                    depth -= 1;
                    self.i += 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                b'\'' | b'"' => self.quoted_run()?,
                _ => self.i += 1,
            }
        }
        syntax("unterminated `$(`", self.span(start, 2))
    }

    /// Consume `${name}`, which is otherwise stopped by the `{`.
    fn braced_run(&mut self) -> Result<()> {
        let start = self.i;
        match self.src[self.i..].find('}') {
            Some(n) => {
                self.i += n + 1;
                Ok(())
            }
            None => syntax("unterminated `${`", self.span(start, 2)),
        }
    }
}

/// The leading whitespace of the line `at` sits on, up to `at` itself.
///
/// A `script` opened part way through a line — `task t { script sh - {`, or
/// `version=$(script uv run - {` — has only its line's indentation to be
/// measured by, which is what a reader closing the block by eye would line up
/// against too.
///
/// `at` is an offset in the whole file, never in a fragment: the leading
/// whitespace of a capture's first line belongs to the line, and the capture
/// starts after it.
pub(crate) fn line_indent(source: &str, at: usize) -> &str {
    let start = source[..at].rfind('\n').map_or(0, |n| n + 1);
    let end = source[start..at]
        .find(|c| c != ' ' && c != '\t')
        .map_or(at, |n| start + n);
    &source[start..end]
}

/// A shell identifier: the name half of an assignment, or a `$name`.
pub(crate) fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
