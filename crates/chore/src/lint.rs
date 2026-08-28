//! `chore check` — diagnostics in the form editors already understand.
//!
//! One line per finding, `path:line:col` plus the message, and an indented
//! `help:` line when the library has a concrete suggestion. The analysis
//! itself lives in `chorefile::check`; this only renders it.
//!
//! A finding can point into any file that contributed to the run, so the
//! line and column come from [`Sources`], which holds the text of each one:
//! resolving `line:col` against the top-level file's text would put an
//! included file's finding at whatever line that offset happens to land on in
//! a different file. With one source file the answer is identical, so this is
//! the right rendering with or without includes.

use std::io::{self, Write};

use chorefile::check::{Diagnostic, Severity};
use chorefile::resolve::Sources;

use crate::style::Style;

/// Print every finding. Returns how many were errors, so the caller can pick
/// the exit code.
///
/// Warnings are printed but do not fail the run. `check` warns about a command
/// it could not find on *this* machine's `PATH`, which is routinely a fact
/// about the machine rather than about the chorefile — a tool installed only
/// in CI, or only on one platform. Failing on that would make `check`
/// unusable as the CI gate it exists to be.
///
/// The severity is carried by colour and by nothing else: the line format is
/// `path:line:col: message`, which is what an editor's error matcher parses,
/// so a `warning:` word cannot be added in front without breaking every
/// matcher already pointed at it. Colour is invisible to those tools and to a
/// pipe, and tells a person at a terminal the one thing the line does not say
/// on its own.
pub fn report(
    out: &mut dyn Write,
    findings: &[Diagnostic],
    sources: &Sources,
    style: Style,
) -> io::Result<usize> {
    for finding in findings {
        let message = finding.message.trim_end();
        let message = match finding.severity {
            Severity::Error => style.error(message),
            Severity::Warning => style.warn(message),
        };
        // The position recedes: it is there to be clicked or copied, not read.
        writeln!(
            out,
            "{}: {message}",
            style.dim(&sources.render(&finding.at))
        )?;
        if let Some(help) = &finding.help {
            writeln!(
                out,
                "{}",
                style.dim(&format!("  help: {}", help.trim_end()))
            )?;
        }
    }
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings.len() - errors;
    match (errors, warnings) {
        (0, 0) => writeln!(out, "ok")?,
        (0, n) => writeln!(out, "\n{}", count(n, "warning"))?,
        (n, 0) => writeln!(out, "\n{}", count(n, "problem"))?,
        (e, w) => writeln!(out, "\n{}, {}", count(e, "problem"), count(w, "warning"))?,
    }
    Ok(errors)
}

/// `1 problem` / `2 problems`, so the summary line reads as English.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
