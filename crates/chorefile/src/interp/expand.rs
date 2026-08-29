//! Turning words into argv.
//!
//! A quoted word is always exactly one argument. An unquoted word splits on
//! the whitespace that *interpolation* introduced — never on whitespace the
//! source wrote literally, which the lexer already quoted away. There are no
//! arrays and no quoting inside a variable, so this is the whole of it.

use crate::ast::{Chain, PartKind, VarRef, Word};
use crate::error::{Error, Result};

use super::run::describe;
use super::{Dest, Flags, Interpreter, Mode};

impl Interpreter<'_> {
    /// Expand one word into argv entries.
    ///
    /// A quoted word is always exactly one entry. An unquoted word splits on
    /// whitespace that *interpolation* introduced — never on whitespace that
    /// was written literally in the source, which was already quoted away by
    /// the lexer.
    pub(super) fn expand(&mut self, word: &Word) -> Result<Vec<String>> {
        if word.quoted {
            return Ok(vec![self.expand_to_string(word)?]);
        }

        let mut fields: Vec<String> = vec![String::new()];
        for part in &word.parts {
            match &part.kind {
                PartKind::Literal(text) => fields.last_mut().unwrap().push_str(text),
                PartKind::Var(VarRef::All) => {
                    // Each argument stays one argv entry: it arrived as one,
                    // and re-splitting it would lose an argument the caller
                    // deliberately quoted.
                    self.touch_args("$@");
                    let args = self.frame().args.clone();
                    for arg in args {
                        push_field(&mut fields);
                        fields.last_mut().unwrap().push_str(&arg);
                        push_field(&mut fields);
                    }
                }
                PartKind::Var(other) => {
                    let text = self.var(other)?;
                    split_into(&mut fields, &text);
                }
                PartKind::Capture(chain) => {
                    let text = self.capture(chain)?;
                    split_into(&mut fields, &text);
                }
            }
        }
        // An interpolation that produced nothing contributes no argument, as
        // in sh: `remove $stale` with `stale` empty removes nothing.
        fields.retain(|f| !f.is_empty());
        Ok(fields)
    }

    /// Expand a word to exactly one string, whatever its quoting: for
    /// assignments, comparisons and redirect targets, where splitting would
    /// have nothing to split into.
    pub(super) fn expand_to_string(&mut self, word: &Word) -> Result<String> {
        let mut s = String::new();
        for part in &word.parts {
            match &part.kind {
                PartKind::Literal(text) => s.push_str(text),
                PartKind::Var(VarRef::All) => {
                    self.touch_args("$@");
                    s.push_str(&self.frame().args.join(" "));
                }
                PartKind::Var(other) => {
                    let text = self.var(other)?;
                    s.push_str(&text);
                }
                PartKind::Capture(chain) => {
                    let text = self.capture(chain)?;
                    s.push_str(&text);
                }
            }
        }
        Ok(s)
    }

    /// Read one variable, and — under `--dry` — carry its mark along.
    ///
    /// This is the whole of propagation. Every path that turns a `$name`,
    /// `$1` or `$@` into text comes through here or through `expand`'s two
    /// `$@` arms, so a value the preview invented marks whatever is computed
    /// from it without any of the callers having to know. `$#` is deliberately
    /// left out: a count of arguments is a fact about the call, not about the
    /// values in it.
    fn var(&mut self, var: &VarRef) -> Result<String> {
        match var {
            // `$env::NAME` is the machine, not the file: it is read through
            // the overlay first, so a value `env NAME value`, `env NAME=value`
            // or a `dotenv` put there is what it answers with, and the
            // process environment only when nothing did. Unset is an error
            // like any other undefined variable — the optional reading is
            // written `x=$(try env NAME)` or `if env NAME`, where the miss is
            // an answer rather than a hole in a command line.
            VarRef::Named(name) if crate::vars::env_ref(name).is_some() => {
                let name = crate::vars::env_ref(name).expect("checked by the guard");
                self.envs.get(name).ok_or_else(|| Error::Run {
                    message: format!(
                        "environment variable `{name}` is not set; where it may be absent, \
                         write `if env {name} {{ }}` or capture `$(try env {name})`"
                    ),
                })
            }
            VarRef::Named(name) => {
                let value = self
                    .lookup(name)
                    .ok_or_else(|| self.undefined(&format!("${name}")))?;
                self.touch_named(name);
                Ok(value)
            }
            VarRef::Positional(n) => {
                let value = self
                    .frame()
                    .args
                    .get(n.wrapping_sub(1))
                    .cloned()
                    .ok_or_else(|| self.undefined(&format!("${n}")))?;
                self.touch_args(&format!("${n}"));
                Ok(value)
            }
            VarRef::Count => Ok(self.frame().args.len().to_string()),
            VarRef::All => {
                self.touch_args("$@");
                Ok(self.frame().args.join(" "))
            }
        }
    }

    fn undefined(&self, name: &str) -> Error {
        let task = &self.frame().task;
        if task.is_empty() {
            Error::Run {
                message: format!("undefined variable `{name}`"),
            }
        } else {
            Error::Run {
                message: format!("undefined variable `{name}` in task `{task}`"),
            }
        }
    }

    /// `$(...)`: stdout, trimmed. It runs even under `--dry`.
    ///
    /// Under `--dry` a capture that failed yields the empty string with a note
    /// on stderr instead of ending the run: the command it asked may be one
    /// the preview could not carry out — a `read` of a file the skipped
    /// `download` would have fetched — and stopping there would cut the
    /// preview off at the step whose successors the author wanted to see. In
    /// `Mode::Run` a failed capture is still an error: there the value would
    /// be wrong, not merely unknown.
    fn capture(&mut self, chain: &Chain) -> Result<String> {
        let flags = Flags {
            echo: false,
            needed: true,
        };
        // Whatever inside the chain could not be evaluated, the *capture* is
        // what a mark has to name: it is the text the author wrote, and the
        // thing whose value is missing. So the innermost reason — a message
        // from `dry_failed`, a skipped block's command line — is replaced
        // here by the chain as written, once the whole capture has had its
        // say. An `unevaluated` from further out is left alone.
        let outer = self.unevaluated.take();
        let value = match self.chain(chain, Dest::Capture, None, flags) {
            Ok(out) if out.success() => Ok(out.captured()),
            Ok(out) if self.mode == Mode::Dry => {
                let code = out.code;
                self.dry_failed(&format!("capture failed with exit code {code}; using \"\""));
                Ok(String::new())
            }
            Ok(out) => Err(Error::Run {
                message: format!("capture failed with exit code {}", out.code),
            }),
            Err(e) => Err(e),
        };
        let inner = self.unevaluated.take();
        let blamed = inner.map(|_| describe(chain, &mut |w| self.preview(w)));
        self.unevaluated = outer.or(blamed);
        value
    }

    /// Preview a word for an error message, without running its captures.
    pub(super) fn preview(&self, word: &Word) -> String {
        word.parts
            .iter()
            .map(|p| match &p.kind {
                PartKind::Literal(text) => text.clone(),
                PartKind::Var(VarRef::Named(n)) => format!("${n}"),
                PartKind::Var(VarRef::Positional(n)) => format!("${n}"),
                PartKind::Var(VarRef::All) => "$@".into(),
                PartKind::Var(VarRef::Count) => "$#".into(),
                PartKind::Capture(_) => "$(...)".into(),
            })
            .collect()
    }
}

/// Start a new argv entry, unless the current one is still empty.
fn push_field(fields: &mut Vec<String>) {
    if !fields.last().map(String::is_empty).unwrap_or(true) {
        fields.push(String::new());
    }
}

/// Append interpolated text to an unquoted word, splitting on the whitespace
/// the interpolation itself introduced.
fn split_into(fields: &mut Vec<String>, text: &str) {
    if text.is_empty() {
        return;
    }
    if text.starts_with(char::is_whitespace) {
        push_field(fields);
    }
    let mut pieces = text.split_whitespace();
    if let Some(first) = pieces.next() {
        fields.last_mut().unwrap().push_str(first);
    }
    for piece in pieces {
        fields.push(piece.to_string());
    }
    if text.ends_with(char::is_whitespace) {
        push_field(fields);
    }
}
