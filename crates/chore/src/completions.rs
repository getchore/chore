//! `chore completions` — shell completion for task names.
//!
//! The scripts here do not embed a task list. They shell out to
//! `chore list --names` in whatever directory the user is standing in, and
//! `chore` finds the nearest chorefile from there, so completion follows a
//! person between projects with nothing to configure per repository. That
//! call costs about two milliseconds because `list` only parses: it never
//! evaluates a global, so it does no I/O.
//!
//! Bare `chore completions` is for a person and says what to add and where.
//! `chore completions <shell>` is for a machine and prints the script, which
//! is what a package manager's formula redirects into place.

use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;

/// The shells with a completion script.
// `PowerShell` ends with the enum's name, which clippy reads as a stutter. It
// is the shell's actual name, and `Shell::Power` would be worse for every
// reader to save one lint.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl Shell {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            "powershell" | "pwsh" => Some(Self::PowerShell),
            _ => None,
        }
    }

    /// The user's shell, from `$SHELL`.
    ///
    /// `$SHELL` is the login shell rather than the running one, so it is a
    /// guess. It is the right guess almost always, and `chore completions
    /// <shell>` is there for when it is not.
    pub fn detect() -> Option<Self> {
        let shell = std::env::var("SHELL").ok()?;
        // "/opt/homebrew/bin/zsh" -> "zsh"
        let name = shell.rsplit('/').next()?;
        Self::parse(name)
    }

    #[cfg(test)]
    pub fn all() -> [Self; 4] {
        [Self::Bash, Self::Zsh, Self::Fish, Self::PowerShell]
    }

    pub fn script(self) -> &'static str {
        match self {
            Self::Bash => BASH,
            Self::Zsh => ZSH,
            Self::Fish => FISH,
            Self::PowerShell => POWERSHELL,
        }
    }

    /// The file to add the line to, and the line to add.
    ///
    /// fish reads a directory rather than a startup file, so its "line" is
    /// the redirect that writes the script there.
    fn install(self) -> Option<(PathBuf, String)> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        Some(match self {
            Self::Bash => (
                home.join(".bashrc"),
                "source <(chore completions bash)".into(),
            ),
            Self::Zsh => (
                home.join(".zshrc"),
                "source <(chore completions zsh)".into(),
            ),
            Self::Fish => (
                home.join(".config/fish/completions/chore.fish"),
                "chore completions fish > ~/.config/fish/completions/chore.fish".into(),
            ),
            // $PROFILE is resolved by PowerShell itself and moves between
            // Windows, PowerShell 7 and the ISE, so guessing it from here
            // would be guessing wrong on someone.
            Self::PowerShell => return None,
        })
    }
}

impl fmt::Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        })
    }
}

/// `chore completions <shell>`: the script itself, for a redirect.
pub fn script(out: &mut dyn Write, shell: Shell) -> io::Result<()> {
    write!(out, "{}", shell.script())
}

/// `chore completions`: what to add, and where.
pub fn guide(out: &mut dyn Write, shell: Option<Shell>) -> io::Result<()> {
    let Some(shell) = shell else {
        writeln!(
            out,
            "chore could not tell which shell you use.\n\n\
             Run one of:\n\n    \
             chore completions bash\n    \
             chore completions zsh\n    \
             chore completions fish\n    \
             chore completions powershell"
        )?;
        return Ok(());
    };

    match shell.install() {
        Some((path, line)) if shell == Shell::Fish => {
            writeln!(
                out,
                "{shell}\n\nRun this once:\n\n    {line}\n\nThen restart your shell.\n\n\
                 Or let chore do it: chore completions --write"
            )?;
            let _ = path;
        }
        Some((path, line)) => {
            writeln!(
                out,
                "{shell}\n\nAdd this to {}:\n\n    {line}\n\nThen run: exec {shell}\n\n\
                 Or let chore do it: chore completions --write",
                tilde(&path)
            )?;
        }
        None => {
            writeln!(
                out,
                "{shell}\n\nAdd this to your $PROFILE:\n\n    \
                 chore completions powershell | Out-String | Invoke-Expression\n\n\
                 Then open a new terminal."
            )?;
        }
    }
    Ok(())
}

/// `chore completions --write`: do it, once.
///
/// Idempotent on purpose. Running it twice is a no-op rather than a second
/// copy of the line, because the second run is usually someone who is not
/// sure the first one worked.
pub fn write(out: &mut dyn Write, shell: Option<Shell>) -> io::Result<bool> {
    let Some(shell) = shell else {
        writeln!(
            out,
            "chore could not tell which shell you use. \
             Try: chore completions zsh --write"
        )?;
        return Ok(false);
    };
    let Some((path, line)) = shell.install() else {
        writeln!(
            out,
            "--write cannot find PowerShell's $PROFILE. Run `chore completions` \
             for the line to add."
        )?;
        return Ok(false);
    };

    if shell == Shell::Fish {
        // fish reads a directory, so writing the script *is* the install.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, Shell::Fish.script())?;
        writeln!(out, "wrote {}\n\nThen restart your shell.", tilde(&path))?;
        return Ok(true);
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == line) {
        writeln!(out, "{} already has it. Nothing to do.", tilde(&path))?;
        return Ok(true);
    }

    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("\n# chore completions\n");
    text.push_str(&line);
    text.push('\n');
    std::fs::write(&path, text)?;

    writeln!(
        out,
        "added to {}:\n\n    {line}\n\nRun: exec {shell}",
        tilde(&path)
    )?;
    Ok(true)
}

/// `/Users/ada/.zshrc` reads better as `~/.zshrc`.
fn tilde(path: &std::path::Path) -> String {
    let display = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && display.starts_with(&home) => {
            format!("~{}", &display[home.len()..])
        }
        _ => display,
    }
}

const BASH: &str = r#"# chore completions for bash.
_chore() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    if [ "$COMP_CWORD" -ne 1 ]; then
        return
    fi
    local names
    names="$(chore list --names 2>/dev/null | cut -f1)"
    COMPREPLY=($(compgen -W "$names list help check spec completions" -- "$cur"))
}
complete -F _chore chore
"#;

// zsh gets the description column, which is the whole reason a task carries
// the comment above it.
const ZSH: &str = r#"#compdef chore
_chore() {
    local -a tasks
    local name desc
    while IFS=$'\t' read -r name desc; do
        tasks+=("${name}:${desc}")
    done < <(chore list --names 2>/dev/null)
    tasks+=('list:tasks and descriptions' 'help:syntax and builtins'
            'check:lint without running' 'spec:full reference as JSON'
            'completions:shell completion')
    _describe -t chore-tasks 'task' tasks
}
compdef _chore chore
"#;

// fish reads `name<TAB>description` from a command directly, so the script is
// the one line.
const FISH: &str = r#"# chore completions for fish.
complete -c chore -f -a '(chore list --names 2>/dev/null)'
complete -c chore -f -n '__fish_use_subcommand' -a 'list' -d 'tasks and descriptions'
complete -c chore -f -n '__fish_use_subcommand' -a 'help' -d 'syntax and builtins'
complete -c chore -f -n '__fish_use_subcommand' -a 'check' -d 'lint without running'
complete -c chore -f -n '__fish_use_subcommand' -a 'spec' -d 'full reference as JSON'
complete -c chore -f -n '__fish_use_subcommand' -a 'completions' -d 'shell completion'
complete -c chore -l dry -d 'echo commands without side effects'
complete -c chore -l force -d 'disable run-once'
"#;

const POWERSHELL: &str = r#"# chore completions for PowerShell.
Register-ArgumentCompleter -Native -CommandName chore -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $lines = @()
    try { $lines = @(chore list --names 2>$null) } catch { }
    $lines += "list`ttasks and descriptions"
    $lines += "help`tsyntax and builtins"
    $lines += "check`tlint without running"
    $lines += "spec`tfull reference as JSON"
    $lines += "completions`tshell completion"
    foreach ($line in $lines) {
        $parts = $line -split "`t", 2
        $name = $parts[0]
        if ($name -like "$wordToComplete*") {
            $desc = if ($parts.Length -gt 1 -and $parts[1]) { $parts[1] } else { $name }
            [System.Management.Automation.CompletionResult]::new(
                $name, $name, 'ParameterValue', $desc)
        }
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_name_it_prints() {
        for shell in Shell::all() {
            assert_eq!(Shell::parse(&shell.to_string()), Some(shell));
        }
    }

    #[test]
    fn pwsh_is_an_alias() {
        assert_eq!(Shell::parse("pwsh"), Some(Shell::PowerShell));
        assert_eq!(Shell::parse("nushell"), None);
    }

    #[test]
    fn every_script_calls_list_names() {
        for shell in Shell::all() {
            assert!(
                shell.script().contains("chore list --names"),
                "{shell} script must ask chore for the task list"
            );
        }
    }
}
