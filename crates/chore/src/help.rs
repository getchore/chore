//! `chore help` — the language reference, rendered for a terminal.
//!
//! Every word of reference content comes from `chorefile::spec`; nothing about
//! the language is written down here, so the JSON `chore spec` prints and the
//! text `chore help` prints can never drift apart.

use std::io::Write;

use chorefile::spec;

use crate::Exit;
use crate::style::Style;

/// Space between the two columns of every listing.
const GAP: usize = 2;

/// Wrap width. The reference stores prose as one long line so the JSON stays
/// clean, which leaves the wrapping to whoever prints it.
const WIDTH: usize = 88;

pub fn overview(out: &mut dyn Write, style: Style) -> Result<(), Exit> {
    writeln!(
        out,
        "{}",
        style.bold(&format!("chorefile {}", spec::version()))
    )?;
    section(out, "syntax", style)?;
    let rows: Vec<_> = spec::syntax()
        .iter()
        .map(|form| (form.syntax, form.meaning))
        .collect();
    columns(out, &rows, style)?;

    section(out, "conditions", style)?;
    let rows: Vec<_> = spec::conditions()
        .iter()
        .map(|c| (c.syntax, c.meaning))
        .collect();
    columns(out, &rows, style)?;

    section(out, "chaining", style)?;
    let rows: Vec<_> = spec::chaining()
        .iter()
        .map(|op| (op.symbol, op.meaning))
        .collect();
    columns(out, &rows, style)?;

    section(out, "variables", style)?;
    let rows: Vec<_> = spec::variables()
        .iter()
        .map(|v| (v.name, v.meaning))
        .collect();
    columns(out, &rows, style)?;

    section(out, "builtins", style)?;
    let rows: Vec<_> = spec::builtins()
        .iter()
        .map(|b| (b.name, b.summary))
        .collect();
    columns(out, &rows, style)?;

    section(out, "resolution", style)?;
    for rule in spec::resolution() {
        writeln!(out, "{}", wrap(rule.rule, "  "))?;
    }

    section(out, "rules", style)?;
    for rule in spec::rules() {
        writeln!(out, "  {}", rule.name)?;
        writeln!(out, "{}\n", wrap(rule.rule, "    "))?;
    }

    writeln!(
        out,
        "`chore help <builtin>` explains one builtin. `chore spec` prints the whole\nreference as JSON."
    )?;
    Ok(())
}

pub fn builtin(out: &mut dyn Write, name: &str, style: Style) -> Result<(), Exit> {
    let Some(builtin) = spec::builtin(name) else {
        return Err(Exit::usage(unknown(name)));
    };
    writeln!(out, "{}\n", style.bold(builtin.usage))?;
    writeln!(out, "{}\n", wrap(builtin.summary, "  "))?;
    writeln!(out, "{}", wrap(builtin.description, "  "))?;
    if !builtin.flags.is_empty() {
        section(out, "flags", style)?;
        let rows: Vec<_> = builtin
            .flags
            .iter()
            .map(|flag| {
                let left = if flag.argument.is_empty() {
                    flag.name.to_string()
                } else {
                    format!("{} <{}>", flag.name, flag.argument)
                };
                (
                    left,
                    format!("{} (default: {})", flag.meaning, flag.default),
                )
            })
            .collect();
        let rows: Vec<(&str, &str)> = rows.iter().map(|(l, r)| (l.as_str(), r.as_str())).collect();
        columns(out, &rows, style)?;
    }
    Ok(())
}

/// A heading. Bold and nothing more: these are the only landmarks in a page
/// of otherwise unbroken reference text, and a colour here would compete with
/// the listings underneath rather than separate them.
fn section(out: &mut dyn Write, title: &str, style: Style) -> Result<(), Exit> {
    writeln!(out, "\n{}\n", style.bold(title))?;
    Ok(())
}

/// Two aligned columns. The right column is reflowed under itself, so a long
/// explanation stays inside its column instead of wrapping back to column one
/// and breaking the alignment that makes the listing readable.
fn columns(out: &mut dyn Write, rows: &[(&str, &str)], style: Style) -> Result<(), Exit> {
    let width = rows.iter().map(|(left, _)| left.len()).max().unwrap_or(0);
    let indent = " ".repeat(2 + width + GAP);
    for (left, right) in rows {
        if right.is_empty() {
            writeln!(out, "  {}", style.accent(left))?;
            continue;
        }
        let text = wrap(right, &indent);
        // Pad from the bare text, then colour. Formatting the coloured string
        // to a width counts the escape bytes as characters, which pushes every
        // description a few columns right of the one above it.
        let pad = " ".repeat(width - left.len() + GAP);
        writeln!(out, "  {}{pad}{}", style.accent(left), text.trim_start())?;
    }
    Ok(())
}

/// Reflow a paragraph, indenting every line — the first included, which the
/// column printer then trims off.
fn wrap(text: &str, indent: &str) -> String {
    let mut out = String::new();
    let mut line = String::from(indent);
    let mut width = indent.chars().count();
    for word in text.split_whitespace() {
        let word_width = word.chars().count();
        // Columns, not bytes: an em dash is three bytes wide and one column.
        if width > indent.chars().count() && width + 1 + word_width > WIDTH {
            out.push_str(&line);
            out.push('\n');
            line = String::from(indent);
            width = indent.chars().count();
        }
        if width > indent.chars().count() {
            line.push(' ');
            width += 1;
        }
        line.push_str(word);
        width += word_width;
    }
    out.push_str(&line);
    out
}

/// A near miss is usually a typo, so point at the closest builtin rather than
/// making the user run `chore help` and read the whole list again.
fn unknown(name: &str) -> String {
    match spec::builtins()
        .iter()
        .find(|b| b.name.starts_with(name) || name.starts_with(b.name))
    {
        Some(near) => format!("no builtin `{name}` (did you mean `{}`?)", near.name),
        None => format!("no builtin `{name}` (try `chore help`)"),
    }
}
