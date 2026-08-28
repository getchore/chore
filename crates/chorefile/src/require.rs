//! `require`: the oldest `chore` a chorefile is willing to be run by.
//!
//! A chorefile written against a newer language than the binary reading it
//! fails somewhere far from the cause: `task deploy target=$TRIPLE` in a build
//! that has no parameter defaults reports "task parameter `target=$TRIPLE`
//! must be a name", which sends the author to inspect syntax that is in fact
//! correct. `require 1.4.0` moves that diagnosis to the top of the file, where
//! it can be stated instead of guessed at.
//!
//! The requirement is a floor, not a range: it says "at least this", so there
//! is nothing to express with an operator and nothing to negotiate. Keeping
//! the grammar to one bare `major.minor.patch` means a chorefile cannot ask a
//! question this module would have to answer with a resolver.
//!
//! # Where the check happens
//!
//! Not in [`resolve`](crate::resolve): an unmet requirement must be an error
//! for a run, a *finding* for `check`, and a warning for `list`, and a tree
//! that refuses to resolve leaves the last two with nothing to report on. So
//! resolving records the requirement and each caller decides what an unmet one
//! costs it.
//!
//! # Which failure is reported
//!
//! Every file in an include tree may carry its own `require`, and each is
//! checked. [`unmet`] reports the strictest of the failures, because that is
//! the version that makes all of them go away: telling someone to upgrade to
//! 1.2.0 when a second file needs 1.6.0 buys them a second trip.

use std::fmt;
use std::path::Path;

use crate::ast;
use crate::error::Location;
use crate::resolve::Merged;
use crate::spec;

/// The one-liner that installs the current release, which is the whole
/// remedy for every failure this module reports.
const INSTALL: &str = "curl -fsSL https://getchore.github.io/chore/install.sh | sh";
const INSTALL_PS: &str = "irm https://getchore.github.io/chore/install.ps1 | iex";

/// A `major.minor.patch` version.
///
/// The field order is the comparison order, which is why [`Ord`] is derived
/// rather than written: comparing the components as numbers, most significant
/// first, is exactly what the derive does. The alternative a `require` must
/// never fall back on is comparing the text, under which 1.10.0 precedes
/// 1.9.0 and a chorefile written for the newer one runs on the older.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// Parse `major.minor.patch`, and nothing else.
    ///
    /// Strict on purpose. `v1.4.0`, `1.4`, `^1.4.0` and `1.4.0-rc1` are all
    /// spellings someone could reasonably expect to work, and each one that
    /// were accepted would be a second thing the comparison has to mean. The
    /// parse error names the shape instead, once, at the line that wrote it.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut number = || {
            let part = parts.next()?;
            // `u64::from_str` accepts a leading `+` and unicode is not what
            // `1.4.0` is made of, so the digits are checked before parsing.
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse::<u64>().ok()
        };
        let version = Self {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The version of the `chore` doing the reading.
///
/// `None` only if that version is not a plain triple, which the crate
/// manifest makes impossible today. A build that ever does carry a suffix is
/// deliberately *not* locked out by every `require` in existence: an
/// unreadable own version is a fact about this binary, and refusing to run on
/// it would be a worse failure than the stale-binary confusion the whole
/// feature exists to remove.
pub fn running() -> Option<Version> {
    Version::parse(spec::version())
}

/// A requirement this binary does not meet, and where it was written.
pub struct Unmet {
    pub required: Version,
    pub running: Version,
    /// The `require` line itself, in the file that wrote it, which is not
    /// always the top-level chorefile and is the one the reader has to open.
    pub at: Location,
}

impl Unmet {
    /// The message, in the shape every other diagnostic uses: what is wrong,
    /// then the two facts that explain it. The location a caller renders in
    /// front of this names the file, so the message does not repeat it.
    pub fn message(&self) -> String {
        format!(
            "this chorefile requires chore {} or newer, and this is chore {}",
            self.required, self.running
        )
    }

    /// What to do about it. There is exactly one answer, so it is spelled out
    /// rather than gestured at.
    pub fn help(&self) -> String {
        format!("update chore: `{INSTALL}` (PowerShell: `{INSTALL_PS}`)")
    }
}

/// The requirement one file states, if this binary does not meet it.
pub fn unmet_in(file: &ast::File, path: &Path) -> Option<Unmet> {
    let require = file.require.as_ref()?;
    let running = running()?;
    (require.version > running).then(|| Unmet {
        required: require.version,
        running,
        at: Location::new(path, require.span),
    })
}

/// The strictest requirement in a merged tree that this binary does not meet.
///
/// Ties go to the file loaded first, which is the include chain's depth-first
/// order: with two files asking for the same version there is nothing to
/// choose between the messages, and the earlier one is nearer the chorefile
/// the user named.
pub fn unmet(merged: &Merged) -> Option<Unmet> {
    merged
        .parts
        .iter()
        .filter_map(|part| unmet_in(&part.file, &part.path))
        .max_by_key(|found| found.required)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_a_bare_triple() {
        assert_eq!(
            Version::parse("1.4.0"),
            Some(Version {
                major: 1,
                minor: 4,
                patch: 0
            })
        );
        for bad in [
            "v1.4.0", "1.4", "1", "^1.4.0", ">=1.4.0", "1.4.0-rc1", "1.4.0.0", "1..0", "1.4.",
            "", "1.4.x", "1.+4.0", " 1.4.0",
        ] {
            assert_eq!(Version::parse(bad), None, "`{bad}` is not a version");
        }
    }

    #[test]
    fn components_compare_as_numbers() {
        // The comparison a string would get wrong, which is the whole reason
        // the version is parsed rather than compared as written.
        assert!(Version::parse("1.10.0") > Version::parse("1.9.0"));
        assert!(Version::parse("10.0.0") > Version::parse("9.99.99"));
        assert!(Version::parse("1.4.10") > Version::parse("1.4.2"));
        assert!(Version::parse("1.4.10") > Version::parse("1.4.9"));
        assert_eq!(Version::parse("1.4.0"), Version::parse("1.4.0"));
    }

    #[test]
    fn the_running_version_is_readable() {
        assert_eq!(running().map(|v| v.to_string()).as_deref(), Some(spec::version()));
    }
}
