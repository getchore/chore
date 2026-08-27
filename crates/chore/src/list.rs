//! `chore list` — the tasks in the chorefile, in source order.
//!
//! Listing never runs globals, so it works even when a global reads a file
//! that is not there yet.

use std::io::{self, Write};

use chorefile::ast::Task;

/// Space between the name column and the description column.
const GAP: usize = 4;

pub fn text(out: &mut dyn Write, tasks: &[Task]) -> io::Result<()> {
    if tasks.is_empty() {
        return writeln!(out, "no tasks");
    }
    let width = tasks.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for task in tasks {
        match &task.doc {
            Some(doc) => writeln!(out, "  {:width$}{}{doc}", task.name, " ".repeat(GAP)),
            None => writeln!(out, "  {}", task.name),
        }?;
    }
    Ok(())
}

pub fn json(out: &mut dyn Write, tasks: &[Task]) -> io::Result<()> {
    writeln!(out, "[")?;
    for (i, task) in tasks.iter().enumerate() {
        let comma = if i + 1 == tasks.len() { "" } else { "," };
        let params: Vec<String> = task.params.iter().map(|p| quote(p)).collect();
        writeln!(
            out,
            "  {{\"name\": {}, \"description\": {}, \"params\": [{}]}}{comma}",
            quote(&task.name),
            task.doc.as_deref().map_or("null".into(), quote),
            params.join(", "),
        )?;
    }
    writeln!(out, "]")
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
}
