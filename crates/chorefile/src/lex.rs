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
    lex_offset(source, 0)
}

/// Tokenize a fragment whose first byte sits at `base` in the enclosing file,
/// so tokens inside a `$(...)` still point at the original source.
pub(crate) fn lex_offset(source: &str, base: usize) -> Result<Vec<Spanned>> {
    Lexer {
        src: source,
        base,
        i: 0,
        out: Vec::new(),
        stmt_start: true,
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
    base: usize,
    i: usize,
    out: Vec<Spanned>,
    /// Whether the next word could start a statement. `name=value` only splits
    /// into `Word` `Assign` `Word` there, so `cmake -DFOO=ON` and `a=b=c` keep
    /// their `=` inside the word.
    stmt_start: bool,
}

impl Lexer<'_> {
    fn run(mut self) -> Result<Vec<Spanned>> {
        while self.i < self.src.len() {
            match self.byte(self.i) {
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'\n' => {
                    self.punct(Token::Newline, 1);
                    self.stmt_start = true;
                }
                b'#' => self.comment(),
                b'{' => {
                    self.punct(Token::LBrace, 1);
                    self.stmt_start = true;
                }
                b'}' => {
                    self.punct(Token::RBrace, 1);
                    self.stmt_start = true;
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
        self.push(token, start, start + len);
        self.stmt_start = false;
    }

    fn comment(&mut self) {
        let start = self.i;
        let end = self.src[start..]
            .find('\n')
            .map_or(self.src.len(), |n| start + n);
        let text = self.src[start + 1..end].trim_end().to_string();
        self.push(Token::Comment(text), start, end);
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
                    self.stmt_start = false;
                    return Ok(());
                }
                _ => self.i += 1,
            }
        }
        let text = self.src[start..self.i].to_string();
        self.push(Token::Word { text, quoted }, start, self.i);
        self.stmt_start = false;
        Ok(())
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

/// A shell identifier: the name half of an assignment, or a `$name`.
pub(crate) fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
