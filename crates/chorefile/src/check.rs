//! `chore check` — everything that can be known without running anything.
//!
//! Reports syntax errors, tasks named after a reserved subcommand or builtin,
//! duplicate task names across a flat `include`, include cycles, an `include`
//! of a file that discovery can also find on its own, unknown commands,
//! undefined variables — in a parameter's default as much as in a
//! body — a parameter name declared twice in one header or read as `$name`
//! when parameters are positional, and non-portable commands with the builtin
//! that replaces them.
//!
//! Nothing here touches the filesystem except to look a command up on `PATH`,
//! and nothing is evaluated: `check` works on a file whose globals read paths
//! that do not exist yet.
//!
//! A `script` block is the one construct this module deliberately says almost
//! nothing about — see [`Checker::script`].
//!
//! The one place platform matters is that `PATH` lookup, and it is skipped
//! inside an `if` that this machine's `$OS`, `$ARCH` or `$ENV` decides against
//! — see [`host_value`].
//!
//! # One file, or a whole include tree
//!
//! [`check`] and [`check_source`] check one file on its own: includes are
//! recorded but not followed, so a name that could only come from an included
//! file is taken on trust rather than reported.
//!
//! [`check_merged`] checks a [`Merged`] tree — a chorefile and everything its
//! `include`s pulled in. Names resolve against the merged tables, so a task
//! calling `libs::build` is checked for real, and every finding points into the
//! file it actually came from rather than into the top-level one. Rendering
//! such a finding needs *that* file's text: use
//! [`Sources::render`](crate::resolve::Sources::render), never the top-level
//! source.
//!
//! # What `check` does not report
//!
//! Anything [`resolve`](crate::resolve) rejects outright never reaches this
//! module, because a rejected tree does not merge: an include cycle, a missing
//! or unreadable included file, and a duplicate name across a flat merge are
//! all `resolve` errors, reported by turning that error into a [`Diagnostic`]
//! with [`Diagnostic::from_error`] — see [`check_path`]. The include findings
//! here are the ones that survive a successful merge, plus everything
//! [`check_source`] can still say when it is looking at a single file.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::{
    Block, Chain, Command, CompareOp, Cond, File, If, Include, Param, PartKind, Script, Stmt, Task,
    VarRef, Word,
};
use crate::error::{Error, Location, Span};
use crate::resolve::Merged;
use crate::{
    FILE_EXT, FILE_NAME, NAMESPACE_SEP, RESERVED_TASKS, builtins, parse, require, resolve, vars,
};

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
///
/// One file only: `include` is recorded, not followed. Use [`check_path`] or
/// [`check_merged`] to check a chorefile together with what it includes.
pub fn check_source(source: &str, path: &Path) -> Vec<Diagnostic> {
    match parse::parse(source, path) {
        Ok(file) => check(&file, source, path),
        Err(e) => vec![Diagnostic::from_error(&e, path)],
    }
}

/// Check an already-parsed file. `source` is needed to sort findings into
/// file order.
pub fn check(file: &File, source: &str, path: &Path) -> Vec<Diagnostic> {
    let names = Names::from_file(file);
    // One file on its own is its own top level, so its directory is `$ROOT`.
    let root = path.parent().unwrap_or(Path::new("")).to_path_buf();
    let mut checker = Checker::new(path, &root, names, None, false, file);
    checker.run(file, source)
}

/// Resolve the chorefile at `path` — following its `include`s — and check the
/// whole tree.
///
/// The one entry point that can report *everything*: a failure to merge is a
/// diagnostic like any other finding rather than an error the caller has to
/// handle separately, so a missing included file, an unreadable one and an
/// include cycle all come back in the same list as an undefined variable.
///
/// The [`Merged`] is returned alongside the findings because rendering them
/// needs it: a finding may point into any file that contributed, and
/// [`Sources::render`](crate::resolve::Sources::render) is what turns it into
/// `path:line:col` using the text of *that* file.
pub fn check_path(path: &Path) -> (Vec<Diagnostic>, Option<Merged>) {
    match resolve::resolve(path) {
        Ok(merged) => {
            let found = check_merged(&merged);
            (found, Some(merged))
        }
        Err(e) => (vec![Diagnostic::from_error(&e, path)], None),
    }
}

/// Check a chorefile and everything it included.
///
/// Every contributing file is checked in its own right — so a finding carries
/// the path of the file it is really in — while every name is resolved against
/// the merged tables, so a call to `libs::build` and a global defined two
/// includes away are both checked for real.
///
/// The two halves of [`Merged`] are used for exactly what each is the
/// authority on: `parts` for positions and per-file structure, `file` for
/// names. Nothing here re-parses a source or re-derives a namespace — the
/// resolver already did both, and a second implementation of the namespacing
/// rule would be free to disagree with the one that actually ran.
pub fn check_merged(merged: &Merged) -> Vec<Diagnostic> {
    let names = Names::from_merged(&merged.file);
    let mut out = Vec::new();
    // One entry per *load*, so a file two includes both reach appears twice.
    // Its findings are about its own text and would be identical both times,
    // and the same message twice at the same `path:line:col` is noise, so the
    // first arrival is the one that reports. Which arrival that is does not
    // change what is found: a file's own names are in scope for it whatever
    // prefix it was given (see [`Checker::own_tasks`]), and every namespace
    // the run knows is in `names` either way.
    let mut seen: HashSet<&Path> = HashSet::new();
    for part in &merged.parts {
        if !seen.insert(part.path.as_path()) {
            continue;
        }
        let Some(source) = merged.sources.get(&part.path) else {
            // `parts` and `sources` are filled in together, so this cannot
            // happen; skipping beats unwrapping on a bookkeeping miss.
            continue;
        };
        let mut checker = Checker::new(
            &part.path,
            &merged.root,
            names.clone(),
            part.prefix.clone(),
            true,
            &part.file,
        );
        out.extend(checker.run(&part.file, source));
    }
    out
}

/// Every name a chorefile defines, as the merged tree spells them.
///
/// Owned rather than borrowed: in a merged tree the names outlive any one
/// file's parse, and there is one of these per run, not per task.
#[derive(Debug, Clone, Default)]
struct Names {
    tasks: HashSet<String>,
    globals: HashSet<String>,
    /// Every `as` namespace, from the merged names and from the `include`
    /// directives themselves.
    namespaces: HashSet<String>,
}

impl Names {
    fn from_file(file: &File) -> Self {
        let mut names = Self {
            tasks: file.tasks.iter().map(|t| t.name.clone()).collect(),
            globals: file.globals.iter().map(|g| g.name.clone()).collect(),
            namespaces: HashSet::new(),
        };
        names.absorb_namespaces(file);
        names
    }

    /// The merged tree's own tables. Names are already namespaced here, so
    /// `libs::build` is what `tasks` holds and the namespaces fall out of it.
    fn from_merged(file: &File) -> Self {
        let mut names = Self::from_file(file);
        let qualified: Vec<String> = names
            .tasks
            .iter()
            .chain(names.globals.iter())
            .filter_map(|n| n.split_once(NAMESPACE_SEP).map(|(ns, _)| ns.to_string()))
            .collect();
        names.namespaces.extend(qualified);
        names
    }

    fn absorb_namespaces(&mut self, file: &File) {
        for include in &file.includes {
            if let Some(ns) = &include.namespace {
                self.namespaces.insert(ns.clone());
            }
        }
    }
}

/// What is in scope at one point in a walk.
struct Scope<'a> {
    /// `None` at the top level, where there are no arguments at all.
    task: Option<&'a Task>,
    /// How many of the task's parameters are bound here. A body sees all of
    /// them — an optional one is bound to its default when the caller passes
    /// nothing, so `$2` is set either way. A parameter's *default* is
    /// evaluated while the frame is still being built, and sees only the
    /// parameters declared before it.
    params_bound: usize,
    /// Globals, plus locals and loop variables bound so far.
    names: HashSet<String>,
    /// Inside an `if` whose condition this machine's platform decides against.
    /// A `PATH` miss here is not evidence of anything: see [`host_value`].
    off_platform: bool,
}

struct Checker<'a> {
    path: &'a Path,
    /// `$ROOT` for the run: the directory of the *top-level* chorefile, not of
    /// this file. Only [`Checker::discoverable`] needs it, and it needs the
    /// top-level one — whether an included file is inside the project is a
    /// question about the project, not about whichever file did the including.
    root: PathBuf,
    /// Every name in scope for this file: the merged tables when includes were
    /// followed, this file's own names when they were not.
    names: Names,
    /// The names this file defines itself, unprefixed and exactly as written.
    ///
    /// A task calling a sibling in its own file writes the bare name whatever
    /// namespace the merge gave the file, so these are always in scope — and
    /// they are what keeps a prefix this walk failed to recover from turning
    /// a correct call into an invented "unknown command".
    own_tasks: HashSet<String>,
    own_globals: HashSet<String>,
    /// The namespace prefix this file's names carry in the merged tree.
    prefix: Option<String>,
    /// Were includes followed? Decides whether a namespaced name that is not
    /// in the tables is unknown or merely unknowable.
    merged: bool,
    /// `PATH` lookups are filesystem hits; a chorefile calls `cargo` dozens of
    /// times and one answer is enough.
    on_path: HashMap<String, bool>,
    /// How many `script` blocks this file has, and where the first one is —
    /// the two things the once-per-file summary in [`Checker::unchecked`]
    /// needs.
    scripts: usize,
    first_script: Option<Span>,
    out: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    fn new(
        path: &'a Path,
        root: &Path,
        mut names: Names,
        prefix: Option<String>,
        merged: bool,
        file: &File,
    ) -> Self {
        // This file's own `as` namespaces matter even in a merged tree: an
        // include that pulled in nothing still names a namespace.
        names.absorb_namespaces(file);
        Self {
            path,
            root: normalize(root),
            names,
            own_tasks: file.tasks.iter().map(|t| t.name.clone()).collect(),
            own_globals: file.globals.iter().map(|g| g.name.clone()).collect(),
            prefix,
            merged,
            on_path: HashMap::new(),
            scripts: 0,
            first_script: None,
            out: Vec::new(),
        }
    }

    /// Everything checkable about one file, in source order.
    fn run(&mut self, file: &File, source: &str) -> Vec<Diagnostic> {
        self.requirement(file);
        self.includes(file);
        self.declared_names(file, source);

        // Globals see the globals written above them, plus every global that
        // came from another file — the merge decides that order, not this
        // file, so an included file's global is simply in scope. With no
        // includes followed there are no such globals and the rule is the
        // familiar one: assign before use.
        let outside = self.outside_globals();
        let mut scope = Scope {
            task: None,
            params_bound: 0,
            names: outside.clone(),
            off_platform: false,
        };
        for global in &file.globals {
            self.assignment(&global.name, global.span);
            self.word(&global.value, &scope);
            scope.names.insert(global.name.clone());
        }

        // A task body sees every global, since all of them are evaluated
        // before the first task runs.
        let mut visible = outside;
        visible.extend(self.own_globals.iter().cloned());
        for task in &file.tasks {
            self.params(task, &visible);
            let mut scope = Scope {
                task: Some(task),
                params_bound: task.params.len(),
                names: visible.clone(),
                off_platform: false,
            };
            self.block(&task.body, &mut scope);
        }
        self.unchecked();

        let mut out = std::mem::take(&mut self.out);
        out.sort_by_key(|d| (d.at.line_col(source), d.at.span.start));
        out
    }

    /// A `require` this binary does not meet.
    ///
    /// Reported per file rather than once per tree: `check` is a report, and
    /// an author fixing an include tree is better served knowing every file
    /// that will stop them than the strictest one. A *run* wants the opposite
    /// and gets it from [`require::unmet`], which reports only the version
    /// that makes all of them go away.
    fn requirement(&mut self, file: &File) {
        if let Some(unmet) = require::unmet_in(file, self.path) {
            self.out
                .push(Diagnostic::error(unmet.message(), unmet.at.clone()).with_help(unmet.help()));
        }
    }

    /// The globals that reached this file from somewhere else, under both the
    /// spelling the merge gave them and the bare one this file writes.
    fn outside_globals(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for global in &self.names.globals {
            if self.own_globals.contains(global) {
                continue;
            }
            if let Some(bare) = self.strip_prefix(global) {
                // A global of this file's own, seen through its merged name.
                if self.own_globals.contains(bare) {
                    continue;
                }
                out.insert(bare.to_string());
            }
            out.insert(global.clone());
        }
        out
    }

    fn at(&self, span: Span) -> Location {
        Location::new(self.path, span)
    }

    fn push(&mut self, d: Diagnostic) {
        self.out.push(d);
    }

    // -- merged name lookup ---------------------------------------------------

    /// This name as the merged tree spells it, when this file is namespaced.
    fn qualified(&self, name: &str) -> Option<String> {
        self.prefix
            .as_ref()
            .map(|p| format!("{p}{NAMESPACE_SEP}{name}"))
    }

    /// The bare name behind a merged one, when it belongs to this file.
    fn strip_prefix<'n>(&self, name: &'n str) -> Option<&'n str> {
        let prefix = self.prefix.as_deref()?;
        name.strip_prefix(prefix)?.strip_prefix(NAMESPACE_SEP)
    }

    fn is_task(&self, name: &str) -> bool {
        self.own_tasks.contains(name)
            || self.names.tasks.contains(name)
            || self
                .qualified(name)
                .is_some_and(|q| self.names.tasks.contains(&q))
    }

    /// Every task name a call in this file could plausibly have meant.
    fn candidate_tasks(&self) -> impl Iterator<Item = &str> {
        self.own_tasks
            .iter()
            .chain(self.names.tasks.iter())
            .map(String::as_str)
    }

    // -- names --------------------------------------------------------------

    /// Reserved names, shadowed builtins, and duplicates in a flat merge.
    ///
    /// A task named after a subcommand and a task named after a builtin fail
    /// in opposite directions: `chore list` is always the subcommand, so that
    /// task is dead code, while a task named `write` wins over the builtin and
    /// takes the name away from it. The two messages say so separately.
    fn declared_names(&mut self, file: &File, source: &str) {
        let mut seen: HashMap<&str, Span> = HashMap::new();
        for task in &file.tasks {
            let name = task.name.as_str();
            let at = self.at(task.span);

            // Only at the top level: `chore list` is the subcommand, but
            // `chore libs::list` is not, so a namespaced task of that name is
            // reachable and fine.
            if RESERVED_TASKS.contains(&name) && self.prefix.is_none() {
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

    /// An assignment that the interpreter will not honour.
    ///
    /// `$ROOT` is answered from the run itself, not from the variable map, so
    /// a chorefile that assigns it changes nothing — and would be reading a
    /// different root from every builtin if it did. With includes that is
    /// worse than a dead line: an included file could otherwise move the whole
    /// project's root out from under `remove`'s "never delete `$ROOT`" guard.
    ///
    /// The read-only platform variables are deliberately not covered: the
    /// interpreter does let a chorefile bind over `$OS` and friends, and
    /// [`platform_var`] already stops trusting one that has been bound.
    fn assignment(&mut self, name: &str, span: Span) {
        if name != "ROOT" {
            return;
        }
        self.push(
            Diagnostic::error(
                "assigning `ROOT` has no effect: `$ROOT` is the top-level chorefile's directory \
                 and is answered by the run, not by this variable"
                    .into(),
                self.at(span),
            )
            .with_help(
                "read `$ROOT` instead of setting it, and use a name of your own — `dist=$ROOT/dist` \
                 — for a directory you want to move",
            ),
        );
    }

    // -- parameters ---------------------------------------------------------

    /// A task's header: the names it declares, and the defaults it gives them.
    ///
    /// A default is a [`Word`] like any other, so it is walked like any other
    /// — otherwise `task t x=$nope { }` says nothing until the day someone
    /// calls `t` bare. What it may *see* is narrower than a body: it is
    /// evaluated while the frame is being built, so it reads the globals and
    /// the builtin variables, plus the parameters declared before it — `$1`
    /// while binding the second parameter — and nothing after.
    ///
    /// The order of required and optional parameters is the grammar's
    /// business, not this module's: a required parameter after an optional one
    /// never parses, so there is nothing here to report.
    fn params(&mut self, task: &Task, globals: &HashSet<String>) {
        let mut seen: HashSet<&str> = HashSet::new();
        for (i, param) in task.params.iter().enumerate() {
            if !seen.insert(param.name.as_str()) {
                self.push(
                    Diagnostic::error(
                        format!(
                            "task `{}` declares the parameter `{}` twice",
                            task.name, param.name
                        ),
                        self.at(param.span),
                    )
                    .with_help(format!(
                        "parameters are bound by position, so the second `{}` is `${}` and the \
                         first is still `${}` — give them different names",
                        param.name,
                        i + 1,
                        task.params
                            .iter()
                            .position(|p| p.name == param.name)
                            .unwrap_or(0)
                            + 1,
                    )),
                );
            }
            if let Some(default) = &param.default {
                let scope = Scope {
                    task: Some(task),
                    // Only what is already bound: the parameters to the left.
                    params_bound: i,
                    names: globals.clone(),
                    off_platform: false,
                };
                self.word(default, &scope);
            }
        }
    }

    // -- includes -----------------------------------------------------------

    /// What `check` still has to say about `include` once the tree has merged.
    ///
    /// Deliberately not here: an include cycle, a missing or unreadable file,
    /// and a duplicate name across a flat merge. Those stop `resolve` dead, so
    /// there is no merged tree to check and no second wording of the same
    /// finding — see the module docs. What remains is knowable from the
    /// directives alone, which is also everything [`check_source`] can say
    /// about a file it is looking at on its own.
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
            } else if seen.insert(resolved.clone()) {
                self.discoverable(include, &resolved, &at);
            } else {
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
                if self.is_task(ns) {
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

    /// An `include` whose target is itself named `chorefile`: a file that is
    /// both merged into this one *and* discoverable on its own.
    ///
    /// `chore` finds the chorefile governing a directory by walking up from
    /// the working directory to the first file named exactly `chorefile`, and
    /// the directory holding it becomes `$ROOT`. That rule is the whole reason
    /// `include` has its own extension: a fragment named `libs/tasks.chore`
    /// cannot be reached by that walk, so it only ever means what its includer
    /// makes it mean. Name the same fragment `libs/chorefile` and it acquires
    /// a second life — `cd libs && chore <anything>` stops at it, showing only
    /// its tasks with `$ROOT` at `libs/`, and every relative path inside it
    /// resolves against a different directory than it does through the
    /// include. Nothing fails; the same task simply writes to somewhere else.
    /// That is the shape worth a word, and the spec's own example (`include
    /// libs/chorefile as libs`) is how people arrive at it.
    ///
    /// **A warning, not an error, and no attempt to tell the two intents
    /// apart.** The shape is legal and sometimes exactly right: a subproject
    /// with its own lockfile *should* stand alone from its own directory, and
    /// `chore`'s own repo does this. The tempting refinement — probe the
    /// target's directory for a `package.json`, `Cargo.toml`, `go.mod` and
    /// friends and stay quiet when one is there — is not taken, for three
    /// reasons. It is wrong in both directions: a subproject whose only
    /// manifest *is* the chorefile would still be warned about, and a fragment
    /// parked in `vendor/thing/` next to somebody else's manifest would be
    /// silently excused. It would make the finding depend on the filesystem,
    /// so the same text checks differently on two machines — this module goes
    /// out of its way to avoid that (see the module docs). And it would be
    /// answering the wrong question anyway: when the target really is a
    /// subproject the warning is not a false positive, it is a true statement
    /// about a trade-off the author made deliberately, so the help says so
    /// and lets them keep it. Cheap to ignore, and never wrong.
    ///
    /// **Once per include**, unlike the `script` summary in
    /// [`Checker::unchecked`]: each one names a different file and each is
    /// fixed on its own, by renaming that file. Only the include that already
    /// draws a "more than once" error is skipped, so a doubled include is not
    /// told twice about one file.
    ///
    /// Confined to targets under `$ROOT`, which is what "discoverable instead
    /// of this file" means: `../other-project/chorefile` is outside the
    /// project, is another project's root by construction, and walking up from
    /// it was never going to land here.
    fn discoverable(&mut self, include: &Include, resolved: &Path, at: &Location) {
        if resolved.file_name() != Some(FILE_NAME.as_ref()) || !inside(&self.root, resolved) {
            return;
        }
        let shown = self.relative(resolved);
        let dir = resolved
            .parent()
            .map(|p| self.relative(p))
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| ".".to_string());
        let instead = format!("{dir}/tasks.{FILE_EXT}");

        self.push(
            Diagnostic::warning(
                format!(
                    "`include {}` points at `{shown}`, which `chore` can also discover on its own",
                    include.path
                ),
                at.clone(),
            )
            .with_help(format!(
                "discovery walks up from the working directory to the first file named exactly \
                 `{FILE_NAME}`, so `cd {dir} && chore list` stops at `{shown}` instead of this \
                 file: it shows only the tasks written there, with `$ROOT` at `{dir}/` rather \
                 than the project root, so a relative path inside it — say `download vendor/thing` \
                 — means a different place depending on which directory it was run from. If that \
                 file only means anything merged into this one, rename it to something ending in \
                 `.{FILE_EXT}` — `{instead}` — and point the include at that; discovery never \
                 finds those. If it is a standalone subproject, meant to work on its own from \
                 `{dir}/`, then the second view is deliberate and there is nothing to fix here"
            )),
        );
    }

    /// A path as a message should show it: relative to `$ROOT` when it is
    /// under it, so a finding reads `libs/chorefile` rather than an absolute
    /// path the reader has to scan to the end of.
    fn relative(&self, path: &Path) -> String {
        vars::display(path.strip_prefix(&self.root).unwrap_or(path))
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
                self.assignment(&a.name, a.span);
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
            // `return`'s code word is checked exactly as `exit`'s is; the
            // difference between them is where control goes, not what a
            // reader can write in the code.
            Stmt::Exit(code) | Stmt::Return(code) => {
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

    /// Every chain, wherever one appears.
    ///
    /// There is exactly one of these, and every walk that reaches something
    /// runnable goes through it: a statement, a `try`, an `if` condition,
    /// either side of a `&&`, `||` or `|`, and a `$( ... )` capture inside any
    /// word — a command's name, its arguments, a redirect target, an
    /// assignment's value, a parameter's default, a `for`'s items. That is what
    /// makes [`Script`] living in [`Chain`] rather than in [`Stmt`] cost this
    /// module one arm: a block gets the same treatment in every one of those
    /// positions, and there is no position a block can reach that does not come
    /// through here.
    fn chain(&mut self, chain: &Chain, scope: &Scope) {
        match chain {
            Chain::Single(cmd) => self.command(cmd, scope),
            Chain::Script(script) => self.script(script, scope),
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
        self.resolution(name, cmd.force_path, cmd.span, scope);
        if !cmd.force_path && name == "parallel" {
            self.parallel_tasks(&args, cmd.span);
        }
        // Only when every argument is literal: `literal` drops the ones that
        // are not, and a dropped word would shift the command out of place.
        if !cmd.force_path && name == "env" && args.len() == cmd.args.len() {
            self.env_prefixed(&args, cmd.span, scope);
        }
    }

    /// `env NAME=value <cmd> [args...]` runs a command, so `check` has to see
    /// one.
    ///
    /// Without this the whole line reads as arguments to a builtin, and
    /// `env CGO_ENABLED=0 cargooo build` — a typo in the command, in a form
    /// whose entire point is that a command follows — is reported by nothing.
    /// The words are already checked for undefined variables by the caller;
    /// what is left is the resolution every other command name gets, with the
    /// same `^` rule and the same platform guard.
    fn env_prefixed(&mut self, args: &[&str], span: Span, scope: &Scope) {
        // The interpreter's rule, and the reason it is one word: if the first
        // argument has an `=`, this is the per-command form. `env NAME value`
        // and `env NAME` set and read, and neither has a command in it.
        if !args.first().is_some_and(|first| first.contains('=')) {
            return;
        }
        // The leading bindings, then the command. A first word that has an `=`
        // but is not a binding is the interpreter's error to report, and a
        // form with no command at all is too: there is no command name here to
        // say anything about either way.
        if !is_binding(args[0]) {
            return;
        }
        let Some(first) = args.iter().find(|word| !is_binding(word)) else {
            return;
        };
        // Never `^`-forced: the parser takes a `^` only in front of a
        // statement's command name, so one cannot be written here at all.
        self.resolution(first, false, span, scope);
    }

    // -- script blocks ------------------------------------------------------

    /// `script <command...> { ... }`: the command is checked, the body is not.
    ///
    /// A block is a [`Chain`], so it can appear wherever anything else that
    /// runs can — a statement, a pipe, a `$( ... )` capture, an `if` condition,
    /// nested in any combination of those. Everything below holds in all of
    /// them and is written once, because [`Checker::chain`] is the only way to
    /// get here: the position a block was written in decides what happens to
    /// its *value*, and nothing about what `check` may say about it.
    ///
    /// **The body is another language.** Nothing in it is looked at — not for
    /// undefined variables, not for non-portable commands, not for anything
    /// else. A `$PATH` inside a Python string is not a chore variable and a
    /// `curl` inside a shell block is not a chore command, and a checker that
    /// guessed otherwise would produce confident nonsense about text it cannot
    /// parse. The rule is not "be careful with the body", it is "never read
    /// it": there is no span here that a finding may point into.
    ///
    /// The command is an ordinary command, and gets the ordinary treatment. Its
    /// words are expanded, so an undefined variable in the argv is a finding
    /// exactly as it is anywhere else, and the name resolves task → builtin →
    /// `PATH` with a miss reported as a warning that a platform guard can
    /// silence — `script pwsh -` under `if $OS == windows` says nothing on a
    /// Mac.
    ///
    /// Two things it does *not* do to the command:
    ///
    /// - No portability finding. [`builtins::REPLACEMENTS`] answers a
    ///   non-portable command with the builtin that replaces it, and no builtin
    ///   is an interpreter — none of them reads a program on stdin. Offering
    ///   `read` in place of `script cat -` would be advice that cannot be
    ///   taken. The one real portability trap here is the interpreter being a
    ///   host shell, which [`Checker::host_shell`] reports in its own terms.
    /// - No claim about the body reaching it. Whether the named command reads
    ///   stdin at all is knowable only by running it.
    fn script(&mut self, script: &Script, scope: &Scope) {
        self.scripts += 1;
        // The earliest block *in the file*, by position rather than by the
        // order this walk happens to reach it. The two agreed while a block was
        // a statement; a block in a capture is now reached while its enclosing
        // command is being walked, and comparing positions is true by
        // construction instead of by an argument about traversal order.
        let earliest = self
            .first_script
            .is_none_or(|first| script.span.start < first.start);
        if earliest {
            self.first_script = Some(script.span);
        }

        for word in &script.command {
            self.word(word, scope);
        }

        // A name built from a variable or a capture is only knowable at run
        // time, exactly as for any other command.
        let Some(first) = script.command.first().and_then(literal) else {
            return;
        };
        // `^` forces `PATH` wherever a command name is written; if the parser
        // hands it over still attached, read it the same way rather than
        // reporting a program called `^sh`.
        let (force_path, name) = match first.strip_prefix('^') {
            Some(rest) => (true, rest),
            None => (false, first),
        };
        if name.is_empty() {
            return;
        }
        self.host_shell(name, script);
        self.resolution(name, force_path, script.span, scope);
    }

    /// An interpreter that is the host's shell.
    ///
    /// `script uv run -` and `script nu --stdin` hand the block to a program
    /// that behaves the same wherever it is installed. `script sh -` and
    /// `script bash -` hand it to the host shell, which is the one thing
    /// `chore` exists to remove: `sh` is dash on one machine and bash on
    /// another, `cmd` and `powershell` exist only on Windows, and none of the
    /// POSIX ones exist there at all. A chorefile whose portable builtins are
    /// wrapped around one `script sh` block is as unportable as the shell
    /// script it replaced, and nothing else in `check` will say so — the body
    /// is unread, so the `rm -rf` and the `curl` inside it that would each be
    /// an error in a chorefile are invisible.
    ///
    /// A warning, per block, and never an error: a deliberate author writing a
    /// shell block behind `if $OS == windows` has done the honest thing, and
    /// this is a fact worth stating rather than a mistake. It is not silenced
    /// by a platform guard, because unlike a `PATH` miss it is not a fact about
    /// the machine running `check` — the guard is in the help text instead, as
    /// the shape a deliberate author is aiming for.
    fn host_shell(&mut self, name: &str, script: &Script) {
        let Some(shell) = shell_family(name) else {
            return;
        };
        let note = match shell {
            ShellFamily::Posix => format!(
                "`{name}` is a different program from platform to platform — dash here, bash \
                 there — and Windows has none of them"
            ),
            ShellFamily::Windows => format!("`{name}` runs on Windows and nowhere else"),
        };
        self.push(
            Diagnostic::warning(
                format!("`script {name}` hands the block to a host shell: {note}"),
                self.at(script.span),
            )
            .with_help(
                "a `script` block is for a language that behaves the same everywhere — `python3`, \
                 `uv run`, `node`, `nu`. If the shell is deliberate, guard it with `if $OS == ...` \
                 and give every platform a block; if it is only doing what a builtin does, write \
                 the builtin, which `check` and `--dry` can still see",
            ),
        );
    }

    /// The one thing `check` says about `script` blocks in general: that they
    /// exist, and that they are where its two guarantees stop.
    ///
    /// **Once per file, whatever the count.** A per-block warning was the
    /// obvious design and is the wrong one: a chorefile with ten legitimate
    /// blocks would emit ten permanent warnings that no edit can ever clear,
    /// and a warning nobody can act on is how a reader learns to skim past the
    /// ones that matter. Saying nothing is worse still — the whole point is
    /// that a reader should be *told* the guarantees have a hole rather than
    /// left to assume they hold everywhere, and reporting only when something
    /// else about the block is suspect would tell them exactly when the tool
    /// happened to notice something, which is not the same statement at all.
    ///
    /// So: one line, carrying the count, pointing at the first block. The count
    /// is the part a reader needs — it says how much of this file is outside
    /// the analysis — and the position gives them somewhere to start reading.
    /// A file with no `script` block gets nothing, so nobody pays for a feature
    /// they do not use.
    ///
    /// The count is every block in the file, not every block written as a
    /// statement: one inside a `$( ... )` capture or on the far side of a pipe
    /// is exactly as unread as any other, and a summary that quietly excluded
    /// them would understate how much of the file is outside the analysis.
    ///
    /// Per *file* rather than per tree, so an included chorefile reports its
    /// own blocks against its own path, like every other finding here.
    fn unchecked(&mut self) {
        let (Some(span), n) = (self.first_script, self.scripts) else {
            return;
        };
        let message = if n == 1 {
            "this file has a `script` block: `check` reads none of its body, and `--dry` skips it \
             whole"
                .to_string()
        } else {
            format!(
                "this file has {n} `script` blocks: `check` reads none of their bodies, and \
                 `--dry` skips them whole"
            )
        };
        self.push(Diagnostic::warning(message, self.at(span)).with_help(
            "nothing to fix — this is the escape hatch working as intended. It is said once \
                 per file, at the first block, because both guarantees stop at the opening brace: \
                 an undefined variable, a non-portable command or a missing program inside is not \
                 reported, and `--dry` skips the block rather than running it",
        ));
    }

    /// `parallel`'s arguments are task names, not paths, so a typo in one is
    /// a mistake `check` can see: the run would otherwise get as far as the
    /// call before saying so. A name built from a variable is skipped, like
    /// every other name only knowable at run time.
    fn parallel_tasks(&mut self, args: &[&str], span: Span) {
        for arg in args {
            if arg.starts_with("--") || self.is_task(arg) {
                continue;
            }
            if let Some((ns, task)) = arg.split_once(NAMESPACE_SEP) {
                // The same reading a call gets: an unfollowed include's
                // namespace is taken on trust rather than called wrong.
                self.namespaced(arg, ns, task, span);
                continue;
            }
            let mut d = Diagnostic::error(
                format!("`parallel {arg}`: `{arg}` is not a task"),
                self.at(span),
            );
            d = match self.suggestion(arg) {
                Some(similar) => d.with_help(format!("did you mean `{similar}`?")),
                None => d.with_help(
                    "`parallel` takes the names of tasks in this file, not commands".to_string(),
                ),
            };
            self.push(d);
        }
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
        // A task or a builtin of the same name already wins over `PATH`, so
        // only a `^`-forced call actually reaches the non-portable program.
        // Tasks matter as much as builtins here: `test` is among the most
        // common task names there is, and calling it from an aggregate task
        // is the most common thing to do with it.
        if !cmd.force_path && (self.is_task(name) || builtins::is_builtin(name)) {
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
    fn resolution(&mut self, name: &str, force_path: bool, span: Span, scope: &Scope) {
        if !force_path {
            if name == "cd" || self.is_task(name) || builtins::is_builtin(name) {
                return;
            }
            if let Some((ns, task)) = name.split_once(NAMESPACE_SEP) {
                if self.namespaced(name, ns, task, span) {
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

        let called = if force_path {
            format!("^{name}")
        } else {
            name.to_string()
        };
        let mut help = format!(
            "`check` looked on this machine's `PATH`, which is not necessarily the machine that \
             runs the task — if `{name}` is installed only in CI or in a container, this is fine"
        );
        if !force_path {
            if let Some(similar) = self.suggestion(name) {
                help = format!("did you mean `{similar}`? Otherwise: {help}");
            }
        }
        self.push(
            Diagnostic::warning(
                format!("`{called}` is not a task, not a builtin, and was not found on `PATH`"),
                self.at(span),
            )
            .with_help(help),
        );
    }

    /// A call to `ns::task`, which no task answers to.
    ///
    /// Returns whether it was dealt with. Before includes were followed there
    /// was nothing honest to say — the task was in a file `check` had not
    /// read — so a known namespace was taken on trust. Once they are followed
    /// the merged table is complete, and a name it does not hold is wrong.
    ///
    /// Never a `PATH` fallback: `::` is not a character a program on `PATH` is
    /// spelled with, so "not found on `PATH`" would be a misleading way to
    /// report a name that was only ever going to be a task.
    fn namespaced(&mut self, name: &str, ns: &str, task: &str, span: Span) -> bool {
        let known = self.names.namespaces.contains(ns)
            || self
                .names
                .tasks
                .iter()
                .any(|t| t.starts_with(&format!("{ns}{NAMESPACE_SEP}")));
        if !self.merged {
            // Includes were not followed. A namespace this file declares is
            // taken on trust; one it does not falls through to the ordinary
            // unknown-command path.
            return known;
        }
        let d = if known {
            let mut d = Diagnostic::error(
                format!("`{name}` is not a task: namespace `{ns}` has no task `{task}`"),
                self.at(span),
            );
            if let Some(similar) = self.in_namespace(ns, task) {
                d = d.with_help(format!("did you mean `{similar}`?"));
            }
            d
        } else {
            let d = Diagnostic::error(
                format!("`{name}` names the namespace `{ns}`, which no `include` defines"),
                self.at(span),
            );
            // The namespace itself is the likeliest typo here, and
            // `suggestion` will only offer one whose task half already
            // matches exactly.
            match self.suggestion(name) {
                Some(similar) => d.with_help(format!("did you mean `{similar}`?")),
                None => d.with_help(format!(
                    "add `as {ns}` to the include that should provide it, or drop the \
                     `{ns}{NAMESPACE_SEP}` prefix if the task is in this file"
                )),
            }
        };
        self.push(d);
        true
    }

    /// The nearest task or builtin name, when the difference looks like a typo.
    ///
    /// Namespace-aware, because a merged tree has several tasks called
    /// `build`. A typo in the task half is answered from that namespace and
    /// nowhere else: suggesting `tools::build` for `libs::buidl` names the
    /// right word and the wrong project, which is worse than saying nothing.
    /// A typo in the namespace half is answered only by a candidate whose task
    /// half already matches exactly, so `lib::build` still finds
    /// `libs::build`. A bare call is answered only by bare candidates, since a
    /// namespaced task is not something the author mistyped their way into.
    fn suggestion(&self, name: &str) -> Option<String> {
        let Some((ns, task)) = name.split_once(NAMESPACE_SEP) else {
            let flat = self
                .candidate_tasks()
                .filter(|c| !c.contains(NAMESPACE_SEP))
                .chain(builtins::NAMES.iter().copied());
            return nearest(name, flat);
        };
        if let Some(similar) = self.in_namespace(ns, task) {
            return Some(similar);
        }
        // The task half is right and the namespace half is not.
        let suffix = format!("{NAMESPACE_SEP}{task}");
        nearest(
            name,
            self.candidate_tasks().filter(|c| c.ends_with(&suffix)),
        )
    }

    /// The nearest task to `task` inside namespace `ns`, fully qualified.
    fn in_namespace(&self, ns: &str, task: &str) -> Option<String> {
        let prefix = format!("{ns}{NAMESPACE_SEP}");
        let tails: Vec<&str> = self
            .candidate_tasks()
            .filter_map(|c| c.strip_prefix(&prefix))
            .collect();
        nearest(task, tails.into_iter()).map(|t| format!("{ns}{NAMESPACE_SEP}{t}"))
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
                let defined =
                    vars::BUILTIN_NAMES.contains(&name.as_str()) || scope.names.contains(name);
                // A parameter's name is not a variable: parameters are bound
                // by position, so `$target` in a task declaring `target` reads
                // whatever `target` means outside the task — or nothing.
                let param = scope
                    .task
                    .and_then(|t| t.params.iter().position(|p| p.name == *name));
                if let Some(i) = param {
                    let task = scope.task.expect("a parameter implies a task");
                    let d = if defined {
                        Diagnostic::warning(
                            format!(
                                "`${name}` reads the global, not the parameter `{name}` of task \
                                 `{}`",
                                task.name
                            ),
                            at,
                        )
                        .with_help(format!(
                            "parameters are read by position: write `${}` for `{name}`, or rename \
                             the parameter if the global is what was meant",
                            i + 1
                        ))
                    } else {
                        Diagnostic::error(format!("undefined variable `${name}`"), at).with_help(
                            format!(
                                "`{name}` is parameter {} of task `{}`, and parameters are read \
                                 by position — write `${}`",
                                i + 1,
                                task.name,
                                i + 1
                            ),
                        )
                    };
                    self.push(d);
                    return;
                }
                if defined {
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
                Some(_) if *n <= scope.params_bound && *n > 0 => {}
                // Inside a default, and naming a parameter that exists but is
                // not bound yet. The header is not what is wrong here.
                Some(task) if *n > 0 && *n <= task.params.len() => {
                    let this = &task.params[scope.params_bound];
                    self.push(
                        Diagnostic::error(
                            format!(
                                "`${n}` is not bound yet: it is the default for `{}`, parameter \
                                 {} of task `{}`",
                                this.name,
                                scope.params_bound + 1,
                                task.name
                            ),
                            at,
                        )
                        .with_help(if scope.params_bound == 0 {
                            format!(
                                "a default is evaluated as the call is bound, so `{}` — the first \
                                 parameter — can only read globals and the builtin variables",
                                this.name
                            )
                        } else {
                            format!(
                                "a default may read the parameters declared before it, `$1` \
                                 through `${}`, and nothing after",
                                scope.params_bound
                            )
                        }),
                    );
                }
                Some(task) => {
                    self.push(
                        Diagnostic::error(
                            format!(
                                "`${n}` is never set: task `{}` declares {}",
                                task.name,
                                declared_params(task)
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

/// A `NAME=value` word in `env`'s per-command form, read the way the
/// interpreter's `binding` reads it: an identifier, an `=`, and anything.
fn is_binding(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| crate::lex::is_ident(name))
}

/// Which kind of host shell an interpreter is, if it is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellFamily {
    /// `sh` and its relatives: present on Unix, absent on Windows, and not the
    /// same program twice.
    Posix,
    /// Windows' own: absent everywhere else.
    Windows,
}

/// Is this interpreter a host shell?
///
/// The list is deliberately the shells a machine *hands you* — the ones whose
/// behavior is a property of the host rather than of the chorefile. A
/// cross-platform interpreter that happens to be a shell, `nu` above all, is
/// not here: it is installed on purpose and behaves the same wherever it is,
/// which is the whole difference this finding is about. `fish` is left out for
/// the same reason, and because being wrong here costs a warning on a correct
/// file.
///
/// Matched on the file name, so `/bin/sh` and `C:/Windows/System32/cmd.exe`
/// are recognised as readily as `sh` and `cmd`.
fn shell_family(name: &str) -> Option<ShellFamily> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    let base = base.to_ascii_lowercase();
    match base.as_str() {
        "sh" | "bash" | "zsh" | "dash" | "ash" | "ksh" | "csh" | "tcsh" => Some(ShellFamily::Posix),
        "cmd" | "command" | "powershell" => Some(ShellFamily::Windows),
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

/// What a task's header gives a caller, in the terms the caller cares about:
/// how many arguments they *have* to pass, and which names are answered for
/// them by a default. `2 parameter(s): 1 required (src), 1 optional (dest)`.
fn declared_params(task: &Task) -> String {
    if task.params.is_empty() {
        return "no parameters".to_string();
    }
    let mut parts = Vec::new();
    for (label, group) in [
        ("required", split_params(task, true)),
        ("optional", split_params(task, false)),
    ] {
        if !group.is_empty() {
            parts.push(format!("{} {label} ({})", group.len(), group.join(", ")));
        }
    }
    format!("{} parameter(s): {}", task.params.len(), parts.join(", "))
}

fn split_params(task: &Task, required: bool) -> Vec<&str> {
    task.params
        .iter()
        .filter(|p| p.required() == required)
        .map(|p| p.name.as_str())
        .collect()
}

/// A plausible `task` header for a task that reads `$n`.
///
/// The parameters it already has are written as they were declared — an
/// optional one keeps its `=`, since the fix is to add a parameter, never to
/// drop somebody's default.
fn header_params(task: &Task, n: usize) -> String {
    let mut params: Vec<String> = task.params.iter().map(param_header).collect();
    while params.len() < n {
        params.push(format!("arg{}", params.len() + 1));
    }
    params.join(" ")
}

/// One parameter as a header writes it: `name`, `name=value` when the default
/// is plain text, `name=...` when it is something only a run can produce.
fn param_header(param: &Param) -> String {
    match &param.default {
        None => param.name.clone(),
        Some(word) => match literal(word) {
            Some(text) if !text.is_empty() => format!("{}={text}", param.name),
            _ => format!("{}=...", param.name),
        },
    }
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

/// Is `path` under `root`, both already normalized?
///
/// An empty `root` is the working directory — what a bare `chorefile` with no
/// directory in front of it has — and there is no prefix to match against, so
/// the test becomes the one that survives normalization: a relative path that
/// never climbed out.
fn inside(root: &Path, path: &Path) -> bool {
    if root.as_os_str().is_empty() {
        !path.is_absolute() && !path.starts_with("..")
    } else {
        path.starts_with(root)
    }
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
