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
    files(out, style)?;

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
        // The name is the header someone scans for, so it gets the same
        // colour a builtin name and a task name get. The rule under it is a
        // paragraph and stays plain.
        writeln!(out, "  {}", style.accent(rule.name))?;
        writeln!(out, "{}\n", wrap(rule.rule, "    "))?;
    }

    writeln!(
        out,
        "`chore help <builtin>` explains one builtin, `chore help include` one statement form,\n\
         `chore help files` the file names. `chore spec` prints the whole reference as JSON."
    )?;
    // The question anyone adopting a task runner asks next is how to get it
    // onto CI, and `chore help` is where an agent looks before a README.
    writeln!(
        out,
        "\nIn GitHub Actions, {} installs it:\n  {}",
        style.accent(chorefile::spec::ACTION),
        style.dim(chorefile::spec::ACTION_URL)
    )?;
    Ok(())
}

/// The files section: the first question anyone has, before any syntax.
fn files(out: &mut dyn Write, style: Style) -> Result<(), Exit> {
    section(out, "files", style)?;
    let rows: Vec<_> = spec::files().iter().map(|f| (f.name, f.meaning)).collect();
    columns(out, &rows, style)?;
    Ok(())
}

/// `chore help <topic>`: a builtin, a statement form such as `include`, or
/// `files`. Every name someone might type at the top of a chorefile answers
/// here, so "no builtin `include`" is not a reply anyone gets any more.
pub fn topic(out: &mut dyn Write, name: &str, style: Style) -> Result<(), Exit> {
    if let Some(builtin) = spec::builtin(name) {
        return self::builtin(out, builtin, style);
    }
    if let Some(form) = spec::form(name) {
        writeln!(out, "{}\n", style.bold(form.syntax))?;
        writeln!(out, "{}\n", wrap(form.meaning, "  "))?;
        for line in form.example.lines() {
            writeln!(out, "  {}", style.accent(line))?;
        }
        return Ok(());
    }
    if name == "files" {
        writeln!(out, "{}", style.bold("files"))?;
        for kind in spec::files() {
            writeln!(out, "\n  {}", style.accent(kind.name))?;
            if kind.examples != kind.name {
                writeln!(out, "    {}", style.dim(kind.examples))?;
            }
            writeln!(out, "{}", wrap(kind.meaning, "    "))?;
        }
        return Ok(());
    }
    Err(Exit::usage(unknown(name)))
}

fn builtin(out: &mut dyn Write, builtin: &spec::Builtin, style: Style) -> Result<(), Exit> {
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

/// A near miss is usually a typo, so point at the closest name rather than
/// making the user run `chore help` and read the whole list again.
fn unknown(name: &str) -> String {
    let near = |n: &str| n.starts_with(name) || name.starts_with(n);
    let builtin = spec::builtins().iter().map(|b| b.name).find(|n| near(n));
    let form = spec::syntax().iter().map(|f| f.name).find(|n| near(n));
    match builtin.or(form) {
        Some(near) => format!("no help topic `{name}` (did you mean `{near}`?)"),
        None => format!(
            "no help topic `{name}`: a builtin, a statement such as `include`, or `files` \
             (try `chore help`)"
        ),
    }
}
