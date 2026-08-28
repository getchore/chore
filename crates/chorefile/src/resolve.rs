//! Following `include`, and merging what it pulls in.
//!
//! Parsing produces one [`ast::File`] per source file, with its `include`
//! directives recorded but not followed. This module follows them: it reads
//! each included file, parses it, and merges the result into one tree the
//! interpreter can run without knowing that more than one file existed.
//!
//! The rules are in `docs/SPEC.md` and are worth restating, because each one
//! is load-bearing:
//!
//! - An include path resolves against the file **doing the including**, not
//!   the working directory, so a nested include of a sibling keeps working
//!   wherever `chore` is invoked from.
//! - A path naming a directory means the `chorefile` inside it.
//! - `$ROOT` stays the top-level chorefile's directory in every included
//!   file. One root per invocation, or a `download ... third_party/` in an
//!   included file would land somewhere its author did not choose.
//! - `as name` namespaces the file's tasks *and* its globals as `name::task`.
//!   Without it everything merges flat, and any duplicate — task or global —
//!   is an error rather than a silent last-one-wins.
//! - A cycle is an error, not an infinite loop.
//!
//! # What the spec leaves to this module
//!
//! **A namespace renames definitions, so it must also rename the references
//! that reached them.** An included file is written without knowing what it
//! will be called: its tasks call each other by bare name and read its globals
//! by bare name, and those names stop existing the moment `as libs` renames
//! the definitions to `libs::`. So prefixing a file rewrites, inside that
//! file's own bodies, every reference that *resolved within it*:
//!
//! - a command whose name is one literal word naming a task of the subtree
//!   becomes `libs::task`;
//! - `$x` naming a global of the subtree becomes `$libs::x`.
//!
//! Everything else is left alone, which is what lets an included file still
//! reach a builtin, a program on `PATH`, or — under a flat merge — a name the
//! including file defines. The rule composes through nesting: a file included
//! `as inner` and then, with its includer, `as outer` ends up with
//! `outer::inner::task`, because each prefix is applied to names that are
//! already resolved one level down.
//!
//! Two things are deliberately outside the rewrite. A command name that is
//! *computed* — `$cmd`, `"$prefix-build"` — is not a literal, so a namespaced
//! file that dispatches to its own tasks through a variable has to spell the
//! namespace itself; there is nothing in the tree to rewrite. And a name a
//! task assigns anywhere in its body (or binds with `for`) is treated as
//! local for the whole body, so it is never rewritten to a global — matching
//! the interpreter, where an assignment inside a task writes a local. The
//! cost is that a task which reads a namespaced global *and* later assigns
//! the same name reads its own local instead; the alternative, flow-sensitive
//! analysis, would make the meaning of `$x` depend on a line further down.
//!
//! **Globals from an include are evaluated before the including file's.** The
//! spec fixes only that top-level assignments run once before the first task.
//! Includes are ordered depth-first, in source order, with each file's own
//! assignments last. That direction is the only one that composes: the
//! including file knows what it included and can build on it
//! (`toolchain=$libs::default-toolchain`), while an included file cannot name
//! its includer and so has nothing to gain from running later. Shadowing is
//! not the reason for the order and cannot happen either way — a flat merge
//! makes a duplicate global an error, and `as` gives it a different name — so
//! the order only ever decides what a global can *read*.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::ast;
use crate::error::{Error, Location, Result, Span};
use crate::{FILE_NAME, NAMESPACE_SEP, parse, vars};

/// One chorefile and everything it included, merged into a single tree.
///
/// Two views of the same run, and the split matters: [`file`](Self::file) is
/// the authority on **names** — one flat tree, every name spelled the way the
/// run will see it — while [`parts`](Self::parts) is the authority on
/// **positions** and per-file structure. A span in the merged tree no longer
/// says which file it came from, so anything that has to point at source
/// works from `parts`, and anything that has to resolve a name works from
/// `file`.
pub struct Merged {
    /// The merged tree: every task and global, namespaced where `as` asked.
    ///
    /// Its `includes` are empty: they have been followed, and leaving them in
    /// place would invite a second traversal of the same files.
    pub file: ast::File,
    /// Each contributing file's own parse, in the order they were loaded.
    pub parts: Vec<Part>,
    /// The directory of the top-level chorefile — `$ROOT` for the whole run.
    pub root: PathBuf,
    /// Every file that contributed, with its text.
    ///
    /// A diagnostic's [`Location`](crate::error::Location) names one of these
    /// files, and rendering it as `path:line:col` needs *that* file's text,
    /// not the top-level one's.
    pub sources: Sources,
}

/// One file as it was written, before merging.
///
/// The prefix travels with the parse because it is not recoverable from
/// either end afterwards: the merged tree has forgotten which file a name
/// came from, and the file has forgotten what it was included as. Re-deriving
/// it by walking the include graph a second time is a second implementation
/// of the namespacing rule, free to disagree with this one — so the walk that
/// applied the prefix reports it instead.
pub struct Part {
    /// The file, spelled the same way its [`Location`](crate::error::Location)s
    /// and its [`Sources`] key are.
    pub path: PathBuf,
    /// This file alone: its own tasks, globals and `include` directives, with
    /// spans into its own text and names exactly as written — un-namespaced,
    /// because that is what its spans describe.
    pub file: ast::File,
    /// What `as` made of this file's names: `None` for a flat include and for
    /// the top-level file, `Some("outer::inner")` where namespaces nested.
    /// A name in `file` appears in [`Merged::file`] as `prefix::name`.
    pub prefix: Option<String>,
}

/// The text of every file that contributed to a merged chorefile.
#[derive(Debug, Default)]
pub struct Sources(HashMap<PathBuf, String>);

impl Sources {
    pub fn insert(&mut self, path: impl Into<PathBuf>, text: impl Into<String>) {
        self.0.insert(path.into(), text.into());
    }

    pub fn get(&self, path: &Path) -> Option<&str> {
        self.0.get(path).map(String::as_str)
    }

    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.0.keys().map(PathBuf::as_path)
    }

    /// `path:line:col` for a diagnostic, using the text of the file it points
    /// into. Falls back to the path alone when that file is not known, so a
    /// diagnostic is never lost to a bookkeeping miss.
    pub fn render(&self, at: &crate::error::Location) -> String {
        match self.get(&at.file) {
            Some(text) => at.render(text),
            None => at.file.display().to_string(),
        }
    }
}

/// Read the chorefile at `path`, follow its includes, and merge them.
pub fn resolve(path: &Path) -> Result<Merged> {
    let path = normalize(path);
    let root = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    let mut resolver = Resolver {
        root: root.clone(),
        ..Resolver::default()
    };
    let unit = resolver.load(&path, key(&path), None, None)?;

    Ok(Merged {
        file: ast::File {
            // A `require` belongs to the file that wrote it, and the merged
            // tree has no file to be: every requirement is checked through
            // `parts`, where each one still knows what to name. See
            // [`require::unmet`](crate::require::unmet).
            require: None,
            includes: Vec::new(),
            globals: unit.globals.into_iter().map(|d| d.item).collect(),
            tasks: unit.tasks.into_iter().map(|d| d.item).collect(),
        },
        parts: resolver.parts,
        root,
        sources: resolver.sources,
    })
}

// ---------------------------------------------------------------------------
// The merge
// ---------------------------------------------------------------------------

/// A definition, and the file it was written in.
///
/// The file travels with the definition because a duplicate is only
/// actionable when the message can say where the *other* one is, and by the
/// time two files have been merged the tree no longer remembers.
struct Def<T> {
    file: PathBuf,
    item: T,
}

/// One file and everything it included, merged and namespaced.
///
/// Held apart from [`ast::File`] because a merge needs the name tables to
/// catch duplicates, and the interpreter never does.
#[derive(Default)]
struct Unit {
    globals: Vec<Def<ast::Assign>>,
    tasks: Vec<Def<ast::Task>>,
    global_from: HashMap<String, PathBuf>,
    task_from: HashMap<String, PathBuf>,
}

impl Unit {
    /// Fold `other` in, rejecting any name two *files* both define.
    ///
    /// The diagnostic points at the later definition — the one being merged
    /// in — since that is the one the author is most likely to be editing,
    /// and names the file the earlier one came from.
    ///
    /// A file that defines one name twice is left alone: `check` reports that
    /// with the line of the earlier definition, which is more use than
    /// anything this walk can say, and failing here first would be the only
    /// thing the user ever saw. So only the collision that needs a merge to
    /// be visible at all is a merge error.
    fn absorb(&mut self, other: Unit, root: &Path) -> Result<()> {
        for global in other.globals {
            let name = global.item.name.clone();
            if let Some(first) = self.global_from.get(&name).filter(|f| **f != global.file) {
                return Err(duplicate(
                    "global",
                    &name,
                    first,
                    &global.file,
                    global.item.span,
                    root,
                ));
            }
            // First one wins the table: a second definition in the same file
            // is `check`'s to report, and the earlier file is the one a
            // later cross-file collision should name.
            self.global_from.entry(name).or_insert(global.file.clone());
            self.globals.push(global);
        }
        for task in other.tasks {
            let name = task.item.name.clone();
            if let Some(first) = self.task_from.get(&name).filter(|f| **f != task.file) {
                return Err(duplicate(
                    "task",
                    &name,
                    first,
                    &task.file,
                    task.item.span,
                    root,
                ));
            }
            self.task_from.entry(name).or_insert(task.file.clone());
            self.tasks.push(task);
        }
        Ok(())
    }

    /// Rename every definition to `ns::name`, and every reference that
    /// reached one of them along with it. See the module docs.
    fn namespace(&mut self, ns: &str) {
        let renamer = Renamer {
            ns,
            tasks: self.tasks.iter().map(|d| d.item.name.clone()).collect(),
            globals: self.globals.iter().map(|d| d.item.name.clone()).collect(),
        };

        for global in &mut self.globals {
            // A global's value may read an earlier global; nothing is local
            // at the top level, so there is nothing to shadow it.
            renamer.word(&mut global.item.value, &HashSet::new());
            global.item.name = renamer.qualify(&global.item.name);
        }
        for task in &mut self.tasks {
            let locals = locals_of(&task.item.body);
            renamer.block(&mut task.item.body, &locals);
            task.item.name = renamer.qualify(&task.item.name);
        }

        self.global_from = std::mem::take(&mut self.global_from)
            .into_iter()
            .map(|(name, file)| (renamer.qualify(&name), file))
            .collect();
        self.task_from = std::mem::take(&mut self.task_from)
            .into_iter()
            .map(|(name, file)| (renamer.qualify(&name), file))
            .collect();
    }
}

/// The traversal: one entry per file currently being loaded, so a cycle is a
/// path that is already on the stack rather than a stack that runs out.
#[derive(Default)]
struct Resolver {
    /// The top-level chorefile's directory: not `$ROOT` here, only the base a
    /// message shortens its paths against.
    root: PathBuf,
    sources: Sources,
    parts: Vec<Part>,
    /// `(identity, path as parsed)` for every file on the current chain.
    stack: Vec<(PathBuf, PathBuf)>,
}

/// The `include` that asked for a file: where to report a failure, and how
/// the author spelled the path.
struct Origin<'a> {
    at: Location,
    wrote: &'a str,
}

impl Resolver {
    /// Load one file and everything it includes.
    ///
    /// `at` is the `include` that asked for this file, and is where a failure
    /// to read it is reported; the top-level file has none.
    fn load(
        &mut self,
        path: &Path,
        id: PathBuf,
        from: Option<&Origin>,
        prefix: Option<&str>,
    ) -> Result<Unit> {
        let at = from.map(|origin| &origin.at);
        if let Some(start) = self.stack.iter().position(|(seen, _)| *seen == id) {
            return Err(self.cycle(start, path, at));
        }

        let text = std::fs::read_to_string(path).map_err(|e| Error::Syntax {
            // Named the way the author wrote it, with the path that spelling
            // worked out to — the two differ, and only one of them is
            // greppable in the file the reader is about to open.
            message: match from {
                Some(origin) => format!(
                    "cannot read `{}`: {e} (looked for `{}`)",
                    origin.wrote,
                    relative(path, &self.root)
                ),
                None => format!("cannot read `{}`: {e}", vars::display(path)),
            },
            // A missing include is reported at the `include` line; a missing
            // top-level chorefile has nowhere else to point but itself.
            at: at
                .cloned()
                .unwrap_or_else(|| Location::new(path, Span::new(0, 0))),
        })?;
        let file = parse::parse(&text, path)?;
        // Parsed twice on purpose: `parts` keeps the file as written, spans
        // and all, while the merge renames and consumes its copy. Sharing one
        // tree would mean handing out the renamed one, whose names no longer
        // match the text those spans point into.
        self.parts.push(Part {
            path: path.to_path_buf(),
            file: parse::parse(&text, path)?,
            prefix: prefix.map(str::to_owned),
        });
        self.sources.insert(path, text);

        self.stack.push((id, path.to_path_buf()));
        let merged = self.merge(file, path, prefix);
        self.stack.pop();
        merged
    }

    /// Includes first, in source order, then the file's own definitions — the
    /// evaluation order the module docs argue for.
    fn merge(&mut self, file: ast::File, path: &Path, prefix: Option<&str>) -> Result<Unit> {
        let mut unit = Unit::default();

        for include in &file.includes {
            let origin = Origin {
                at: Location::new(path, include.span),
                wrote: &include.path,
            };
            let target = include_path(path, &include.path);
            // The prefix a file ends up under is the chain of every `as` on
            // the way down to it, which is known here, on the way in; the
            // rename itself happens on the way out, once the child's own
            // includes have folded in and there is one set of names to apply
            // it to.
            let nested = match (prefix, include.namespace.as_deref()) {
                (Some(outer), Some(ns)) => Some(format!("{outer}{NAMESPACE_SEP}{ns}")),
                (Some(outer), None) => Some(outer.to_owned()),
                (None, ns) => ns.map(str::to_owned),
            };
            let mut child = self.load(&target, key(&target), Some(&origin), nested.as_deref())?;
            if let Some(ns) = &include.namespace {
                child.namespace(ns);
            }
            unit.absorb(child, &self.root)?;
        }

        unit.absorb(
            Unit {
                globals: file
                    .globals
                    .into_iter()
                    .map(|item| Def {
                        file: path.to_path_buf(),
                        item,
                    })
                    .collect(),
                tasks: file
                    .tasks
                    .into_iter()
                    .map(|item| Def {
                        file: path.to_path_buf(),
                        item,
                    })
                    .collect(),
                global_from: HashMap::new(),
                task_from: HashMap::new(),
            },
            &self.root,
        )?;
        Ok(unit)
    }

    /// The chain from the file that is being re-entered back around to it, so
    /// the message shows the loop and not just its last hop.
    fn cycle(&self, start: usize, repeat: &Path, at: Option<&Location>) -> Error {
        let mut chain: Vec<String> = self.stack[start..]
            .iter()
            .map(|(_, path)| relative(path, &self.root))
            .collect();
        chain.push(relative(repeat, &self.root));
        Error::Syntax {
            message: format!("include cycle: {}", chain.join(" -> ")),
            at: at
                .cloned()
                .unwrap_or_else(|| Location::new(repeat, Span::new(0, 0))),
        }
    }
}

/// A name two files both define, merged flat.
fn duplicate(
    kind: &str,
    name: &str,
    first: &Path,
    second: &Path,
    span: Span,
    root: &Path,
) -> Error {
    Error::Syntax {
        message: format!(
            "duplicate {kind} `{name}`: `{name}` is already defined in `{}`; a flat `include` \
             merges every name, so rename one or include with `as <namespace>`",
            relative(first, root)
        ),
        at: Location::new(second, span),
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Where an `include` points: relative to the including file, with a
/// directory meaning the `chorefile` inside it.
///
/// A path with no extension that is not a directory is taken literally — no
/// `.chore` is guessed onto it. Guessing would make `include libs` name a
/// different file depending on what happens to exist on disk, and a chorefile
/// that resolves differently on two machines is worse than one that fails to
/// resolve at all. The error says what was looked for.
fn include_path(from: &Path, path: &str) -> PathBuf {
    let base = from.parent().unwrap_or_else(|| Path::new("."));
    let joined = base.join(vars::to_native(path));
    let joined = if joined.is_dir() {
        joined.join(FILE_NAME)
    } else {
        joined
    };
    normalize(&joined)
}

/// A path as a message should show it: relative to the top-level chorefile's
/// directory when it is under it, so a cycle reads `libs/a.chore -> ...`
/// rather than twice the width of the terminal in temp-directory prefixes.
fn relative(path: &Path, root: &Path) -> String {
    vars::display(path.strip_prefix(root).unwrap_or(path))
}

/// The identity of a file, for cycle detection: two spellings of one path
/// must compare equal, symlinks and `..` included.
///
/// Falls back to the lexical form when the file does not exist —
/// `canonicalize` needs it to — leaving the read to report the miss.
fn key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize(path))
}

/// Collapse `.` and `foo/..` so two spellings of one path compare equal.
/// Purely lexical: the file may not exist yet, and `canonicalize` would fail.
fn normalize(path: &Path) -> PathBuf {
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
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

// ---------------------------------------------------------------------------
// Namespacing references
// ---------------------------------------------------------------------------

/// Rewrites the references inside a subtree that reached its own definitions,
/// to follow those definitions under their new `ns::` names.
struct Renamer<'a> {
    ns: &'a str,
    tasks: HashSet<String>,
    globals: HashSet<String>,
}

impl Renamer<'_> {
    fn qualify(&self, name: &str) -> String {
        format!("{}{NAMESPACE_SEP}{name}", self.ns)
    }

    fn block(&self, block: &mut ast::Block, locals: &HashSet<String>) {
        for stmt in block {
            match stmt {
                ast::Stmt::Assign(assign) => self.word(&mut assign.value, locals),
                ast::Stmt::Command(chain) | ast::Stmt::Try(chain) => self.chain(chain, locals),
                ast::Stmt::If(node) => {
                    self.cond(&mut node.cond, locals);
                    self.block(&mut node.then, locals);
                    if let Some(otherwise) = &mut node.otherwise {
                        self.block(otherwise, locals);
                    }
                }
                ast::Stmt::For(node) => {
                    for item in &mut node.items {
                        self.word(item, locals);
                    }
                    self.block(&mut node.body, locals);
                }
                // `exit` and `return` differ in where they stop, not in what
                // they name, so a code written as `$status` is renamed the
                // same way in both.
                ast::Stmt::Exit(code) | ast::Stmt::Return(code) => {
                    if let Some(word) = code {
                        self.word(word, locals);
                    }
                }
            }
        }
    }

    fn cond(&self, cond: &mut ast::Cond, locals: &HashSet<String>) {
        match cond {
            ast::Cond::Compare { left, right, .. } => {
                self.word(left, locals);
                self.word(right, locals);
            }
            ast::Cond::Command(chain) => self.chain(chain, locals),
            ast::Cond::Not(inner) => self.cond(inner, locals),
            ast::Cond::And(a, b) | ast::Cond::Or(a, b) => {
                self.cond(a, locals);
                self.cond(b, locals);
            }
        }
    }

    fn chain(&self, chain: &mut ast::Chain, locals: &HashSet<String>) {
        match chain {
            ast::Chain::Single(cmd) => self.command(cmd, locals),
            // The interpreter's argv is namespaced like any other command's,
            // so `script $tool -` inside an included file still resolves. The
            // body is another language and is left exactly as written.
            ast::Chain::Script(script) => {
                for word in &mut script.command {
                    self.word(word, locals);
                }
            }
            ast::Chain::And(a, b) | ast::Chain::Or(a, b) | ast::Chain::Pipe(a, b) => {
                self.chain(a, locals);
                self.chain(b, locals);
            }
        }
    }

    fn command(&self, cmd: &mut ast::Command, locals: &HashSet<String>) {
        // `^name` is forced to PATH and never named a task, so renaming it
        // would point it at a namespace that has no bearing on it.
        let call = literal(&cmd.name).filter(|name| self.tasks.contains(*name));
        if !cmd.force_path {
            if let Some(qualified) = call.map(|name| self.qualify(name)) {
                if let Some(part) = cmd.name.parts.first_mut() {
                    part.kind = ast::PartKind::Literal(qualified);
                }
            }
        }
        self.word(&mut cmd.name, locals);
        for arg in &mut cmd.args {
            self.word(arg, locals);
        }
        for redirect in &mut cmd.redirects {
            self.word(&mut redirect.target, locals);
        }
    }

    fn word(&self, word: &mut ast::Word, locals: &HashSet<String>) {
        for part in &mut word.parts {
            match &mut part.kind {
                ast::PartKind::Literal(_) => {}
                ast::PartKind::Var(ast::VarRef::Named(name)) => {
                    if self.globals.contains(name.as_str()) && !locals.contains(name.as_str()) {
                        *name = self.qualify(name);
                    }
                }
                ast::PartKind::Var(_) => {}
                ast::PartKind::Capture(chain) => self.chain(chain, locals),
            }
        }
    }
}

/// The whole of a word, when it is one plain literal — the only form of
/// command name that can be matched against a task name before the run.
fn literal(word: &ast::Word) -> Option<&str> {
    match word.parts.as_slice() {
        [
            ast::WordPart {
                kind: ast::PartKind::Literal(text),
                ..
            },
        ] => Some(text),
        _ => None,
    }
}

/// Every name a task body binds: an assignment anywhere in it, or a `for`
/// variable. Body-wide rather than flow-sensitive, so that `$x` means the
/// same thing on every line of a task.
fn locals_of(block: &ast::Block) -> HashSet<String> {
    fn walk(block: &ast::Block, out: &mut HashSet<String>) {
        for stmt in block {
            match stmt {
                ast::Stmt::Assign(assign) => {
                    out.insert(assign.name.clone());
                }
                ast::Stmt::If(node) => {
                    walk(&node.then, out);
                    if let Some(otherwise) = &node.otherwise {
                        walk(otherwise, out);
                    }
                }
                ast::Stmt::For(node) => {
                    out.insert(node.var.clone());
                    walk(&node.body, out);
                }
                ast::Stmt::Command(_)
                | ast::Stmt::Try(_)
                | ast::Stmt::Exit(_)
                | ast::Stmt::Return(_) => {}
            }
        }
    }
    let mut out = HashSet::new();
    walk(block, &mut out);
    out
}
