//! ANSI colour, hand-rolled.
//!
//! A colour crate would be the obvious way to do this, and `chore` has no
//! dependencies beyond its own library on purpose: the binary is meant to be
//! one small static file, and "make the task list cyan" is not worth a
//! transitive tree. What is actually needed here is six escape sequences and
//! one decision about whether to emit them.
//!
//! Only the 8/16-colour codes are used. 256-colour and truecolour pick an
//! exact shade, which is exactly the problem: a shade that reads well on a
//! dark theme is often unreadable on a light one. The basic codes name a
//! *role* and let the terminal's own palette decide the pixels, so the output
//! respects whatever theme the user configured.
//!
//! The decision is per stream, not per process. `chore list | less` should
//! stay plain while the `chore: ...` line that follows it on a terminal
//! stderr stays red, and the two streams are redirected independently.

use std::io::IsTerminal;

/// Whether a given stream gets escape sequences, and the helpers that write
/// them.
///
/// A [`Style`] is copied down the call chain rather than read from a global,
/// which is what keeps `list --json`, `list --names` and `spec` honest: their
/// renderers take no [`Style`] at all, so no future edit can slip an escape
/// sequence into output a program is going to parse. A global flag would have
/// been reachable from inside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    on: bool,
}

/// Reset. Every helper closes what it opens, so a truncated line, or output
/// interleaved with a task's own, cannot leave the terminal stuck in a colour.
const RESET: &str = "\x1b[0m";

const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

impl Style {
    /// Colour for stdout, if stdout will be read by a person.
    pub fn stdout() -> Self {
        Self::for_stream(std::io::stdout().is_terminal())
    }

    /// Colour for stderr. Asked separately from [`Style::stdout`] because the
    /// shell redirects the two separately: `chore build > log` still has a
    /// terminal on stderr, and the error line is the one thing the user is
    /// still watching for.
    pub fn stderr() -> Self {
        Self::for_stream(std::io::stderr().is_terminal())
    }

    fn for_stream(tty: bool) -> Self {
        Self {
            on: decide(tty, |name| std::env::var(name).ok()),
        }
    }

    /// A task name in the listing.
    pub fn task(self, s: &str) -> String {
        self.wrap(CYAN, s)
    }

    /// Secondary text: a description, a `help:` line, a file position. Dim
    /// rather than a colour of its own, so it recedes without competing with
    /// the two colours that mean something.
    pub fn dim(self, s: &str) -> String {
        self.wrap(DIM, s)
    }

    pub fn bold(self, s: &str) -> String {
        self.wrap(BOLD, s)
    }

    /// A word the reader is meant to type back: a subcommand, a flag.
    pub fn accent(self, s: &str) -> String {
        self.wrap(CYAN, s)
    }

    pub fn error(self, s: &str) -> String {
        self.wrap(RED, s)
    }

    pub fn warn(self, s: &str) -> String {
        self.wrap(YELLOW, s)
    }

    fn wrap(self, code: &str, s: &str) -> String {
        if self.on {
            format!("{code}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
}

/// Whether to colour, given whether the stream is a terminal and a way to read
/// the environment.
///
/// The environment is passed in rather than read here so the rules can be
/// tested: `std::env::set_var` is unsafe in edition 2024 and would race the
/// other tests in the same process anyway.
///
/// The order below is precedence, and it is the order the conventions were
/// agreed in. `NO_COLOR` (no-color.org) and `CLICOLOR=0` are the user saying
/// no, and a user saying no outranks a terminal that could have displayed it.
/// `CLICOLOR_FORCE` is the user saying yes to a pipe, which is what a build
/// log viewer or a CI web UI needs, so it outranks the TTY check but not a
/// refusal. `TERM=dumb` is not a preference at all: it is the terminal
/// stating it cannot render the sequences, so honouring a force there would
/// only print garbage.
fn decide(tty: bool, var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());

    if var("TERM").as_deref() == Some("dumb") {
        return false;
    }
    // Per no-color.org the *presence* of a non-empty value is the signal; the
    // value itself is never inspected, so `NO_COLOR=0` still means no.
    if set("NO_COLOR") || var("CLICOLOR").as_deref() == Some("0") {
        return false;
    }
    if set("CLICOLOR_FORCE") {
        return true;
    }
    tty
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment built from pairs, for [`decide`].
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn a_terminal_gets_colour_and_a_pipe_does_not() {
        assert!(decide(true, env(&[])));
        assert!(!decide(false, env(&[])));
    }

    #[test]
    fn no_color_wins_over_a_terminal() {
        assert!(!decide(true, env(&[("NO_COLOR", "1")])));
        // Any non-empty value counts, including one that looks like "off".
        assert!(!decide(true, env(&[("NO_COLOR", "0")])));
        // Empty is not set, so it says nothing either way.
        assert!(decide(true, env(&[("NO_COLOR", "")])));
    }

    #[test]
    fn clicolor_zero_disables_and_other_values_do_not() {
        assert!(!decide(true, env(&[("CLICOLOR", "0")])));
        assert!(decide(true, env(&[("CLICOLOR", "1")])));
    }

    #[test]
    fn clicolor_force_colours_a_pipe_but_never_beats_a_refusal() {
        assert!(decide(false, env(&[("CLICOLOR_FORCE", "1")])));
        assert!(!decide(
            false,
            env(&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")])
        ));
    }

    #[test]
    fn a_dumb_terminal_is_never_coloured() {
        assert!(!decide(true, env(&[("TERM", "dumb")])));
        assert!(!decide(
            false,
            env(&[("TERM", "dumb"), ("CLICOLOR_FORCE", "1")])
        ));
        assert!(decide(true, env(&[("TERM", "xterm-256color")])));
    }

    #[test]
    fn a_disabled_style_never_emits_an_escape() {
        let off = Style { on: false };
        assert_eq!(off.task("build"), "build");
        assert_eq!(off.error("boom"), "boom");
        assert_eq!(off.dim("why"), "why");
    }

    #[test]
    fn colour_wraps_and_always_resets() {
        let on = Style { on: true };
        assert_eq!(on.warn("hm"), "\x1b[33mhm\x1b[0m");
        assert!(on.bold("x").ends_with(RESET));
    }
}
