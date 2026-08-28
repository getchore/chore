//! Tokens to an [`ast::File`](crate::ast::File).
//!
//! Hand-written recursive descent. Newlines separate statements, so the
//! grammar is line-oriented but never line-bound: a block may open on the line
//! of its header and close on the same line as its body, which is what makes
//! `if a { x } else { y }` legal.

use std::path::Path;

use crate::ast::{
    Assign, Block, Chain, Command, CompareOp, Cond, File, For, If, Include, Param, PartKind,
    Redirect, RedirectKind, Require, Stmt, Task, VarRef, Word, WordPart,
};
use crate::error::{Error, Location, Result, Span};
use crate::lex::{self, Spanned, Token};

/// Parse one chorefile. `include` directives are recorded but not followed;
/// resolving them is the caller's job, so that cycle detection and namespacing
/// happen in one place.
pub fn parse(source: &str, file: &Path) -> Result<File> {
    let tokens = lex::lex(source).map_err(|e| locate(e, file))?;
    Parser { tokens, i: 0, file }
        .file()
        .map_err(|e| locate(e, file))
}

/// The lexer and the inner parsers report spans without a path; only [`parse`]
/// knows the file the source came from.
fn locate(error: Error, file: &Path) -> Error {
    match error {
        Error::Syntax { message, at } => Error::Syntax {
            message,
            at: Location {
                file: file.to_path_buf(),
                span: at.span,
            },
        },
        other => other,
    }
}

/// Words that read as operators in a condition rather than as arguments.
const COMPARE_OPS: [(&str, CompareOp); 5] = [
    ("==", CompareOp::Eq),
    ("!=", CompareOp::Ne),
    ("contains", CompareOp::Contains),
    ("starts-with", CompareOp::StartsWith),
    ("ends-with", CompareOp::EndsWith),
];

struct Parser<'a> {
    tokens: Vec<Spanned>,
    i: usize,
    file: &'a Path,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Spanned {
        // `lex` always terminates the stream with `Eof`, so this cannot fail.
        &self.tokens[self.i.min(self.tokens.len() - 1)]
    }

    fn kind(&self) -> &Token {
        &self.peek().token
    }

    fn span(&self) -> Span {
        self.peek().span
    }

    /// The end of the last token consumed — where a header stops.
    ///
    /// A header's own end is not the start of what follows: `if a || b\n{`
    /// has a newline in between, and `if (a)` ends at a `)` that the tree does
    /// not keep. Measuring backwards from the parser's position gets both.
    fn prev_end(&self) -> usize {
        self.tokens[self.i.saturating_sub(1)].span.end
    }

    fn bump(&mut self) -> Spanned {
        let t = self.peek().clone();
        if self.i < self.tokens.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.kind() == token {
            self.bump();
            true
        } else {
            false
        }
    }

    /// A bare (unquoted) word with exactly this text — how keywords, `in`,
    /// `as` and the comparison operators are recognised.
    fn at_keyword(&self, word: &str) -> bool {
        matches!(self.kind(), Token::Word { text, quoted: false } if text == word)
    }

    fn err<T>(&self, message: impl Into<String>) -> Result<T> {
        self.err_at(message, self.span())
    }

    fn err_at<T>(&self, message: impl Into<String>, span: Span) -> Result<T> {
        Err(self.error_at(message, span))
    }

    fn error(&self, message: impl Into<String>) -> Error {
        self.error_at(message, self.span())
    }

    fn error_at(&self, message: impl Into<String>, span: Span) -> Error {
        Error::Syntax {
            message: message.into(),
            at: Location {
                file: self.file.to_path_buf(),
                span,
            },
        }
    }

    fn expect(&mut self, token: Token, what: &str) -> Result<Spanned> {
        if self.kind() == &token {
            Ok(self.bump())
        } else {
            self.err(format!("expected {what}, found {}", self.kind().describe()))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.kind(), Token::Newline | Token::Comment(_)) {
            self.bump();
        }
    }

    // --- file -----------------------------------------------------------

    fn file(mut self) -> Result<File> {
        let mut file = File::default();
        // The comment line directly above a task becomes its description; a
        // blank line in between breaks the association.
        let mut doc: Option<String> = None;
        loop {
            match self.kind().clone() {
                Token::Eof => break,
                Token::Newline => {
                    self.bump();
                    doc = None;
                }
                Token::Comment(text) => {
                    let text = strip_doc(&text);
                    self.bump();
                    self.eat(&Token::Newline);
                    doc = Some(text);
                }
                Token::Word { .. } if self.at_keyword("task") => {
                    file.tasks.push(self.task(doc.take())?);
                    self.end_of_stmt()?;
                }
                Token::Word { .. } if self.at_keyword("require") => {
                    doc = None;
                    let require = self.require()?;
                    // Two requirements are two answers to one question, and
                    // the file cannot say which it means, so neither can we.
                    // Both versions are named: whichever the author meant to
                    // delete, the message has told them where the other is.
                    if let Some(first) = &file.require {
                        return self.err_at(
                            format!(
                                "a chorefile may state only one `require`; this file \
                                 already requires {}, and this line requires {}",
                                first.version, require.version
                            ),
                            require.span,
                        );
                    }
                    file.require = Some(require);
                    self.end_of_stmt()?;
                }
                Token::Word { .. } if self.at_keyword("include") => {
                    doc = None;
                    file.includes.push(self.include()?);
                    self.end_of_stmt()?;
                }
                Token::Word { .. } if self.tokens[self.i + 1].token == Token::Assign => {
                    doc = None;
                    file.globals.push(self.assign()?);
                    self.end_of_stmt()?;
                }
                // `return` leaves a task, and at the top level there is no
                // task to leave. Saying so beats the generic "expected a task,
                // an assignment or an include", which leaves the reader to
                // guess that `exit` is the statement they wanted.
                Token::Word { .. } if self.at_keyword("return") => {
                    return self.err(
                        "`return` is only valid inside a task; \
                         at the top level, use `exit` to end the run",
                    );
                }
                ref other => {
                    // A statement this build does not know reads here as a
                    // stray word, which is the one confusion `require` cannot
                    // fix for itself: the binary that needs telling is the one
                    // too old to have the keyword. A word is either a typo or
                    // a form from a later `chore`, so the message offers both.
                    let hint = match other {
                        Token::Word { .. } => {
                            "; if it is a newer chorefile keyword, this `chore` may be too old \
                             (`chore --version`)"
                        }
                        _ => "",
                    };
                    return self.err(format!(
                        "expected a task, an assignment or an include at the top level, found {}{hint}",
                        other.describe()
                    ));
                }
            }
        }
        Ok(file)
    }

    /// `require major.minor.patch`, the whole grammar of the directive.
    ///
    /// Strict, and deliberately so: the version is a floor, so an operator or
    /// a range would be asking a question this language has no vocabulary to
    /// answer. Rejecting them here, where the shape can be shown, beats
    /// accepting `^1.4.0` and quietly meaning something else by it.
    fn require(&mut self) -> Result<Require> {
        let start = self.bump().span.start;
        let word = self.word("a version after `require`")?;
        let version = literal(&word)
            .as_deref()
            .and_then(crate::require::Version::parse);
        let Some(version) = version else {
            return self.err_at(
                "a `require` version must be written `<major>.<minor>.<patch>`, as in \
                 `require 1.4.0`; it means \"at least this\", so there are no ranges, \
                 operators, prereleases or `v` prefixes",
                word.span,
            );
        };
        Ok(Require {
            version,
            span: Span::new(start, word.span.end),
        })
    }

    fn include(&mut self) -> Result<Include> {
        let start = self.bump().span.start;
        let path_word = self.word("a path after `include`")?;
        let Some(path) = literal(&path_word) else {
            return self.err_at(
                "an include path must be a plain path, without interpolation",
                path_word.span,
            );
        };
        let mut end = path_word.span.end;
        let mut namespace = None;
        if self.at_keyword("as") {
            self.bump();
            let ns = self.word("a namespace after `as`")?;
            let name = literal(&ns).filter(|n| lex::is_ident(n));
            match name {
                Some(name) => namespace = Some(name),
                None => {
                    return self.err_at("an include namespace must be a name", ns.span);
                }
            }
            end = ns.span.end;
        }
        Ok(Include {
            path,
            namespace,
            span: Span::new(start, end),
        })
    }

    fn task(&mut self, doc: Option<String>) -> Result<Task> {
        let start = self.bump().span.start;
        let name_tok = self.bump();
        let name = match &name_tok.token {
            Token::Word {
                text,
                quoted: false,
            } if !text.contains('$') => text.clone(),
            other => {
                return self.err_at(
                    format!("expected a task name, found {}", other.describe()),
                    name_tok.span,
                );
            }
        };
        let params = self.params()?;
        let (body, end) = self.block()?;
        Ok(Task {
            name,
            params,
            doc,
            body,
            span: Span::new(start, end),
        })
    }

    /// The parameter list of a task header: names, each optionally followed by
    /// `=` and a default word.
    ///
    /// Required parameters must come before optional ones. A required
    /// parameter after an optional one is unreachable — the caller's arguments
    /// are positional, so supplying the required one means supplying the
    /// optional one too, and the default could never apply. Rejecting it here
    /// turns a promise no call can honour into one syntax error, rather than a
    /// confusing arity complaint at every call site.
    ///
    /// A default is left as a [`Word`]: quoting, `$name` and `$( )` all work,
    /// and the interpreter evaluates it at call time, in the called task's
    /// scope, only when the caller left the parameter out.
    fn params(&mut self) -> Result<Vec<Param>> {
        let mut params: Vec<Param> = Vec::new();
        while let Token::Word { text, quoted } = self.kind().clone() {
            if quoted || !lex::is_ident(&text) {
                return self.err(format!("task parameter `{text}` must be a name"));
            }
            let name_span = self.bump().span;
            let default = match self.kind() {
                Token::Assign => {
                    let eq = self.bump().span;
                    Some(self.default(eq)?)
                }
                _ => None,
            };
            let end = default.as_ref().map_or(name_span.end, |w| w.span.end);
            let param = Param {
                name: text,
                default,
                span: Span::new(name_span.start, end),
            };
            if param.required()
                && let Some(optional) = params.iter().find(|p| !p.required())
            {
                return self.err_at(
                    format!(
                        "required parameter `{}` cannot follow optional parameter `{}`; \
                         arguments are positional, so anything after an optional parameter \
                         can only be reached by supplying that one too — \
                         give `{}` a default, or declare it before `{}`",
                        param.name, optional.name, param.name, optional.name
                    ),
                    param.span,
                );
            }
            params.push(param);
        }
        Ok(params)
    }

    /// The default after a parameter's `=`, given the span of that `=`.
    ///
    /// `env=` with nothing after it is the empty string, exactly as the
    /// assignment `env=` is: one spelling of `name=`, one meaning. It still
    /// makes the parameter optional, and `env=""` says the same thing louder.
    fn default(&mut self, eq: Span) -> Result<Word> {
        match self.kind() {
            Token::Word { .. } => self.word("a default value after `=`"),
            _ => Ok(Word {
                parts: Vec::new(),
                quoted: true,
                span: Span::new(eq.end, eq.end),
            }),
        }
    }

    // --- statements -----------------------------------------------------

    /// A `{ ... }` block. Returns the statements and the offset just past the
    /// closing brace, for the enclosing node's span.
    fn block(&mut self) -> Result<(Block, usize)> {
        while matches!(self.kind(), Token::Newline) {
            self.bump();
        }
        let open = self.expect(Token::LBrace, "`{`")?.span;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            match self.kind() {
                Token::RBrace => {
                    let end = self.bump().span.end;
                    return Ok((stmts, end));
                }
                Token::Eof => return self.err_at("unclosed `{`", open),
                _ => {}
            }
            stmts.push(self.stmt()?);
            self.end_of_stmt()?;
        }
    }

    /// Statements end at a newline; a closing brace or end of file ends the
    /// last one, so `{ x }` needs no separator.
    fn end_of_stmt(&mut self) -> Result<()> {
        if matches!(self.kind(), Token::Comment(_)) {
            self.bump();
        }
        match self.kind() {
            Token::Newline => {
                self.bump();
                Ok(())
            }
            Token::RBrace | Token::Eof => Ok(()),
            other => Err(self.error(format!(
                "expected end of line, found {}; statements are separated by newlines",
                other.describe()
            ))),
        }
    }

    fn stmt(&mut self) -> Result<Stmt> {
        if self.at_keyword("if") {
            return Ok(Stmt::If(self.if_stmt()?));
        }
        if self.at_keyword("for") {
            return Ok(Stmt::For(self.for_stmt()?));
        }
        if self.at_keyword("try") {
            self.bump();
            return Ok(Stmt::Try(self.chain()?));
        }
        if self.at_keyword("exit") {
            self.bump();
            return Ok(Stmt::Exit(self.status_code("an exit code")?));
        }
        if self.at_keyword("return") {
            self.bump();
            return Ok(Stmt::Return(self.status_code("a return code")?));
        }
        // `require` states what the *file* needs, so a task body is never the
        // place for it: a requirement that only applied once a particular task
        // ran would be checked after the parse it exists to explain.
        if self.at_keyword("require") {
            return self.err(
                "`require` is only valid at the top level, conventionally as the first \
                 line; it states the oldest `chore` that can run the whole file, not one task",
            );
        }
        if matches!(self.kind(), Token::Word { .. })
            && self.tokens[self.i + 1].token == Token::Assign
        {
            return Ok(Stmt::Assign(self.assign()?));
        }
        Ok(Stmt::Command(self.chain()?))
    }

    /// The optional code after `exit` or `return`. Both take one at most, and
    /// a bare keyword — the common case — means zero.
    fn status_code(&mut self, what: &str) -> Result<Option<Word>> {
        match matches!(self.kind(), Token::Word { .. }) {
            true => Ok(Some(self.word(what)?)),
            false => Ok(None),
        }
    }

    fn assign(&mut self) -> Result<Assign> {
        let name_tok = self.bump();
        let Token::Word { text, .. } = &name_tok.token else {
            return self.err_at("expected a name", name_tok.span);
        };
        let name = text.clone();
        let eq = self.bump().span;
        // `x=` with nothing after it assigns the empty string.
        let value = match self.kind() {
            Token::Word { .. } => self.word("a value")?,
            _ => Word {
                parts: Vec::new(),
                quoted: true,
                span: Span::new(eq.end, eq.end),
            },
        };
        let span = Span::new(name_tok.span.start, value.span.end);
        Ok(Assign { name, value, span })
    }

    fn if_stmt(&mut self) -> Result<If> {
        let start = self.bump().span.start;
        let cond = self.cond()?;
        let span = Span::new(start, self.prev_end());
        let (then, _) = self.block()?;
        // `else` may sit on the line after the closing brace, as in the spec's
        // aligned `if`/`else` pairs, so look past newlines before giving up.
        let save = self.i;
        self.skip_newlines();
        if !self.at_keyword("else") {
            self.i = save;
            return Ok(If {
                cond,
                then,
                otherwise: None,
                span,
            });
        }
        self.bump();
        let otherwise = if self.at_keyword("if") {
            vec![Stmt::If(self.if_stmt()?)]
        } else {
            self.block()?.0
        };
        Ok(If {
            cond,
            then,
            otherwise: Some(otherwise),
            span,
        })
    }

    fn for_stmt(&mut self) -> Result<For> {
        let start = self.bump().span.start;
        let var_tok = self.bump();
        let var = match &var_tok.token {
            Token::Word {
                text,
                quoted: false,
            } if lex::is_ident(text) => text.clone(),
            other => {
                return self.err_at(
                    format!("expected a loop variable name, found {}", other.describe()),
                    var_tok.span,
                );
            }
        };
        if !self.at_keyword("in") {
            return self.err(format!("expected `in`, found {}", self.kind().describe()));
        }
        self.bump();
        let mut items = Vec::new();
        while matches!(self.kind(), Token::Word { .. }) {
            items.push(self.word("a loop item")?);
        }
        let span = Span::new(start, self.prev_end());
        let (body, _) = self.block()?;
        Ok(For {
            var,
            items,
            body,
            span,
        })
    }

    // --- conditions -----------------------------------------------------

    fn cond(&mut self) -> Result<Cond> {
        let mut left = self.cond_and()?;
        while self.eat(&Token::OrOr) {
            left = Cond::Or(Box::new(left), Box::new(self.cond_and()?));
        }
        Ok(left)
    }

    fn cond_and(&mut self) -> Result<Cond> {
        let mut left = self.cond_not()?;
        while self.eat(&Token::AndAnd) {
            left = Cond::And(Box::new(left), Box::new(self.cond_not()?));
        }
        Ok(left)
    }

    fn cond_not(&mut self) -> Result<Cond> {
        if self.eat(&Token::Bang) {
            return Ok(Cond::Not(Box::new(self.cond_not()?)));
        }
        self.cond_atom()
    }

    fn cond_atom(&mut self) -> Result<Cond> {
        if self.eat(&Token::LParen) {
            let inner = self.cond()?;
            self.expect(Token::RParen, "`)`")?;
            return Ok(inner);
        }
        if let Some(op) = self.compare_op_ahead() {
            let left = self.word("a value")?;
            self.bump();
            let right = self.word("a value to compare against")?;
            return Ok(Cond::Compare { left, op, right });
        }
        // Anything else is a command, true when it exits zero. `&&` and `||`
        // belong to the condition, so only `|` may appear inside it.
        Ok(Cond::Command(self.pipeline()?))
    }

    /// The comparison operator after the next word, if that word is followed
    /// by one.
    fn compare_op_ahead(&self) -> Option<CompareOp> {
        if !matches!(self.kind(), Token::Word { .. }) {
            return None;
        }
        let Token::Word {
            text,
            quoted: false,
        } = &self.tokens.get(self.i + 1)?.token
        else {
            return None;
        };
        COMPARE_OPS
            .iter()
            .find(|(name, _)| name == text)
            .map(|(_, op)| *op)
    }

    // --- commands -------------------------------------------------------

    fn chain(&mut self) -> Result<Chain> {
        let mut left = self.pipeline()?;
        loop {
            if self.eat(&Token::AndAnd) {
                left = Chain::And(Box::new(left), Box::new(self.pipeline()?));
            } else if self.eat(&Token::OrOr) {
                left = Chain::Or(Box::new(left), Box::new(self.pipeline()?));
            } else {
                return Ok(left);
            }
        }
    }

    /// `|` binds tighter than `&&` and `||`, as in sh.
    fn pipeline(&mut self) -> Result<Chain> {
        let mut left = Chain::Single(self.command()?);
        while self.eat(&Token::Pipe) {
            left = Chain::Pipe(Box::new(left), Box::new(Chain::Single(self.command()?)));
        }
        Ok(left)
    }

    fn command(&mut self) -> Result<Command> {
        let start = self.span().start;
        let force_path = self.eat(&Token::Caret);
        if !matches!(self.kind(), Token::Word { .. }) {
            return self.err(format!(
                "expected a command, found {}",
                self.kind().describe()
            ));
        }
        let name = self.word("a command")?;
        let mut end = name.span.end;
        let mut args = Vec::new();
        let mut redirects = Vec::new();
        loop {
            let kind = match self.kind().clone() {
                Token::Word { .. } => {
                    let arg = self.word("an argument")?;
                    end = arg.span.end;
                    args.push(arg);
                    continue;
                }
                Token::Gt => RedirectKind::Stdout,
                Token::GtGt => RedirectKind::StdoutAppend,
                Token::ErrGt => RedirectKind::Stderr,
                Token::Caret => {
                    return self.err("`^` may only prefix a command name");
                }
                Token::LParen => {
                    return self.err("unexpected `(`; grouping is only allowed in a condition");
                }
                Token::Bang => return self.err("unexpected `!`; `!` negates a condition"),
                Token::Assign => return self.err("unexpected `=`"),
                _ => break,
            };
            let op = self.bump().span;
            let target = self.word("a file to redirect to")?;
            end = target.span.end;
            redirects.push(Redirect {
                kind,
                target,
                span: Span::new(op.start, end),
            });
        }
        Ok(Command {
            name,
            force_path,
            args,
            redirects,
            span: Span::new(start, end),
        })
    }

    // --- words ----------------------------------------------------------

    fn word(&mut self, what: &str) -> Result<Word> {
        let tok = self.bump();
        let Token::Word { text, quoted } = &tok.token else {
            return self.err_at(
                format!("expected {what}, found {}", tok.token.describe()),
                tok.span,
            );
        };
        let parts = self.word_parts(text, tok.span.start)?;
        Ok(Word {
            parts,
            quoted: *quoted,
            span: tok.span,
        })
    }

    /// Split a word's verbatim source into literals, variables and captures.
    ///
    /// `text` is a byte-for-byte copy of `source[base..]`, so every index in it
    /// is an offset in the original file.
    fn word_parts(&self, text: &str, base: usize) -> Result<Vec<WordPart>> {
        let b = text.as_bytes();
        let mut out = Parts::default();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                // Single quotes are literal: no interpolation, no escapes. The
                // spec only names `"..."`, but a shell-shaped language that
                // silently interpolated `'...'` would be a trap.
                b'\'' => {
                    let end = find(b, i + 1, b'\'');
                    // An unterminated `'` runs to the end of the word.
                    let close = (end + 1).min(b.len());
                    out.push_text(&text[i + 1..end], Span::new(base + i, base + close));
                    i = end + 1;
                }
                b'"' => {
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        match b[i] {
                            // Only `\` `"` and `$` are escapes, as in sh; any
                            // other backslash is an ordinary character.
                            b'\\' if matches!(b.get(i + 1), Some(b'\\' | b'"' | b'$')) => {
                                out.push_text(
                                    &text[i + 1..i + 2],
                                    Span::new(base + i, base + i + 2),
                                );
                                i += 2;
                            }
                            b'$' => i = self.dollar(text, base, i, &mut out)?,
                            _ => i = push_char(text, base, i, &mut out),
                        }
                    }
                    i += 1;
                }
                b'$' => i = self.dollar(text, base, i, &mut out)?,
                _ => i = push_char(text, base, i, &mut out),
            }
        }
        Ok(out.finish())
    }

    /// One `$...` form, starting at `i`. Returns the index just past it.
    fn dollar(&self, text: &str, base: usize, i: usize, out: &mut Parts) -> Result<usize> {
        let b = text.as_bytes();
        let at = |n: usize| b.get(n).copied();
        match at(i + 1) {
            Some(b'(') => {
                let close = capture_end(b, i).ok_or(()).or_else(|()| {
                    self.err_at("unterminated `$(`", Span::new(base + i, base + i + 2))
                })?;
                let inner = &text[i + 2..close];
                if inner.trim().is_empty() {
                    return self.err_at("empty `$( )`", Span::new(base + i, base + close + 1));
                }
                out.flush();
                out.parts.push(WordPart::new(
                    PartKind::Capture(Box::new(self.sub_chain(inner, base + i + 2)?)),
                    Span::new(base + i, base + close + 1),
                ));
                Ok(close + 1)
            }
            Some(b'{') => {
                let close = find(b, i + 2, b'}');
                if close >= b.len() {
                    return self.err_at("unterminated `${`", Span::new(base + i, base + i + 2));
                }
                let name = &text[i + 2..close];
                let span = Span::new(base + i, base + close + 1);
                let var = self.var_ref(name, span)?;
                out.push_var(var, span);
                Ok(close + 1)
            }
            Some(b'@') => {
                out.push_var(VarRef::All, Span::new(base + i, base + i + 2));
                Ok(i + 2)
            }
            Some(b'#') => {
                out.push_var(VarRef::Count, Span::new(base + i, base + i + 2));
                Ok(i + 2)
            }
            // `$1x` is parameter 1 followed by a literal `x`, so a numbered
            // parameter stops at the first non-digit.
            Some(c) if c.is_ascii_digit() => {
                let end = scan(b, i + 1, |c| c.is_ascii_digit());
                let span = Span::new(base + i, base + end);
                let var = self.var_ref(&text[i + 1..end], span)?;
                out.push_var(var, span);
                Ok(end)
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let end = scan(b, i + 1, |c| c.is_ascii_alphanumeric() || c == b'_');
                let span = Span::new(base + i, base + end);
                let var = self.var_ref(&text[i + 1..end], span)?;
                out.push_var(var, span);
                Ok(end)
            }
            // A `$` before anything else — `$ ` or end of word — is literal.
            _ => {
                out.push_text("$", Span::new(base + i, base + i + 1));
                Ok(i + 1)
            }
        }
    }

    fn var_ref(&self, name: &str, span: Span) -> Result<VarRef> {
        match name {
            "" => self.err_at("empty variable name", span),
            "@" => Ok(VarRef::All),
            "#" => Ok(VarRef::Count),
            _ if name.bytes().all(|c| c.is_ascii_digit()) => match name.parse::<usize>() {
                Ok(0) => self.err_at("parameters are numbered from `$1`", span),
                Ok(n) => Ok(VarRef::Positional(n)),
                Err(_) => self.err_at("parameter number is too large", span),
            },
            _ if lex::is_ident(name) => Ok(VarRef::Named(name.to_string())),
            _ => self.err_at(format!("`{name}` is not a variable name"), span),
        }
    }

    /// Parse the inside of a `$(...)`, keeping spans in the enclosing file.
    fn sub_chain(&self, source: &str, base: usize) -> Result<Chain> {
        let tokens = lex::lex_offset(source, base).map_err(|e| locate(e, self.file))?;
        let mut inner = Parser {
            tokens,
            i: 0,
            file: self.file,
        };
        let chain = inner.chain()?;
        match inner.kind() {
            Token::Eof => Ok(chain),
            other => inner.err(format!("unexpected {} inside `$( )`", other.describe())),
        }
    }
}

/// A word under construction: literal text accumulates until a variable or a
/// capture forces it out as its own part.
///
/// The accumulated literal grows its span as it goes, so a run written as
/// `a"b"c` — three source fragments, one part — still reports the whole run.
#[derive(Default)]
struct Parts {
    parts: Vec<WordPart>,
    lit: String,
    lit_span: Option<Span>,
}

impl Parts {
    /// Add decoded text, `span` being the source it was decoded from: an
    /// escape is two bytes of source and one character of text.
    fn push_text(&mut self, text: &str, span: Span) {
        self.lit.push_str(text);
        self.lit_span = Some(match self.lit_span {
            Some(so_far) => Span::new(so_far.start, span.end),
            None => span,
        });
    }

    fn flush(&mut self) {
        let span = self.lit_span.take();
        if !self.lit.is_empty() {
            let text = std::mem::take(&mut self.lit);
            // `lit` is only non-empty after a `push_text`, which always sets
            // the span.
            let span = span.unwrap_or(Span::new(0, 0));
            self.parts
                .push(WordPart::new(PartKind::Literal(text), span));
        }
    }

    fn push_var(&mut self, var: VarRef, span: Span) {
        self.flush();
        self.parts.push(WordPart::new(PartKind::Var(var), span));
    }

    fn finish(mut self) -> Vec<WordPart> {
        self.flush();
        self.parts
    }
}

/// Append the whole UTF-8 character at `i`, returning the next index.
fn push_char(text: &str, base: usize, i: usize, out: &mut Parts) -> usize {
    match text[i..].chars().next() {
        Some(c) => {
            let end = i + c.len_utf8();
            out.push_text(&text[i..end], Span::new(base + i, base + end));
            end
        }
        None => i + 1,
    }
}

/// The end of the run of bytes from `from` that satisfy `keep`.
fn scan(b: &[u8], from: usize, keep: impl Fn(u8) -> bool) -> usize {
    (from..b.len()).find(|&n| !keep(b[n])).unwrap_or(b.len())
}

/// The index of the next `needle`, or the end of the slice.
fn find(b: &[u8], from: usize, needle: u8) -> usize {
    (from..b.len()).find(|&n| b[n] == needle).unwrap_or(b.len())
}

/// The index of the `)` closing the `$(` at `start`, skipping nested
/// parentheses and quoted runs.
fn capture_end(b: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            q @ (b'\'' | b'"') => {
                i = find(b, i + 1, q);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The whole word as plain text, when it holds no interpolation.
fn literal(word: &Word) -> Option<String> {
    match word.parts.as_slice() {
        [] => Some(String::new()),
        [part] => match &part.kind {
            PartKind::Literal(text) => Some(text.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A doc comment loses its `#` and one following space, so `# Build` reads as
/// `Build` while an ASCII-art `#---` keeps its shape.
fn strip_doc(text: &str) -> String {
    text.strip_prefix(' ').unwrap_or(text).to_string()
}
