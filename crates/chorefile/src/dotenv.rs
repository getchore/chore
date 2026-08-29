//! Reading a `.env` file.
//!
//! A hand-written parser for the one format everybody already has in their
//! repository, and deliberately the *small* version of it. What is here is
//! what every implementation agrees on: `KEY=VALUE` a line at a time, blank
//! lines and `#` comments, an optional `export ` in front, and three ways of
//! writing the value — bare, `'single'`, `"double"`.
//!
//! **There is no `${OTHER}` expansion.** The dialects disagree about it —
//! whether it reads the file or the process, what an unset name expands to,
//! how to escape the `$` — and a chorefile that needs a value built out of
//! another one has a language to build it in. A `$` in a value is a `$`.
//!
//! Errors name the file and the line. A `.env` is usually untracked and
//! hand-edited, so "line 7" is the whole of what a reader needs to fix it,
//! and a malformed line is never quietly skipped: a run that silently
//! ignored `DATBASE_URL "postgres://..."` would go on to fail somewhere with
//! nothing to connect the two.

use std::path::Path;

use crate::error::{Error, Result};
use crate::{lex, vars};

/// Parse the text of a `.env` file into its bindings, in file order.
///
/// Duplicates are kept as written — the caller decides what a repeated name
/// means, since the precedence rule ("anything already set wins") lives with
/// the environment rather than with the format.
pub fn parse(text: &str, file: &Path) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        // `lines` already drops a `\n`; a CRLF file leaves the `\r` behind,
        // and it is not part of anybody's value.
        let line = raw.strip_suffix('\r').unwrap_or(raw).trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = strip_export(line);
        let entry = entry(line).map_err(|why| malformed(file, n + 1, raw, &why))?;
        out.push(entry);
    }
    Ok(out)
}

/// One `KEY=VALUE` line, already trimmed of `export` and leading space.
fn entry(line: &str) -> std::result::Result<(String, String), String> {
    let Some((key, rest)) = line.split_once('=') else {
        return Err("a `.env` line is `KEY=value`, and this one has no `=`".into());
    };
    let key = key.trim_end();
    if !lex::is_ident(key) {
        return Err(format!(
            "`{key}` is not a name: a letter or `_` followed by letters, digits and `_`"
        ));
    }
    Ok((key.to_string(), value(rest.trim_start())?))
}

/// The value half: bare, `'single'` or `"double"`.
fn value(rest: &str) -> std::result::Result<String, String> {
    match rest.as_bytes().first() {
        // Literal, as in sh: not even a backslash means anything inside it.
        Some(b'\'') => {
            let (text, tail) = quoted(&rest[1..], '\'')?;
            trailing(tail)?;
            Ok(text)
        }
        Some(b'"') => {
            let (text, tail) = quoted(&rest[1..], '"')?;
            trailing(tail)?;
            Ok(unescape(&text))
        }
        // Bare. A `#` that follows whitespace starts a comment, which is how
        // `PORT=8080 # the dev server` is written everywhere; a `#` touching
        // the value is part of it, so `COLOR=#ff0000` survives.
        _ => Ok(strip_comment(rest).trim_end().to_string()),
    }
}

/// The text up to the closing `quote`, and whatever followed it.
fn quoted(rest: &str, quote: char) -> std::result::Result<(String, &str), String> {
    // A `\"` inside a double-quoted value does not close it; nothing escapes
    // inside a single-quoted one.
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quote == '"' => escaped = true,
            c if c == quote => return Ok((rest[..i].to_string(), &rest[i + 1..])),
            _ => {}
        }
    }
    Err(format!(
        "unterminated {} value: it opens with {quote} and never closes",
        if quote == '"' {
            "double-quoted"
        } else {
            "single-quoted"
        }
    ))
}

/// What may follow a closing quote: nothing, or a comment.
fn trailing(tail: &str) -> std::result::Result<(), String> {
    let tail = tail.trim();
    if tail.is_empty() || tail.starts_with('#') {
        return Ok(());
    }
    Err(format!(
        "unexpected `{tail}` after the closing quote; a quoted value ends the line, \
         apart from a `#` comment"
    ))
}

/// `\n`, `\t`, `\\` and `\"` inside a double-quoted value. Any other
/// backslash is an ordinary character, as it is inside a chorefile's own
/// `"..."`, so a Windows path in a `.env` is not silently mangled.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Cut a bare value at a `#` that follows whitespace.
fn strip_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return &value[..i];
        }
    }
    value
}

/// `export FOO=1` is how a `.env` that is also `source`d is written, and the
/// keyword says nothing chore does not already do.
fn strip_export(line: &str) -> &str {
    match line.strip_prefix("export") {
        Some(rest) if rest.starts_with([' ', '\t']) => rest.trim_start(),
        _ => line,
    }
}

fn malformed(file: &Path, line: usize, text: &str, why: &str) -> Error {
    Error::Run {
        message: format!(
            "{}:{line}: {why}\n  {}",
            vars::display(file),
            text.trim_end()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Vec<(String, String)> {
        parse(text, Path::new(".env")).expect("well-formed")
    }

    fn one(text: &str) -> String {
        let entries = read(text);
        assert_eq!(entries.len(), 1, "expected one binding in {text:?}");
        entries[0].1.clone()
    }

    fn error(text: &str) -> String {
        parse(text, Path::new("cfg/.env"))
            .expect_err("malformed")
            .to_string()
    }

    #[test]
    fn plain_lines_bind_in_order() {
        assert_eq!(
            read("A=1\nB=2\nA=3"),
            [
                ("A".into(), "1".into()),
                ("B".into(), "2".into()),
                ("A".into(), "3".into()),
            ]
        );
    }

    #[test]
    fn blank_lines_and_comments_are_skipped() {
        assert_eq!(
            read("\n\n# a note\n   # indented\nA=1\n"),
            [("A".into(), "1".into())]
        );
    }

    #[test]
    fn export_is_accepted_and_dropped() {
        assert_eq!(read("export A=1"), [("A".into(), "1".into())]);
        // Only as a keyword: a name that merely starts with it is a name.
        assert_eq!(read("exportA=1"), [("exportA".into(), "1".into())]);
    }

    #[test]
    fn a_bare_value_is_trimmed_and_loses_a_trailing_comment() {
        assert_eq!(one("A=  hello  "), "hello");
        assert_eq!(one("PORT=8080 # the dev server"), "8080");
        // A `#` touching the value belongs to it, or no colour would survive.
        assert_eq!(one("COLOR=#ff0000"), "#ff0000");
        // No expansion, ever: a `$` is a `$`.
        assert_eq!(one("A=${B}/x"), "${B}/x");
    }

    #[test]
    fn an_empty_value_is_the_empty_string() {
        assert_eq!(one("A="), "");
        assert_eq!(one("A=   "), "");
        assert_eq!(one("A=\"\""), "");
    }

    #[test]
    fn a_single_quoted_value_is_literal() {
        assert_eq!(one(r"A='a b # c'"), "a b # c");
        assert_eq!(one(r"A='\n'"), r"\n");
    }

    #[test]
    fn a_double_quoted_value_takes_the_four_escapes() {
        assert_eq!(one(r#"A="a\nb\tc""#), "a\nb\tc");
        assert_eq!(one(r#"A="say \"hi\"""#), r#"say "hi""#);
        assert_eq!(one(r#"A="C:\\tools""#), r"C:\tools");
        // Any other backslash is an ordinary character, kept with the letter
        // it precedes rather than swallowed as an escape nobody defined.
        assert_eq!(one(r#"A="C:\version""#), r"C:\version");
    }

    #[test]
    fn a_quoted_value_keeps_its_spaces_and_may_be_followed_by_a_comment() {
        assert_eq!(one("A=' padded '"), " padded ");
        assert_eq!(one(r#"A="v"   # note"#), "v");
    }

    #[test]
    fn crlf_is_tolerated() {
        assert_eq!(
            read("A=1\r\nB=2\r\n"),
            [("A".into(), "1".into()), ("B".into(), "2".into())]
        );
        assert_eq!(one("A=\"v\"\r\n"), "v");
    }

    #[test]
    fn a_line_without_an_equals_names_the_file_and_the_line() {
        let message = error("A=1\nNOT A BINDING\n");
        assert!(message.contains("cfg/.env:2:"), "{message}");
        assert!(message.contains("no `=`"), "{message}");
        assert!(message.contains("NOT A BINDING"), "{message}");
    }

    #[test]
    fn a_key_that_is_not_a_name_is_an_error() {
        let message = error("A B=1");
        assert!(message.contains("cfg/.env:1:"), "{message}");
        assert!(message.contains("`A B` is not a name"), "{message}");

        assert!(error("1A=1").contains("`1A` is not a name"));
        assert!(error("=1").contains("`` is not a name"));
    }

    #[test]
    fn an_unterminated_quote_is_an_error() {
        assert!(error("A=\"open").contains("unterminated double-quoted"));
        assert!(error("A='open").contains("unterminated single-quoted"));
    }

    #[test]
    fn junk_after_a_closing_quote_is_an_error() {
        let message = error("A=\"v\" and more");
        assert!(message.contains("after the closing quote"), "{message}");
    }

    #[test]
    fn spaces_around_the_equals_are_allowed() {
        assert_eq!(read("A = 1"), [("A".into(), "1".into())]);
    }
}
