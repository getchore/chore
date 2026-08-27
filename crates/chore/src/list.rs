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

/// Space between the name column and the description column.
const GAP: usize = 4;

/// Widest the name column may grow. One `deploy::staging::rollback` should
/// not push every description off to the right; a name past the cap simply
/// takes its own line width and gets `GAP` spaces, which reads better than a
/// forty-column gutter down the whole list.
const MAX_NAME: usize = 28;

pub fn text(out: &mut dyn Write, merged: &Merged) -> io::Result<()> {
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
                writeln!(out, "  {}{}{doc}", task.name, " ".repeat(pad))
            }
            None => writeln!(out, "  {}", task.name),
        }?;
    }
    Ok(())
}

pub fn json(out: &mut dyn Write, merged: &Merged) -> io::Result<()> {
    let tasks = &merged.file.tasks;
    writeln!(out, "[")?;
    for (i, task) in tasks.iter().enumerate() {
        let comma = if i + 1 == tasks.len() { "" } else { "," };
        let params: Vec<String> = task.params.iter().map(|p| quote(p)).collect();
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
