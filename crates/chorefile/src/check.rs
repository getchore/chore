//! `chore check` — everything that can be known without running anything.
//!
//! Reports syntax errors, tasks named after a reserved subcommand or builtin,
//! duplicate task names across a flat `include`, include cycles, unknown
//! commands, undefined variables, and non-portable commands with the builtin
//! that replaces them.
//!
//! Nothing here touches the filesystem except to look a command up on `PATH`,
//! and nothing is evaluated: `check` works on a file whose globals read paths
//! that do not exist yet.
//!
//! The one place platform matters is that `PATH` lookup, and it is skipped
//! inside an `if` that this machine's `$OS`, `$ARCH` or `$ENV` decides against
//! — see [`host_value`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::{
    Block, Chain, Command, CompareOp, Cond, File, If, PartKind, Stmt, Task, VarRef, Word,
};
use crate::error::{Error, Location, Span};
use crate::{FILE_NAME, NAMESPACE_SEP, RESERVED_TASKS, builtins, parse, vars};

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The chorefile is wrong: running it would fail, or would run something
    /// other than what it says.
    Error,
    /// Worth knowing, but possibly fine. A `PATH` miss is the whole reason
    /// this exists — the machine linting is not always the machine running.
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// What to do instead, when there is an obvious answer.
    pub help: Option<String>,
    pub at: Location,
}

impl Diagnostic {
    fn error(message: String, at: Location) -> Self {
        Self {
            severity: Severity::Error,
            message,
            help: None,
            at,
        }
    }

    fn warning(message: String, at: Location) -> Self {
        Self {
            severity: Severity::Warning,
            message,
            help: None,
            at,
        }
    }

    fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Turn a parse failure into a diagnostic, so a caller has one uniform
    /// list whether or not the file parsed.
    pub fn from_error(error: &Error, path: &Path) -> Self {
        match error {
            Error::Syntax { message, at } => {
                let mut at = at.clone();
                // The lexer reports without a path; fill in the one we know.
                if at.file.as_os_str().is_empty() {
                    at.file = path.to_path_buf();
                }
                Self::error(message.clone(), at)
            }
            other => Self::error(other.to_string(), Location::new(path, Span::new(0, 0))),
        }
    }
}

/// Parse `source` and check it. A syntax error is reported as a diagnostic
/// rather than returned, because nothing else can be checked once parsing
/// fails and callers want one list either way.
pub fn check_source(source: &str, path: &Path) -> Vec<Diagnostic> {
    match parse::parse(source, path) {
        Ok(file) => check(&file, source, path),
        Err(e) => vec![Diagnostic::from_error(&e, path)],
    }
}

/// Check an already-parsed file. `source` is needed to sort findings into
/// file order.
pub fn check(file: &File, source: &str, path: &Path) -> Vec<Diagnostic> {
    let mut checker = Checker {
        path,
        tasks: file.tasks.iter().map(|t| t.name.as_str()).collect(),
        globals: file.globals.iter().map(|g| g.name.as_str()).collect(),
        namespaces: file
            .includes
            .iter()
            .filter_map(|i| i.namespace.as_deref())
            .collect(),
        on_path: HashMap::new(),
        out: Vec::new(),
    };

    checker.includes(file);
    checker.names(file, source);

    // Globals see only the globals written above them; a task body sees all
    // of them, since every global is evaluated before the first task runs.
    let mut scope = Scope {
        task: None,
        names: HashSet::new(),
        off_platform: false,
    };
    for global in &file.globals {
        checker.word(&global.value, &scope);
        scope.names.insert(global.name.clone());
    }
    for task in &file.tasks {
        let mut scope = Scope {
            task: Some(task),
            names: checker.globals.iter().map(|g| (*g).to_string()).collect(),
            off_platform: false,
        };
        checker.block(&task.body, &mut scope);
    }

    checker
        .out
        .sort_by_key(|d| (d.at.line_col(source), d.at.span.start));
    checker.out
}

/// What is in scope at one point in a walk.
struct Scope<'a> {
    /// `None` at the top level, where there are no arguments at all.
    task: Option<&'a Task>,
    /// Globals, plus locals and loop variables bound so far.
    names: HashSet<String>,
    /// Inside an `if` whose condition this machine's platform decides against.
    /// A `PATH` miss here is not evidence of anything: see [`host_value`].
    off_platform: bool,
}

struct Checker<'a> {
    path: &'a Path,
    tasks: HashSet<&'a str>,
    globals: HashSet<&'a str>,
    namespaces: HashSet<&'a str>,
    /// `PATH` lookups are filesystem hits; a chorefile calls `cargo` dozens of
    /// times and one answer is enough.
    on_path: HashMap<String, bool>,
    out: Vec<Diagnostic>,
}

impl Checker<'_> {
    fn at(&self, span: Span) -> Location {
        Location::new(self.path, span)
    }

    fn push(&mut self, d: Diagnostic) {
        self.out.push(d);
    }

    // -- names --------------------------------------------------------------

    /// Reserved names, shadowed builtins, and duplicates in a flat merge.
    ///
    /// A task named after a subcommand and a task named after a builtin fail
    /// in opposite directions: `chore list` is always the subcommand, so that
    /// task is dead code, while a task named `write` wins over the builtin and
    /// takes the name away from it. The two messages say so separately.
    fn names(&mut self, file: &File, source: &str) {
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for task in &file.tasks {
            let name = task.name.as_str();
            let at = self.at(task.span);

            if RESERVED_TASKS.contains(&name) {
                self.push(
                    Diagnostic::error(
                        format!("task `{name}` can never run: `chore {name}` is a subcommand"),
                        at.clone(),
                    )
                    .with_help(format!(
                        "rename it — `chore {name}` will always mean the subcommand, whatever \
                         this chorefile says"
                    )),
                );
            } else if builtins::is_builtin(name) {
                // The opposite of the subcommand case above: resolution is
                // task → builtin → PATH, so the task wins and it is the
                // builtin that becomes unreachable.
                self.push(
                    Diagnostic::error(
                        format!(
                            "task `{name}` shadows the `{name}` builtin: every `{name}` in this \
                             chorefile runs the task instead"
                        ),
                        at.clone(),
                    )
                    .with_help(format!(
                        "builtin names are reserved; rename the task — otherwise another task \
                         that calls `{name}` meaning the builtin silently gets this one, and \
                         there is no spelling left that reaches the builtin"
                    )),
                );
            } else if name == "cd" {
                // Handled by the interpreter before task lookup, so a task
                // named `cd` is dead code.
                self.push(
                    Diagnostic::error(
                        "task `cd` would never run: `cd` is handled by the interpreter itself"
                            .into(),
                        at.clone(),
                    )
                    .with_help("rename the task"),
                );
            }

            if name.contains(NAMESPACE_SEP) {
                self.push(
                    Diagnostic::error(
                        format!("task name `{name}` contains `{NAMESPACE_SEP}`"),
                        at.clone(),
                    )
                    .with_help(format!(
                        "`{NAMESPACE_SEP}` is reserved for names that come from \
                         `include ... as`; use `-` or `_` instead"
                    )),
                );
            }

            self.duplicate("task", name, task.span, &mut seen, source);
        }

        let mut seen: HashMap<&str, Span> = HashMap::new();
        for global in &file.globals {
            self.duplicate("global", &global.name, global.span, &mut seen, source);
        }
    }

    fn duplicate<'n>(
        &mut self,
        kind: &str,
        name: &'n str,
        span: Span,
        seen: &mut HashMap<&'n str, Span>,
        source: &str,
    ) {
        match seen.get(name) {
            Some(first) => {
                let (line, _) = self.at(*first).line_col(source);
                self.push(
                    Diagnostic::error(format!("duplicate {kind} `{name}`"), self.at(span))
                        .with_help(format!(
                            "`{name}` is already defined on line {line}; the later definition wins \
                         and the earlier one is unreachable, so rename one or merge them"
                        )),
                );
            }
            None => {
                seen.insert(name, span);
            }
        }
    }

    // -- includes -----------------------------------------------------------

    /// What is knowable about `include` before includes are followed:
    /// self-inclusion, a repeated path, and a broken or colliding namespace.
    fn includes(&mut self, file: &File) {
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut namespaces: HashSet<&str> = HashSet::new();
        let this = normalize(self.path);

        for include in &file.includes {
            let at = self.at(include.span);
            let resolved = resolve_include(self.path, &include.path);

            if resolved == this {
                self.push(
                    Diagnostic::error(
                        format!("`include {}` includes this file itself", include.path),
                        at.clone(),
                    )
                    .with_help("an include cycle never terminates; remove it"),
                );
            } else if !seen.insert(resolved) {
                self.push(
                    Diagnostic::error(
                        format!("`{}` is included more than once", include.path),
                        at.clone(),
                    )
                    .with_help(
                        "a flat include merges every name, so including the same file twice \
                         makes every one of its tasks a duplicate; remove the second include",
                    ),
                );
            }

            if let Some(ns) = &include.namespace {
                if ns.contains(NAMESPACE_SEP) {
                    self.push(
                        Diagnostic::error(
                            format!("include namespace `{ns}` contains `{NAMESPACE_SEP}`"),
                            at.clone(),
                        )
                        .with_help(format!(
                            "`{NAMESPACE_SEP}` joins a namespace to a task name, so it cannot \
                             appear inside one; use a single word"
                        )),
                    );
                }
                if self.tasks.contains(ns.as_str()) {
                    self.push(
                        Diagnostic::error(
                            format!("include namespace `{ns}` is also the name of a task"),
                            at.clone(),
                        )
                        .with_help(format!(
                            "rename one of them — `chore {ns}` would be ambiguous between the \
                             task and the namespace"
                        )),
                    );
                }
                if !namespaces.insert(ns.as_str()) {
                    self.push(
                        Diagnostic::error(
                            format!("include namespace `{ns}` is used twice"),
                            at.clone(),
                        )
                        .with_help(
                            "two includes sharing a namespace collide exactly as a flat \
                             include would; give each one its own name",
                        ),
                    );
                }
            }
        }
    }

    // -- statements ---------------------------------------------------------

    fn block(&mut self, block: &Block, scope: &mut Scope) {
        for stmt in block {
            self.stmt(stmt, scope);
        }
    }

    fn stmt(&mut self, stmt: &Stmt, scope: &mut Scope) {
        match stmt {
            Stmt::Assign(a) => {
                self.word(&a.value, scope);
                scope.names.insert(a.name.clone());
            }
            Stmt::Command(chain) | Stmt::Try(chain) => self.chain(chain, scope),
            Stmt::If(node) => {
                self.constant_cond(node);
                // The condition itself runs wherever the `if` does, so it is
                // walked before either arm narrows the platform.
                self.cond(&node.cond, scope);

                // A branch this machine's platform decides against is walked
                // with `off_platform` set, which drops `PATH` misses inside it
                // and nothing else. Once set it stays set: a guard nested in a
                // branch that never runs here does not run here either.
                let outer = scope.off_platform;
                let decided = host_value(&node.cond, scope);

                // Both arms share one scope: a name bound in either is treated
                // as bound afterwards. It over-accepts, which is the right way
                // to be wrong — a false "undefined" is worse than a miss.
                scope.off_platform = outer || decided == Some(false);
                self.block(&node.then, scope);
                if let Some(otherwise) = &node.otherwise {
                    // `else` runs on every platform the condition excludes, so
                    // it is off-platform only when the condition always holds.
                    scope.off_platform = outer || decided == Some(true);
                    self.block(otherwise, scope);
                }
                scope.off_platform = outer;
            }
            Stmt::For(node) => {
                if node.items.is_empty() {
                    self.push(
                        Diagnostic::error(
                            format!("`for {}` has no items, so the body never runs", node.var),
                            self.at(node.span),
                        )
                        .with_help(
                            "give the loop something to iterate: a list of words, a `$var`, or \
                             a `$( ... )` capture",
                        ),
                    );
                }
                for item in &node.items {
                    self.word(item, scope);
                }
                let fresh = scope.names.insert(node.var.clone());
                self.block(&node.body, scope);
                // The loop variable dies with the loop, unless it shadowed
                // something already in scope.
                if fresh {
                    scope.names.remove(&node.var);
                }
            }
            Stmt::Exit(code) => {
                if let Some(word) = code {
                    self.word(word, scope);
                }
            }
        }
    }

    /// A condition that no variable can influence, and so is decided at the
    /// time it is written.
    ///
    /// This is what a forgotten `$` looks like: `if os == windows` compares
    /// the word `os` with the word `windows` and is simply false. The finding
    /// points at the header, since it is the condition as a whole — not one
    /// word in it — that is wrong.
    fn constant_cond(&mut self, node: &If) {
        let Some(value) = constant(&node.cond) else {
            return;
        };
        self.push(
            Diagnostic::error(
                format!("this condition is always {value}: nothing in it can vary"),
                self.at(node.span),
            )
            .with_help(
                "every side of the comparison is literal text, so the result is fixed — a word \
                 without a `$` is not a variable",
            ),
        );
    }

    fn cond(&mut self, cond: &Cond, scope: &mut Scope) {
        match cond {
            Cond::Compare { left, right, .. } => {
                self.word(left, scope);
                self.word(right, scope);
            }
            Cond::Command(chain) => self.chain(chain, scope),
            Cond::Not(inner) => self.cond(inner, scope),
            Cond::And(a, b) | Cond::Or(a, b) => {
                self.cond(a, scope);
                self.cond(b, scope);
            }
        }
    }

    fn chain(&mut self, chain: &Chain, scope: &Scope) {
        match chain {
            Chain::Single(cmd) => self.command(cmd, scope),
            Chain::And(a, b) | Chain::Or(a, b) | Chain::Pipe(a, b) => {
                self.chain(a, scope);
                self.chain(b, scope);
            }
        }
    }

    fn command(&mut self, cmd: &Command, scope: &Scope) {
        self.word(&cmd.name, scope);
        for arg in &cmd.args {
            self.word(arg, scope);
        }
        for r in &cmd.redirects {
            self.word(&r.target, scope);
        }

        // A name built from a variable or a capture is only knowable at run
        // time; there is nothing honest to say about it.
        let Some(name) = literal(&cmd.name) else {
            return;
        };
        let args: Vec<&str> = cmd.args.iter().filter_map(literal).collect();
        self.portability(name, &args, cmd);
        self.resolution(name, cmd, scope);
    }

    /// The check that gives `chore` its point: a command that works on the
    /// author's machine and nowhere else.
    fn portability(&mut self, name: &str, args: &[&str], cmd: &Command) {
        let Some((pattern, replacement)) = builtins::REPLACEMENTS
            .iter()
            .find(|(pattern, _)| matches(pattern, name, args))
        else {
            return;
        };
        // A builtin of the same name already wins over `PATH`, so only a
        // `^`-forced call actually reaches the non-portable program.
        if !cmd.force_path && builtins::is_builtin(name) {
            return;
        }
        let called = if cmd.force_path {
            format!("^{name}")
        } else {
            name.to_string()
        };
        self.push(
            Diagnostic::error(
                format!(
                    "`{called}` is not portable: it is missing, or spelled differently, on at \
                     least one platform this chorefile can run on"
                ),
                self.at(cmd.span),
            )
            .with_help(advice(pattern, replacement)),
        );
    }

    /// task → builtin → `PATH`, exactly as the interpreter resolves it.
    ///
    /// The `PATH` step is the only one that depends on the machine running
    /// `check`, so it is the only one a platform guard can silence.
    fn resolution(&mut self, name: &str, cmd: &Command, scope: &Scope) {
        if !cmd.force_path {
            if name == "cd" || self.tasks.contains(name) || builtins::is_builtin(name) {
                return;
            }
            // Includes are not followed, so a namespaced name is taken on
            // trust rather than reported as unknown.
            if let Some((ns, _)) = name.split_once(NAMESPACE_SEP) {
                if self.namespaces.contains(ns) {
                    return;
                }
            }
        }
        // Nothing to report, and nothing to look up: this branch cannot run on
        // this machine, so whether the command is installed on this machine is
        // not a fact about the chorefile.
        if scope.off_platform {
            return;
        }
        if self.lookup_path(name) {
            return;
        }

        let called = if cmd.force_path {
            format!("^{name}")
        } else {
            name.to_string()
        };
        let mut help = format!(
            "`check` looked on this machine's `PATH`, which is not necessarily the machine that \
             runs the task — if `{name}` is installed only in CI or in a container, this is fine"
        );
        if !cmd.force_path {
            if let Some(similar) = self.suggestion(name) {
                help = format!("did you mean `{similar}`? Otherwise: {help}");
            }
        }
        self.push(
            Diagnostic::warning(
                format!("`{called}` is not a task, not a builtin, and was not found on `PATH`"),
                self.at(cmd.span),
            )
            .with_help(help),
        );
    }

    /// The nearest task or builtin name, when the difference looks like a typo.
    fn suggestion(&self, name: &str) -> Option<String> {
        let candidates = self
            .tasks
            .iter()
            .copied()
            .chain(builtins::NAMES.iter().copied());
        nearest(name, candidates)
    }

    fn lookup_path(&mut self, name: &str) -> bool {
        // A name with a separator is a path relative to the run's directory,
        // which `check` cannot know. Say nothing rather than guess.
        if name.contains('/') || name.contains('\\') {
            return true;
        }
        if let Some(known) = self.on_path.get(name) {
            return *known;
        }
        let found = on_path(name);
        self.on_path.insert(name.to_string(), found);
        found
    }

    // -- variables ----------------------------------------------------------

    fn word(&mut self, word: &Word, scope: &Scope) {
        for part in &word.parts {
            match &part.kind {
                PartKind::Literal(_) => {}
                // The part's own span, not the word's: `"$a/$b"` is one word
                // and two findings, each pointing at its own `$`.
                PartKind::Var(var) => self.var(var, part.span, scope),
                PartKind::Capture(chain) => self.chain(chain, scope),
            }
        }
    }

    fn var(&mut self, var: &VarRef, span: Span, scope: &Scope) {
        let at = self.at(span);
        match var {
            VarRef::Named(name) => {
                if vars::BUILTIN_NAMES.contains(&name.as_str()) || scope.names.contains(name) {
                    return;
                }
                let mut d = Diagnostic::error(format!("undefined variable `${name}`"), at);
                let known = scope
                    .names
                    .iter()
                    .map(String::as_str)
                    .chain(vars::BUILTIN_NAMES.iter().copied());
                d = match nearest(name, known) {
                    Some(similar) => d.with_help(format!("did you mean `${similar}`?")),
                    None => d.with_help(format!(
                        "assign `{name}=...` before this line, or add it as a top-level global"
                    )),
                };
                self.push(d);
            }
            VarRef::Positional(n) => match scope.task {
                Some(task) if *n <= task.params.len() && *n > 0 => {}
                Some(task) => {
                    let declared = if task.params.is_empty() {
                        "no parameters".to_string()
                    } else {
                        format!(
                            "{} parameter(s) ({})",
                            task.params.len(),
                            task.params.join(", ")
                        )
                    };
                    self.push(
                        Diagnostic::error(
                            format!(
                                "`${n}` is never set: task `{}` declares {declared}",
                                task.name
                            ),
                            at,
                        )
                        .with_help(format!(
                            "add the parameter to the header — `task {} {}`",
                            task.name,
                            header_params(task, *n)
                        )),
                    );
                }
                None => self.push(
                    Diagnostic::error(format!("`${n}` is only defined inside a task"), at)
                        .with_help(
                            "the top level takes no arguments; move this into a task with \
                             declared parameters",
                        ),
                ),
            },
            VarRef::All | VarRef::Count => {
                if scope.task.is_none() {
                    let name = if matches!(var, VarRef::All) {
                        "$@"
                    } else {
                        "$#"
                    };
                    self.push(
                        Diagnostic::error(format!("`{name}` is only defined inside a task"), at)
                            .with_help(
                                "the top level takes no arguments, so this always expands to \
                                 nothing; move it into a task",
                            ),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The value of a condition that holds no variable, capture or command, and
/// so is the same on every run and every machine.
fn constant(cond: &Cond) -> Option<bool> {
    match cond {
        Cond::Compare { left, op, right } => Some(compare(*op, literal(left)?, literal(right)?)),
        // A command's exit code is knowable only by running it.
        Cond::Command(_) => None,
        Cond::Not(inner) => Some(!constant(inner)?),
        Cond::And(a, b) => Some(constant(a)? && constant(b)?),
        Cond::Or(a, b) => Some(constant(a)? || constant(b)?),
    }
}

/// What a condition is worth **on the machine running `check`**, when that is
/// knowable without running anything: `Some(false)` means this host never
/// enters the branch, `None` means the analysis cannot tell.
///
/// This exists for one finding. `check` looks a command up on the machine it
/// runs on, but a chorefile is written for every machine it will ever run on,
/// and `gendef` inside `if $OS == windows && $ENV == gnu { ... }` cannot exist
/// on a macOS host. Reporting its absence there is a false positive on a
/// correct file, and a linter that cannot lint a cross-platform chorefile has
/// no business gating cross-platform CI.
///
/// The analysis is deliberately shy, and asymmetric in what it costs to be
/// wrong. `Some` is a *claim*, and one is made only when every operand that
/// decides the condition is literal text or one of the read-only platform
/// variables — `$OS`, `$ARCH`, `$ENV`, `$PLATFORM`, `$EXE` — and none of those
/// names has been shadowed by an assignment. A command's exit code, `$HOME`, a
/// global, a `$( ... )` capture, or a name the chorefile bound itself all
/// collapse the whole condition to `None`, and `None` keeps the finding. A
/// warning missed under an unusual guard costs one line of output; a warning
/// invented under a correct one costs the tool its exit code.
fn host_value(cond: &Cond, scope: &Scope) -> Option<bool> {
    match cond {
        Cond::Compare { left, op, right } => Some(compare(
            *op,
            &host_text(left, scope)?,
            &host_text(right, scope)?,
        )),
        Cond::Command(_) => None,
        Cond::Not(inner) => Some(!host_value(inner, scope)?),
        // Short-circuiting on the value, not on the operand: `$OS == windows
        // && which gendef` is false here whatever `which` would say.
        Cond::And(a, b) => match (host_value(a, scope), host_value(b, scope)) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Cond::Or(a, b) => match (host_value(a, scope), host_value(b, scope)) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
    }
}

/// The word's value on this machine, if every part of it is literal text or a
/// read-only platform variable. Whole words rather than lone variables, so
/// `$OS-$ARCH` and `app$EXE` resolve as readily as `$OS`.
fn host_text(word: &Word, scope: &Scope) -> Option<String> {
    let mut out = String::new();
    for part in &word.parts {
        match &part.kind {
            PartKind::Literal(text) => out.push_str(text),
            PartKind::Var(VarRef::Named(name)) => out.push_str(&platform_var(name, scope)?),
            // A capture, an argument, or `$@`: not decided by the platform.
            _ => return None,
        }
    }
    Some(out)
}

/// One read-only platform variable's value here, unless the chorefile has
/// bound a name of its own over it — in which case `check` has no idea what it
/// holds and must not pretend otherwise.
fn platform_var(name: &str, scope: &Scope) -> Option<String> {
    if scope.names.contains(name) {
        return None;
    }
    match name {
        "OS" => Some(vars::OS.to_string()),
        "ARCH" => Some(vars::ARCH.to_string()),
        "ENV" => Some(vars::ENV.to_string()),
        "PLATFORM" => Some(vars::platform()),
        "EXE" => Some(vars::EXE.to_string()),
        _ => None,
    }
}

/// One comparison, over values that are already known.
fn compare(op: CompareOp, left: &str, right: &str) -> bool {
    match op {
        CompareOp::Eq => left == right,
        CompareOp::Ne => left != right,
        CompareOp::Contains => left.contains(right),
        CompareOp::StartsWith => left.starts_with(right),
        CompareOp::EndsWith => left.ends_with(right),
    }
}

/// The word's text, if it is entirely literal.
fn literal(word: &Word) -> Option<&str> {
    match word.parts.as_slice() {
        [part] => match &part.kind {
            PartKind::Literal(text) => Some(text.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// A [`builtins::REPLACEMENTS`] pattern is either a bare command name or a
/// name plus the flag that makes it non-portable (`mkdir -p`).
fn matches(pattern: &str, name: &str, args: &[&str]) -> bool {
    match pattern.split_once(' ') {
        Some((head, flag)) => head == name && args.contains(&flag),
        None => pattern == name,
    }
}

/// Why the builtin is better, in the terms of the command being replaced.
fn advice(pattern: &str, replacement: &str) -> String {
    match pattern {
        "curl" | "wget" => "use the `download` builtin — it speaks https and \
             `gh://owner/repo/tag/asset`, takes `--retries`, `--timeout` and `--sha256`, and \
             needs nothing installed on the machine"
            .into(),
        "unzip" => "use the `extract` builtin — it unpacks zip, tar, `.gz`, `.xz` and `.zst` \
             with the same flags everywhere, and Windows has no `unzip`"
            .into(),
        "tar" => "use `extract` to unpack and `archive` to create — each picks the format from \
             the extension, so no flag soup and no bsdtar-vs-GNU-tar differences"
            .into(),
        "cp" => "use the `copy` builtin — it copies a file or a whole directory, with no `-r` \
             on one platform and `-R` on another"
            .into(),
        "mv" => "use the `move` builtin — same behavior on every platform, including across \
             filesystems"
            .into(),
        "rm" => "use the `remove` builtin — it is recursive and succeeds on a missing path, so \
             no `-rf` and no `|| true`"
            .into(),
        "mkdir -p" => "use the `mkdir` builtin — it already has `-p` semantics, and `-p` is not \
             what Windows `mkdir` means"
            .into(),
        "cat" => "use the `read` builtin — it prints a file's contents, trimmed, where Windows \
             has `type` and not `cat`"
            .into(),
        "shasum" | "sha256sum" => "use the `sha256` builtin — one name and one output format on \
             every platform, instead of `shasum -a 256` here and `sha256sum` there"
            .into(),
        "test" => "use the `exists` builtin — `if exists path { ... }` is the portable spelling, \
             and `[` is not a program on Windows"
            .into(),
        "sleep" => "use the `sleep` builtin — Windows has no `sleep` program".into(),
        _ => format!(
            "use the `{replacement}` builtin — `chore` implements it, so it behaves identically \
             on macOS, Linux and Windows"
        ),
    }
}

/// A plausible `task` header for a task that reads `$n`.
fn header_params(task: &Task, n: usize) -> String {
    let mut params: Vec<String> = task.params.clone();
    while params.len() < n {
        params.push(format!("arg{}", params.len() + 1));
    }
    params.join(" ")
}

/// The closest candidate, when it is close enough to be a typo rather than a
/// different word.
fn nearest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let limit = (name.chars().count() / 3).max(1);
    candidates
        .filter(|c| *c != name)
        .map(|c| (distance(name, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, c)| (*d, c.len()))
        .map(|(_, c)| c.to_string())
}

/// Levenshtein distance. Only ever asked about short identifiers.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut row = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            row[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1);
        }
        std::mem::swap(&mut prev, &mut row);
    }
    prev[b.len()]
}

/// Where an `include` points: relative to the including file, and a directory
/// means the `chorefile` inside it.
fn resolve_include(from: &Path, path: &str) -> PathBuf {
    let base = from.parent().unwrap_or_else(|| Path::new("."));
    let joined = base.join(vars::to_native(path));
    let joined = if joined.is_dir() {
        joined.join(FILE_NAME)
    } else {
        joined
    };
    normalize(&joined)
}

/// Collapse `.` and `foo/..` so two spellings of one path compare equal.
/// Purely lexical: the file may not exist yet, and `canonicalize` would fail.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Is `name` an executable on this machine's `PATH`?
///
/// Deliberately a warning's worth of evidence and no more: the machine running
/// `check` is not necessarily the machine running the task.
fn on_path(name: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|d| !d.as_os_str().is_empty())
        .any(|dir| candidates(&dir.join(name)).iter().any(|p| executable(p)))
}

/// The name as written, plus the `PATHEXT` variants on Windows.
fn candidates(base: &Path) -> Vec<PathBuf> {
    let mut out = vec![base.to_path_buf()];
    if cfg!(windows) {
        let ext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let name = base.file_name().unwrap_or_default().to_string_lossy();
        for suffix in ext.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            out.push(base.with_file_name(format!("{name}{suffix}")));
        }
    }
    out
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Windows has no execute bit: the extension decides, and [`candidates`] has
/// already applied it.
#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}
