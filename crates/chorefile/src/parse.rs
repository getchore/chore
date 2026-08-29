//! Tokens to an [`ast::File`](crate::ast::File).
//!
//! Hand-written recursive descent. Newlines separate statements, so the
//! grammar is line-oriented but never line-bound: a block may open on the line
//! of its header and close on the same line as its body, which is what makes
//! `if a { x } else { y }` legal.

use std::path::Path;

use crate::ast::{
    Assign, Block, Chain, Command, CompareOp, Cond, File, For, If, Include, Param, PartKind,
    Redirect, RedirectKind, Require, Script, Stmt, Task, VarRef, Word, WordPart,
};
use crate::error::{Error, Location, Result, Span};
use crate::lex::{self, Spanned, Token};

/// Parse one chorefile. `include` directives are recorded but not followed;
/// resolving them is the caller's job, so that cycle detection and namespacing
/// happen in one place.
pub fn parse(source: &str, file: &Path) -> Result<File> {
    let tokens = lex::lex(source).map_err(|e| locate(e, file))?;
    Parser {
        tokens,
        i: 0,
        file,
        src: source,
    }
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

/// Say which parameter of which task an error came out of, keeping its span.
///
/// A default is an ordinary word, so everything that can go wrong in a word can
/// go wrong here — and the reader is looking at a task header, where "expected
/// a default value after `=`" could belong to any of them. The prefix is added
/// on the way out rather than passed down, so the word parser stays unaware of
/// where it was called from.
fn in_param(error: Error, task: &str, param: &str) -> Error {
    match error {
        Error::Syntax { message, at } => Error::Syntax {
            message: format!("in parameter `{param}` of task `{task}`: {message}"),
            at,
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

/// What `2> file` beside `2>&1` is answered with. One command, two answers to
/// "where does stderr go": sh picks by written order, which is the part of
/// `2>&1` people get wrong, so chore refuses to pick at all.
const TWO_PLACES: &str = "a command sends stderr to one place: `2> file` and `2>&1` are both \
written here. Keep the file, or keep `2>&1` and let the `>` decide where both streams go";

/// The span of the redirect that made a command's stderr ambiguous, if one
/// did. Reported from the second operator, which is the one a reader would
/// delete.
fn split_stderr(redirects: &[Redirect]) -> Option<Span> {
    let mut seen: Option<RedirectKind> = None;
    for r in redirects {
        let kind = match r.kind {
            RedirectKind::Stderr | RedirectKind::StderrToStdout => r.kind,
            _ => continue,
        };
        match seen {
            Some(first) if first != kind => return Some(r.span),
            _ => seen = Some(kind),
        }
    }
    None
}

struct Parser<'a> {
    tokens: Vec<Spanned>,
    i: usize,
    file: &'a Path,
    /// The whole file. A `$( )` is re-lexed from the middle of it, and the
    /// lexer needs the file rather than the fragment to measure a `script`
    /// block's indentation by — see [`lex::lex_offset`].
    src: &'a str,
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
        // The description is the *first* line of the contiguous comment block
        // directly above a task. A block is a run of `#` lines with nothing
        // between them: a blank line, or any statement, ends it, which is what
        // keeps a file header separated by a blank line from becoming the first
        // task's description.
        //
        // First line rather than last because a block that says more than one
        // thing says the summary first and the caveats after it, and `list` has
        // room for one line. The last line of
        //
        //     # Run the app under the debugger.
        //     # In CI it skips the styling.
        //
        // is true of the task and useless as its name in a list.
        //
        // A blank `#` inside the block is skipped rather than treated as a
        // paragraph break: the rule is "the first non-empty line of the block",
        // which needs no second concept and reads the same from either end. A
        // block that is entirely blank leaves the task with no description at
        // all, since there was never a line to show.
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
                    if doc.is_none() && !text.trim().is_empty() {
                        doc = Some(first_sentence(&text));
                    }
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
                // Same reasoning as `return`: the keyword is spelled right and
                // reads as a stray word here, so saying where it belongs beats
                // the generic message and its guess about a stale binary.
                Token::Word { .. } if self.at_keyword("script") => {
                    return self.err(
                        "`script` is only valid inside a task; \
                         a chorefile runs nothing at the top level",
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
    ///
    /// One message covers every way of getting it wrong, including the ones
    /// the lexer sees first: `^1.4.0` opens with a `Caret` and `>=1.4.0` with
    /// a `Gt`, so neither is ever a word to parse, and answering those with
    /// "expected a version" would describe the token rather than the shape
    /// the author was reaching for.
    fn require(&mut self) -> Result<Require> {
        let start = self.bump().span.start;
        let at = self.span();
        let found = match self.kind() {
            Token::Word { .. } => {
                let word = self.word("a version after `require`")?;
                literal(&word)
                    .as_deref()
                    .and_then(crate::require::Version::parse)
                    .map(|version| (version, word.span.end))
            }
            _ => None,
        };
        let Some((version, end)) = found else {
            return self.err_at(
                "a `require` version must be written `<major>.<minor>.<patch>`, as in \
                 `require 1.4.0`; it means \"at least this\", so there are no ranges, \
                 operators, prereleases or `v` prefixes",
                at,
            );
        };
        Ok(Require {
            version,
            span: Span::new(start, end),
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
        let params = self.params(&name)?;
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
    ///
    /// Every error out of here names the parameter and the task. A header is
    /// the one place where a mistake lands the parser somewhere that has
    /// nothing to do with what went wrong: the list simply stops at the first
    /// token that is not a word, and `block` then complains that it expected
    /// `{`. "expected `{`, found `=`" is true and tells nobody which of four
    /// parameters is malformed, so the parameter's name is carried into the
    /// message rather than left for the reader to find by column number.
    fn params(&mut self, task: &str) -> Result<Vec<Param>> {
        let mut params: Vec<Param> = Vec::new();
        while let Token::Word { text, quoted } = self.kind().clone() {
            if quoted || !lex::is_ident(&text) {
                return self.err(format!(
                    "in the header of task `{task}`: parameter `{text}` must be a name — \
                     letters, digits and `_`, not starting with a digit; a default follows \
                     it as `name=value`"
                ));
            }
            let name_span = self.bump().span;
            let default = match self.kind() {
                Token::Assign => {
                    let eq = self.bump().span;
                    Some(self.default(eq).map_err(|e| in_param(e, task, &text))?)
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
        // The list ran out of words. `{` is what should be here, and a newline
        // before it is allowed, so those two go on to `block`, which reports a
        // missing body in the words it has always used. Anything else is a
        // broken parameter, and belongs to the header rather than to the body
        // that has not started yet.
        match self.kind() {
            Token::LBrace | Token::Newline | Token::Eof => {}
            other => {
                let after = match params.last() {
                    Some(p) => format!(", after parameter `{}`", p.name),
                    None => String::new(),
                };
                return self.err(format!(
                    "in the header of task `{task}`: expected a parameter name or `{{`, \
                     found {}{after}; a parameter is a name, optionally followed by `=` \
                     and one default value",
                    other.describe()
                ));
            }
        }
        Ok(params)
    }

    /// The default after a parameter's `=`, given the span of that `=`.
    ///
    /// `env=` with nothing after it is the empty string, exactly as the
    /// assignment `env=` is: one spelling of `name=`, one meaning. It still
    /// makes the parameter optional, and `env=""` says the same thing louder.
    ///
    /// The default has to be *touching* the `=`. The lexer keeps no whitespace,
    /// so `force=bin` and `force= bin` arrive as the same three tokens, and the
    /// only thing that tells them apart is whether the word starts exactly
    /// where the `=` ended. Without that test, `task install force= bin=/usr/local/bin`
    /// swallowed `bin` as `force`'s default and then met the `=` of `bin=` with
    /// "expected `{`" — the promise that `env=` is the empty string held only
    /// when nothing followed on the line.
    fn default(&mut self, eq: Span) -> Result<Word> {
        match self.kind() {
            Token::Word { .. } if self.span().start == eq.end => {
                self.word("a default value after `=`")
            }
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

    /// `script <command...> { <raw text> }`.
    ///
    /// **Where the block ends.** At the first line that begins with the
    /// indentation of the line the `script` keyword sits on and then a `}`.
    /// Everything before that line is the body, whatever it contains.
    ///
    /// The unit is the *line*, not the keyword's own column, and that is what
    /// makes the rule survive nesting. A block is a chain element, so the
    /// keyword need not start its line: `version=$(script uv run - {` opens
    /// one part way along, and the block still closes at
    ///
    /// ```sh
    /// task version {
    ///     version=$(script uv run - {
    ///         print("...")
    ///     })
    /// }
    /// ```
    ///
    /// — the `})` written where the `version=` was. Keying on the keyword's
    /// column would have demanded a `}` under the `s` of `script`, which no
    /// one writes and no editor indents to; keying on the line asks for the
    /// alignment the author already used, in a capture exactly as in a
    /// statement. Being a property of the line, it is also read out of the
    /// whole file rather than out of the `$( )` fragment the lexer is working
    /// on, which would have measured from the `(` and thought column zero.
    ///
    /// The body is text chore does not read, and real bodies are full of
    /// braces — a dict, an object literal, a JSON blob — so counting them is
    /// out: the first `}` inside a string would end the block early, and
    /// chore would have to know that language to know it was in a string.
    /// Ending at the first line that is only `}` is nearly as bad, because
    /// closing a dedented brace on its own line is exactly what such a body
    /// does. Indentation is what survives both: the block already sits inside
    /// a task, so its body is indented past its `script`, and a `}` the body
    /// owns is indented with the code it closes. A `}` back at the keyword's
    /// column is the one thing that cannot belong to the body — which is also
    /// how the reader picks it out, so the rule agrees with the eye.
    ///
    /// It costs one restriction, stated rather than inferred: a body line may
    /// not be outdented to the `script`'s own column. That is the price of
    /// never having to parse the body, and an explicit terminator the author
    /// picks would cost more — a second thing to invent, spell and get wrong
    /// in a language whose blocks are otherwise all braces.
    ///
    /// Unterminated blocks are caught while scanning, not two hundred lines
    /// later by whatever eventually failed to parse: reaching the end of the
    /// file without that `}` names the line the block opened on — the line in
    /// the file, so a block inside a capture is reported where it was written
    /// and not at the capture's first line.
    fn script(&mut self) -> Result<Script> {
        let kw = self.bump().span;
        let mut command = Vec::new();
        while matches!(self.kind(), Token::Word { .. }) {
            command.push(self.word("a command after `script`")?);
        }
        if command.is_empty() {
            return self.err_at(
                "`script` needs the command to run the block, as in \
                 `script python3 - { ... }`; the block is fed to it on stdin, \
                 so the command usually ends in whatever means \"read stdin\"",
                kw,
            );
        }
        self.expect(Token::LBrace, "`{` to open the script block")?;
        let body_tok = self.bump();
        let Token::ScriptBody(raw) = &body_tok.token else {
            // The lexer only opens a block once a command word has followed
            // the keyword, and always emits the three tokens together.
            return self.err_at("expected a script block", body_tok.span);
        };
        let body = dedent(raw);
        let end = self.expect(Token::RBrace, "`}` to close the script block")?;
        Ok(Script {
            command,
            body,
            redirects: self.redirects()?,
            // The keyword through the closing brace. A redirect after it is
            // not part of the block: it carries its own span, and this one is
            // what a diagnostic about the block should underline.
            span: Span::new(kw.start, end.span.end),
            body_span: body_tok.span,
        })
    }

    /// The `>`, `>>`, `2>` and `2>&1` redirections written after a script
    /// block, which take their targets exactly as a command's do.
    fn redirects(&mut self) -> Result<Vec<Redirect>> {
        let mut out = Vec::new();
        loop {
            let kind = match self.kind() {
                Token::Gt => RedirectKind::Stdout,
                Token::GtGt => RedirectKind::StdoutAppend,
                Token::ErrGt => RedirectKind::Stderr,
                Token::ErrToOut => RedirectKind::StderrToStdout,
                _ => {
                    if let Some(span) = split_stderr(&out) {
                        return self.err_at(TWO_PLACES, span);
                    }
                    return Ok(out);
                }
            };
            let op = self.bump().span;
            if kind == RedirectKind::StderrToStdout {
                out.push(Redirect {
                    kind,
                    target: None,
                    span: op,
                });
                continue;
            }
            let target = self.word("a file to redirect to")?;
            let span = Span::new(op.start, target.span.end);
            out.push(Redirect {
                kind,
                target: Some(target),
                span,
            });
        }
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
        let mut left = self.chain_atom()?;
        while self.eat(&Token::Pipe) {
            left = Chain::Pipe(Box::new(left), Box::new(self.chain_atom()?));
        }
        Ok(left)
    }

    /// One thing that runs: a command, or a `script` block.
    ///
    /// The two are alternatives at exactly this level, which is what makes a
    /// block compose — captured, piped, redirected, `&&`-ed — everywhere a
    /// command composes, and nowhere else. `script` is a keyword here rather
    /// than a command name: nothing on `PATH` can be reached by writing it,
    /// and `^script` is how you would reach one if it existed.
    fn chain_atom(&mut self) -> Result<Chain> {
        if self.at_keyword("script") {
            return Ok(Chain::Script(self.script()?));
        }
        Ok(Chain::Single(self.command()?))
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
                Token::ErrToOut => RedirectKind::StderrToStdout,
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
            if kind == RedirectKind::StderrToStdout {
                end = op.end;
                redirects.push(Redirect {
                    kind,
                    target: None,
                    span: op,
                });
                continue;
            }
            let target = self.word("a file to redirect to")?;
            end = target.span.end;
            redirects.push(Redirect {
                kind,
                target: Some(target),
                span: Span::new(op.start, end),
            });
        }
        if let Some(span) = split_stderr(&redirects) {
            return self.err_at(TWO_PLACES, span);
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
        let tokens = lex::lex_offset(source, self.src, base).map_err(|e| locate(e, self.file))?;
        let mut inner = Parser {
            tokens,
            i: 0,
            file: self.file,
            src: self.src,
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

/// Remove the indentation every non-blank line of a script body shares.
///
/// A block written inside a task is indented by the task, and that indentation
/// is chore's, not the body's: handing Python a uniformly indented program
/// gets an `IndentationError` for something the author never wrote. So the
/// longest whitespace prefix common to every non-blank line comes off, and
/// what remains — including the relative indentation, which is the body's own
/// and load-bearing — is untouched.
///
/// The prefix is compared byte for byte, so a tab is not four spaces. Two
/// lines indented differently agree only on what they literally share, which
/// is the same rule the other interpreter will apply to them.
///
/// A blank line need not carry the prefix, and loses whatever whitespace it
/// has: trailing spaces on an empty line are not indentation, and no language
/// downstream reads them as such.
fn dedent(body: &str) -> String {
    let common = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(indent_of)
        .reduce(common_prefix)
        .unwrap_or_default();
    if common.is_empty() {
        return body.to_string();
    }
    body.split_inclusive('\n')
        .map(|line| {
            line.strip_prefix(common)
                // Only a blank line can be shorter than the common prefix.
                .unwrap_or_else(|| line.trim_start_matches([' ', '\t']))
        })
        .collect()
}

/// The leading spaces and tabs of one line.
fn indent_of(line: &str) -> &str {
    let end = line.find(|c| c != ' ' && c != '\t').unwrap_or(line.len());
    &line[..end]
}

/// The longer prefix the two strings share, as bytes — safe to slice with,
/// because both are runs of spaces and tabs.
fn common_prefix<'a>(a: &'a str, b: &'a str) -> &'a str {
    let end = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    &a[..end]
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

/// The description stops at the end of its first sentence, terminator kept.
///
/// A listing has room for one line, and a comment that packs two sentences
/// into its first — `Type-check the workspace. Runs clippy too, so it is
/// slow.` — was written for the reader of the file, not the listing. The
/// summary is the first sentence; what follows is the caveat.
///
/// A sentence ends at `.`, `!` or `?` followed by whitespace or the end of
/// the line, so `1.4.0`, `target/debug` and `foo.bar` inside a sentence are
/// untouched. A period after a single letter does not count, which is what
/// keeps `e.g. aarch64-apple-darwin` and `i.e. the slow one` whole; the cost
/// is a sentence ending in a one-letter word, which is rare enough to pay.
fn first_sentence(line: &str) -> String {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if !matches!(b, b'.' | b'!' | b'?') {
            continue;
        }
        let ends_line = bytes
            .get(i + 1)
            .is_none_or(|next| next.is_ascii_whitespace());
        if !ends_line {
            continue;
        }
        // `e.g.`: the letter before this period is itself preceded by a
        // period, a space, or the start of the line.
        let abbreviation = b == b'.'
            && i >= 1
            && bytes[i - 1].is_ascii_alphabetic()
            && (i == 1 || matches!(bytes[i - 2], b'.' | b' '));
        if abbreviation {
            continue;
        }
        return line[..=i].to_string();
    }
    line.to_string()
}
