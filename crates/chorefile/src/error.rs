//! Errors, and the source spans used to point at the offending text.

use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

/// A byte range within one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn range(self) -> Range<usize> {
        self.start..self.end
    }
}

/// Where a diagnostic points: a file, and a span inside it.
#[derive(Debug, Clone)]
pub struct Location {
    pub file: PathBuf,
    pub span: Span,
}

impl Location {
    pub fn new(file: impl Into<PathBuf>, span: Span) -> Self {
        Self {
            file: file.into(),
            span,
        }
    }

    /// A span with no file yet. The lexer works on a string and does not know
    /// which file it came from, so it reports with this and the parser fills
    /// the path in.
    pub fn unknown(span: Span) -> Self {
        Self {
            file: PathBuf::new(),
            span,
        }
    }

    /// The 1-based line and column this location starts at.
    ///
    /// Spans are byte offsets, which is what the lexer and parser want and
    /// what no human wants to read, so every caller that prints a diagnostic
    /// needs this. The column counts characters, not bytes, so a line with a
    /// non-ASCII character before the error still points at the right place.
    pub fn line_col(&self, source: &str) -> (usize, usize) {
        let upto = &source[..self.span.start.min(source.len())];
        let line = upto.bytes().filter(|&b| b == b'\n').count() + 1;
        let col = upto
            .rsplit_once('\n')
            .map_or(upto, |(_, last)| last)
            .chars()
            .count()
            + 1;
        (line, col)
    }

    /// `path:line:col`, the form editors and terminals make clickable.
    ///
    /// The path is spelled with `/` on every platform, like every other path
    /// `chore` prints — one rule, so a diagnostic looks the same wherever it
    /// was produced.
    pub fn render(&self, source: &str) -> String {
        let (line, col) = self.line_col(source);
        format!("{}:{line}:{col}", crate::vars::display(&self.file))
    }
}

#[derive(Debug)]
pub enum Error {
    /// The source did not lex or parse.
    Syntax {
        message: String,
        at: Location,
    },
    /// A task ran and failed, or a command exited nonzero outside `try`.
    Run {
        message: String,
    },
    /// No chorefile in the working directory or any parent.
    NotFound {
        from: PathBuf,
        /// A file in `from` that was probably meant instead, if any: see
        /// [`crate::near_miss`].
        near: Option<String>,
    },
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { message, at } => {
                write!(f, "{}: {message}", crate::vars::display(&at.file))
            }
            Self::Run { message } => write!(f, "{message}"),
            Self::NotFound { from, near } => {
                write!(
                    f,
                    "no {} found in {} or any parent directory",
                    crate::FILE_NAME,
                    crate::vars::display(from)
                )?;
                write!(f, "\n  help: {}", not_found_help(near.as_deref()))
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// What to do about a directory with no chorefile, given what is there.
///
/// Three cases, each the one thing an agent or a person gets wrong: the name
/// spelled with a capital, which opens fine on a case-insensitive disk and
/// not on Linux; a `.chore` fragment mistaken for the file `chore` reads on
/// its own; and an empty directory, where `init` is the answer.
fn not_found_help(near: Option<&str>) -> String {
    match near {
        Some(name) if name.eq_ignore_ascii_case(crate::FILE_NAME) => format!(
            "`{name}` is here, but only the exact lowercase name `{}` is read, on every \
             platform; rename it",
            crate::FILE_NAME
        ),
        Some(name) => format!(
            "`{name}` is here; a `.{}` file is a fragment, reached through `include {name}` in \
             a `{}`, or directly with `chore --file {name}`",
            crate::FILE_EXT,
            crate::FILE_NAME
        ),
        None => format!(
            "`chore init` writes a starter `{}` here; the file is found by walking up from the \
             working directory, and only that exact name is looked for",
            crate::FILE_NAME
        ),
    }
}
