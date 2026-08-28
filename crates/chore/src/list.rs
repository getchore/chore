//! `chore list` — the tasks in the merged chorefile, in merge order.
//!
//! Listing never runs globals, so it works even when a global reads a file
//! that is not there yet.
//!
//! Order is the order the files were merged in — each `include` depth-first
//! in the order it is written, then the including file's own tasks — and
//! never a sort. Grouping by namespace was the alternative and it loses
//! twice: a flat `include` has no namespace to group under, so half the list
//! would keep source order anyway, and re-ordering hides the one thing the
//! author did control. Merge order already puts every task from one file
//! together, which is the grouping a namespace would have given, without
//! inventing an order nobody wrote.

use std::io::{self, Write};
use std::path::Path;

use chorefile::NAMESPACE_SEP;
use chorefile::ast::Task;
use chorefile::resolve::Merged;

use crate::style::Style;

/// Space between the name column and the description column.
const GAP: usize = 4;

/// Widest the name column may grow. One `deploy::staging::rollback` should
/// not push every description off to the right; a name past the cap simply
/// takes its own line width and gets `GAP` spaces, which reads better than a
/// forty-column gutter down the whole list.
const MAX_NAME: usize = 28;

/// The colour is applied after the column width is computed, never before: an
/// escape sequence occupies bytes and no columns, so padding measured on a
/// styled name would drift the description column by exactly the length of
/// the escapes.
pub fn text(out: &mut dyn Write, merged: &Merged, style: Style) -> io::Result<()> {
    let tasks = &merged.file.tasks;
    if tasks.is_empty() {
        return writeln!(out, "no tasks");
    }
    // Characters, not bytes: a non-ASCII task name is one column per char.
    let width = tasks
        .iter()
        .map(|t| t.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(MAX_NAME);
    for task in tasks {
        match &task.doc {
            Some(doc) => {
                let pad = width.saturating_sub(task.name.chars().count()) + GAP;
                writeln!(
                    out,
                    "  {}{}{}",
                    style.task(&task.name),
                    " ".repeat(pad),
                    style.dim(doc)
                )
            }
            None => writeln!(out, "  {}", style.task(&task.name)),
        }?;
    }
    Ok(())
}

/// The chorefile this listing came from, and the `$ROOT` its tasks will see.
///
/// Why this line exists at all: `chore` uses the *nearest* chorefile at or
/// above the working directory, so in a tree with more than one, the same
/// command means different things depending on where you stand. From `repo/`
/// the listing is the whole project and `$ROOT` is `repo/`; from
/// `repo/handoff/`, where `handoff` keeps a chorefile of its own, it is a
/// different set of tasks and `$ROOT` is `repo/handoff/` — so a task that says
/// `download vendor/thing` lands in a different directory, with no error
/// either way. A real subproject genuinely wants that, which is why it stays
/// allowed rather than forbidden; this line is what stops it being silent.
///
/// The chorefile is spelled relative to the working directory, because that is
/// the form that answers "am I where I think I am": `../chorefile` says at a
/// glance that the file governing this listing is not the one here. `$ROOT` is
/// absolute, because a relative `$ROOT` would print `.` both from the repo
/// root and from inside a subproject — the two cases this line exists to tell
/// apart. Printed always, never only when it looks surprising: a line that
/// appears only sometimes makes its own absence load-bearing, and an absence
/// is exactly what nobody notices.
///
/// Dim, one line, above the list: it is the frame around the answer, not the
/// answer, and it has to be read before the tasks are, not after.
pub fn source(out: &mut dyn Write, path: &Path, root: &Path, style: Style) -> io::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    writeln!(
        out,
        "{}",
        style.dim(&format!(
            "using {}, $ROOT = {}",
            relative(path, &cwd),
            chorefile::vars::display(root)
        ))
    )
}

/// `path` as written from `cwd`: `chorefile`, `../chorefile`,
/// `handoff/chorefile`.
///
/// Hand-rolled because `std` has no relative-path operation and the rule is
/// four lines: drop the components the two share, one `..` for each component
/// of `cwd` left over, then what is left of `path`. The one fallback is when
/// nothing at all is shared — a different Windows drive — where no number of
/// `..` connects the two and the absolute path is the only true answer.
///
/// Deliberately no "use the shorter one" rule: `../../../chorefile` is longer
/// than the absolute path in a shallow tree and is still the spelling that
/// says you walked up, which is the whole point of the line. One rule also
/// means the output does not change shape as a project moves down a disk.
///
/// Separators are `/` on every platform, the way `vars::display` reports a
/// path everywhere else in the output.
fn relative(path: &Path, cwd: &Path) -> String {
    let absolute = chorefile::vars::display(path);
    let shared = cwd
        .components()
        .zip(path.components())
        .take_while(|(here, there)| here == there)
        .count();
    if shared == 0 {
        return absolute;
    }
    let up = cwd.components().count() - shared;
    let rest: Vec<String> = path
        .components()
        .skip(shared)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut out = "../".repeat(up);
    out.push_str(&rest.join("/"));
    if out.is_empty() { absolute } else { out }
}

/// One task per line, `name<TAB>description`, for a shell completion script.
///
/// The completion scripts run this on every Tab, so the format is whatever is
/// cheapest to consume from a shell: no padding to strip, no JSON to parse and
/// so no dependency on `jq`. A task with no comment above it prints its name
/// and an empty description, keeping the field count the same on every line.
pub fn names(out: &mut dyn Write, merged: &Merged) -> io::Result<()> {
    for task in &merged.file.tasks {
        writeln!(out, "{}\t{}", task.name, task.doc.as_deref().unwrap_or(""))?;
    }
    Ok(())
}

/// The listing as JSON: which chorefile it came from, what `$ROOT` is, and
/// the tasks.
///
/// The two top-level fields are about the **top-level** chorefile and nothing
/// else — `chorefile` is the file that was discovered by walking up from the
/// working directory, and `root` is its directory, which is `$ROOT` for every
/// task in the run no matter which file the task was written in. A task's own
/// file stays where it always was, in that task's `file`; with `include` the
/// two differ, and the top-level pair is the one that answers "which project
/// is this, and where will its paths land".
///
/// This is why the document is an object and no longer a bare array: an array
/// has nowhere to put a fact about the whole listing, and a tool that reads
/// `list --json` needs the same thing the text listing now says.
/// The document is an **array of tasks**, and stays one.
///
/// The text listing gained a line naming the chorefile and `$ROOT`, and the
/// same two facts belong here — but an array has nowhere to put a fact about
/// the whole listing, so adding them means an object, and that breaks every
/// consumer doing `jq '.[]'`. This is a published contract; it changes in a
/// major release with a note, not quietly in a minor one. Until then a tool
/// reads the per-task `file` field, which is unchanged.
pub fn json(out: &mut dyn Write, merged: &Merged) -> io::Result<()> {
    let tasks = &merged.file.tasks;
    writeln!(out, "[")?;
    for (i, task) in tasks.iter().enumerate() {
        let comma = if i + 1 == tasks.len() { "" } else { "," };
        // A parameter is reported as written, so `dest=build` tells a reader
        // it is optional and what it falls back to — the same thing they
        // would see in the chorefile, and enough to build a call from.
        let params: Vec<String> = task
            .params
            .iter()
            .map(|p| match &p.default {
                Some(_) => quote(&format!("{}=", p.name)),
                None => quote(&p.name),
            })
            .collect();
        // Where the task came from, for a tool that wants to open it: the
        // namespace an `include ... as` gave it, and the file it was written
        // in. Both are `null` for a task in the top-level chorefile of a
        // project that uses no includes.
        let namespace = task
            .name
            .rsplit_once(NAMESPACE_SEP)
            .map_or("null".to_string(), |(ns, _)| quote(ns));
        // `vars::display`, not `Path::display`: a chorefile is written with
        // `/` and reported with `/` on every platform, and this field is read
        // by tools that should not have to care which host produced it.
        let file = origin(task, merged)
            .map_or("null".to_string(), |p| quote(&chorefile::vars::display(p)));
        writeln!(
            out,
            "  {{\"name\": {}, \"description\": {}, \"params\": [{}], \
             \"namespace\": {namespace}, \"file\": {file}}}{comma}",
            quote(&task.name),
            task.doc.as_deref().map_or("null".into(), quote),
            params.join(", "),
        )?;
    }
    writeln!(out, "]")
}

/// The file a task was written in, or `None` when it cannot be known.
///
/// A merged [`Task`] carries a span but not the file the span indexes into,
/// so the file is recovered by looking: the one contributing source whose
/// text, at exactly that span, declares a task of this name. With a single
/// source there is nothing to disambiguate and it is that file. Two files
/// that both happen to declare the same task at the same byte offset are
/// reported as `null` rather than guessed at — a wrong path is worse to a
/// tool than no path.
fn origin<'a>(task: &Task, merged: &'a Merged) -> Option<&'a Path> {
    let files: Vec<&Path> = merged.sources.files().collect();
    if let [only] = files[..] {
        return Some(only);
    }
    let bare = task.name.rsplit(NAMESPACE_SEP).next()?;
    let mut found = None;
    for path in files {
        let Some(text) = merged.sources.get(path) else {
            continue;
        };
        let Some(slice) = text.get(task.span.range()) else {
            continue;
        };
        if !declares(slice, bare) {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(path);
    }
    found
}

/// Whether `slice` — a task's span, which starts at the `task` keyword —
/// declares a task called `name`.
fn declares(slice: &str, name: &str) -> bool {
    let Some(rest) = slice.strip_prefix("task") else {
        return false;
    };
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    rest.trim_start()
        .strip_prefix(name)
        .is_some_and(|after| after.starts_with(|c: char| c.is_whitespace() || c == '{'))
}

/// A JSON string literal. Hand-rolled because the alternative is a serde
/// dependency for one function on a binary that must stay small and static.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_json_strings() {
        assert_eq!(quote("a\"b\\c\n"), r#""a\"b\\c\n""#);
    }

    /// Unix spellings, because the components are what the function works on
    /// and a `\`-separated string is not a path on this host. The `/` in the
    /// output is the contract, and it holds on Windows too: every component
    /// is re-joined with `/` rather than with the host separator.
    #[test]
    fn spells_a_chorefile_relative_to_the_working_directory() {
        let rel = |path: &str, cwd: &str| relative(Path::new(path), Path::new(cwd));
        // Here.
        assert_eq!(rel("/repo/chorefile", "/repo"), "chorefile");
        // Found by walking up, from one level down and from three.
        assert_eq!(rel("/repo/chorefile", "/repo/handoff"), "../chorefile");
        assert_eq!(rel("/repo/chorefile", "/repo/a/b/c"), "../../../chorefile");
        // A subproject's own file, seen from the root above it.
        assert_eq!(rel("/repo/handoff/chorefile", "/repo"), "handoff/chorefile");
    }

    #[test]
    fn falls_back_to_the_absolute_path_only_when_nothing_is_shared() {
        let rel = |path: &str, cwd: &str| relative(Path::new(path), Path::new(cwd));
        // Nothing shared: no number of `..` connects the two.
        assert_eq!(rel("/repo/chorefile", "relative/cwd"), "/repo/chorefile");
        // Sharing only the root still climbs, longer than the absolute path
        // and still the spelling that says which way the file was found.
        assert_eq!(rel("/chorefile", "/a/b/c/d"), "../../../../chorefile");
    }

    #[test]
    fn recognises_a_task_declaration_at_a_span() {
        assert!(declares("task build {\n}", "build"));
        assert!(declares("task greet name {\n}", "greet"));
        assert!(declares("task build{}", "build"));
        // A different task, and a name that is only a prefix of this one.
        assert!(!declares("task builder {\n}", "build"));
        assert!(!declares("task build {\n}", "builder"));
        assert!(!declares("taskbuild {\n}", "build"));
        assert!(!declares("echo build", "build"));
    }
}
