//! `chore check` — diagnostics in the form editors already understand.
//!
//! One line per finding, `path:line:col: message`, plus an indented `help:`
//! line when the library has a concrete suggestion. The analysis itself lives
//! in `chorefile::check`; this only renders it.

use std::io::{self, Write};
use std::path::Path;

use chorefile::check::{self, Severity};

/// Print every finding. Returns how many were errors, so the caller can pick
/// the exit code.
///
/// Warnings are printed but do not fail the run. `check` warns about a command
/// it could not find on *this* machine's `PATH`, which is routinely a fact
/// about the machine rather than about the chorefile — a tool installed only
/// in CI, or only on one platform. Failing on that would make `check`
/// unusable as the CI gate it exists to be.
pub fn report(out: &mut dyn Write, path: &Path, source: &str) -> io::Result<usize> {
    // `check_source` parses for us, so a file too broken to parse still
    // reports its syntax error as a diagnostic rather than as a crash.
    let findings = check::check_source(source, path);
    for finding in &findings {
        writeln!(
            out,
            "{}: {}",
            finding.at.render(source),
            finding.message.trim_end()
        )?;
        if let Some(help) = &finding.help {
            writeln!(out, "  help: {}", help.trim_end())?;
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
