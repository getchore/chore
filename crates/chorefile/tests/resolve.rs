//! `include` resolution, against real files on disk.
//!
//! Every test here writes a real tree in a temp directory rather than feeding
//! the resolver a synthetic AST: the whole feature is about paths — relative
//! to the including file, a directory standing for its `chorefile`, two
//! spellings of one file being one file — and none of that is exercised by a
//! tree that never touched a filesystem.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chorefile::ast::{self, Chain, Cond, PartKind, Stmt, VarRef, Word};
use chorefile::error::Error;
use chorefile::resolve::{self, Merged};
use chorefile::vars;

// ---------------------------------------------------------------------------
// harness — same shape as the e2e tests: a temp dir that removes itself
// ---------------------------------------------------------------------------

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "chore-resolve-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(&path, text).expect("write");
        path
    }

    fn chorefile(&self, text: &str) -> PathBuf {
        self.write("chorefile", text)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn root(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn resolve(path: &Path) -> Merged {
    resolve::resolve(path).unwrap_or_else(|e| panic!("resolve {}: {e}", path.display()))
}

/// The message of a failure that was expected to be one.
fn error(path: &Path) -> String {
    match resolve::resolve(path) {
        Ok(_) => panic!("expected {} to fail", path.display()),
        Err(e) => e.to_string(),
    }
}

fn task_names(merged: &Merged) -> Vec<&str> {
    merged.file.tasks.iter().map(|t| t.name.as_str()).collect()
}

fn global_names(merged: &Merged) -> Vec<&str> {
    merged
        .file
        .globals
        .iter()
        .map(|g| g.name.as_str())
        .collect()
}

fn task<'a>(merged: &'a Merged, name: &str) -> &'a ast::Task {
    merged
        .file
        .tasks
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no task `{name}` in {:?}", task_names(merged)))
}

/// Every command name in a body, in source order, as written.
fn calls(block: &ast::Block) -> Vec<String> {
    fn chain(node: &Chain, out: &mut Vec<String>) {
        match node {
            Chain::Single(cmd) => {
                out.push(text(&cmd.name));
                for arg in &cmd.args {
                    words(arg, out);
                }
            }
            Chain::And(a, b) | Chain::Or(a, b) | Chain::Pipe(a, b) => {
                chain(a, out);
                chain(b, out);
            }
        }
    }
    /// Captures inside an argument are calls too.
    fn words(word: &Word, out: &mut Vec<String>) {
        for part in &word.parts {
            if let PartKind::Capture(inner) = &part.kind {
                chain(inner, out);
            }
        }
    }
    fn cond(node: &Cond, out: &mut Vec<String>) {
        match node {
            Cond::Compare { left, right, .. } => {
                words(left, out);
                words(right, out);
            }
            Cond::Command(c) => chain(c, out),
            Cond::Not(inner) => cond(inner, out),
            Cond::And(a, b) | Cond::Or(a, b) => {
                cond(a, out);
                cond(b, out);
            }
        }
    }
    let mut out = Vec::new();
    for stmt in block {
        match stmt {
            Stmt::Command(c) | Stmt::Try(c) => chain(c, &mut out),
            Stmt::Assign(a) => words(&a.value, &mut out),
            Stmt::If(node) => {
                cond(&node.cond, &mut out);
                out.extend(calls(&node.then));
                if let Some(otherwise) = &node.otherwise {
                    out.extend(calls(otherwise));
                }
            }
            Stmt::For(node) => {
                for item in &node.items {
                    words(item, &mut out);
                }
                out.extend(calls(&node.body));
            }
            Stmt::Exit(_) => {}
        }
    }
    out
}

/// Every `$name` a body reads, in source order.
fn reads(block: &ast::Block) -> Vec<String> {
    fn word(word: &Word, out: &mut Vec<String>) {
        for part in &word.parts {
            match &part.kind {
                PartKind::Var(VarRef::Named(name)) => out.push(name.clone()),
                PartKind::Capture(chain) => chain_words(chain, out),
                _ => {}
            }
        }
    }
    fn chain_words(chain: &Chain, out: &mut Vec<String>) {
        match chain {
            Chain::Single(cmd) => {
                word(&cmd.name, out);
                for arg in &cmd.args {
                    word(arg, out);
                }
                for redirect in &cmd.redirects {
                    word(&redirect.target, out);
                }
            }
            Chain::And(a, b) | Chain::Or(a, b) | Chain::Pipe(a, b) => {
                chain_words(a, out);
                chain_words(b, out);
            }
        }
    }
    fn cond(node: &Cond, out: &mut Vec<String>) {
        match node {
            Cond::Compare { left, right, .. } => {
                word(left, out);
                word(right, out);
            }
            Cond::Command(c) => chain_words(c, out),
            Cond::Not(inner) => cond(inner, out),
            Cond::And(a, b) | Cond::Or(a, b) => {
                cond(a, out);
                cond(b, out);
            }
        }
    }
    let mut out = Vec::new();
    for stmt in block {
        match stmt {
            Stmt::Command(c) | Stmt::Try(c) => chain_words(c, &mut out),
            Stmt::Assign(a) => word(&a.value, &mut out),
            Stmt::If(node) => {
                cond(&node.cond, &mut out);
                out.extend(reads(&node.then));
                if let Some(otherwise) = &node.otherwise {
                    out.extend(reads(otherwise));
                }
            }
            Stmt::For(node) => {
                for item in &node.items {
                    word(item, &mut out);
                }
                out.extend(reads(&node.body));
            }
            Stmt::Exit(code) => {
                if let Some(w) = code {
                    word(w, &mut out);
                }
            }
        }
    }
    out
}

/// A word's literal text, for the command names the tests compare against.
fn text(word: &Word) -> String {
    word.parts
        .iter()
        .map(|part| match &part.kind {
            PartKind::Literal(s) => s.clone(),
            PartKind::Var(VarRef::Named(name)) => format!("${name}"),
            _ => "?".into(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// the flat merge
// ---------------------------------------------------------------------------

#[test]
fn a_flat_include_merges_tasks_and_globals() {
    let dir = Dir::new();
    dir.write("tasks.chore", "dist=dist\ntask build {\n  echo build\n}\n");
    let top = dir.chorefile("include tasks.chore\nversion=1\ntask ship {\n  echo ship\n}\n");

    let merged = resolve(&top);
    assert_eq!(task_names(&merged), ["build", "ship"]);
    assert_eq!(global_names(&merged), ["dist", "version"]);
    assert!(merged.file.includes.is_empty(), "includes were followed");
}

#[test]
fn root_is_the_top_level_files_directory() {
    let dir = Dir::new();
    dir.write("nested/lib.chore", "task a {\n  echo a\n}\n");
    let top = dir.chorefile("include nested/lib.chore\n");

    // Not the included file's directory, and not the working directory.
    assert_eq!(resolve(&top).root, dir.root());
}

#[test]
fn included_globals_are_evaluated_before_the_including_files() {
    let dir = Dir::new();
    dir.write("a.chore", "one=1\n");
    dir.write("b.chore", "two=2\n");
    let top = dir.chorefile("include a.chore\nthree=3\ninclude b.chore\nfour=4\n");

    // Depth-first, in source order, with the including file's own last: an
    // includer can read what it included, never the other way round.
    assert_eq!(
        global_names(&resolve(&top)),
        ["one", "two", "three", "four"]
    );
}

// ---------------------------------------------------------------------------
// `as`
// ---------------------------------------------------------------------------

#[test]
fn as_namespaces_tasks_and_globals() {
    let dir = Dir::new();
    dir.write("libs.chore", "dist=dist\ntask build {\n  echo $dist\n}\n");
    let top = dir.chorefile("include libs.chore as libs\ndist=other\n");

    let merged = resolve(&top);
    assert_eq!(task_names(&merged), ["libs::build"]);
    assert_eq!(global_names(&merged), ["libs::dist", "dist"]);
}

#[test]
fn a_namespaced_file_still_reaches_its_own_names() {
    let dir = Dir::new();
    dir.write(
        "libs.chore",
        "dist=dist\n\
         task build {\n  echo $dist\n}\n\
         task all {\n  build\n  ^build\n  echo $dist > $dist/log\n}\n",
    );
    let top = dir.chorefile("include libs.chore as libs\n");

    let merged = resolve(&top);
    let all = task(&merged, "libs::all");
    // The sibling call follows the rename; `^build` is forced to PATH and so
    // never named the task at all.
    assert_eq!(calls(&all.body), ["libs::build", "build", "echo"]);
    // The global follows it too, in the argument and in the redirect target.
    assert_eq!(reads(&all.body), ["libs::dist", "libs::dist"]);
    assert_eq!(reads(&task(&merged, "libs::build").body), ["libs::dist"]);
}

#[test]
fn a_namespace_leaves_foreign_names_alone() {
    let dir = Dir::new();
    dir.write(
        "libs.chore",
        "task build {\n  echo $OS\n  mkdir dist\n  helper\n}\n",
    );
    let top = dir.chorefile("include libs.chore as libs\ntask helper {\n  echo helper\n}\n");

    let merged = resolve(&top);
    let build = task(&merged, "libs::build");
    // `$OS` is a builtin, `mkdir` a builtin command, and `helper` is defined
    // by the *including* file — none of them is the namespace's to rename.
    assert_eq!(reads(&build.body), ["OS"]);
    assert_eq!(calls(&build.body), ["echo", "mkdir", "helper"]);
}

#[test]
fn a_local_assignment_shadows_a_namespaced_global() {
    let dir = Dir::new();
    dir.write(
        "libs.chore",
        "dist=dist\ntask build {\n  dist=local\n  echo $dist\n}\n",
    );
    let top = dir.chorefile("include libs.chore as libs\n");

    // The interpreter makes an assignment inside a task local, so the read
    // below never reached the global and must not be renamed to it.
    let merged = resolve(&top);
    assert_eq!(reads(&task(&merged, "libs::build").body), ["dist"]);
}

#[test]
fn a_loop_variable_shadows_a_namespaced_global() {
    let dir = Dir::new();
    dir.write(
        "libs.chore",
        "f=global\ntask each {\n  for f in a b {\n    echo $f\n  }\n}\n",
    );
    let top = dir.chorefile("include libs.chore as libs\n");

    let merged = resolve(&top);
    assert_eq!(reads(&task(&merged, "libs::each").body), ["f"]);
}

#[test]
fn a_global_reading_an_earlier_global_follows_the_namespace() {
    let dir = Dir::new();
    dir.write("libs.chore", "dist=dist\nbin=$dist/bin\n");
    let top = dir.chorefile("include libs.chore as libs\n");

    let merged = resolve(&top);
    let bin = &merged.file.globals[1];
    assert_eq!(bin.name, "libs::bin");
    assert_eq!(text(&bin.value), "$libs::dist/bin");
}

#[test]
fn namespaces_nest() {
    let dir = Dir::new();
    dir.write("inner.chore", "task build {\n  echo inner\n}\n");
    dir.write(
        "outer.chore",
        "include inner.chore as inner\ntask all {\n  inner::build\n}\n",
    );
    let top = dir.chorefile("include outer.chore as outer\n");

    let merged = resolve(&top);
    assert_eq!(task_names(&merged), ["outer::inner::build", "outer::all"]);
    // The call written inside `outer` already carried one level; the second
    // prefix is applied to the name as it stands, not to the bare one.
    assert_eq!(
        calls(&task(&merged, "outer::all").body),
        ["outer::inner::build"]
    );
}

// ---------------------------------------------------------------------------
// paths
// ---------------------------------------------------------------------------

#[test]
fn a_nested_include_resolves_against_its_own_file() {
    let dir = Dir::new();
    dir.write("libs/lib.chore", "include helpers/util.chore\n");
    dir.write("libs/helpers/util.chore", "task util {\n  echo util\n}\n");
    let top = dir.chorefile("include libs/lib.chore\n");

    // `helpers/util.chore` is relative to `libs/`, not to the top-level file
    // and not to the working directory.
    assert_eq!(task_names(&resolve(&top)), ["util"]);
}

#[test]
fn a_directory_means_the_chorefile_inside_it() {
    let dir = Dir::new();
    dir.write("libs/chorefile", "task build {\n  echo libs\n}\n");
    let top = dir.chorefile("include libs as libs\n");

    assert_eq!(task_names(&resolve(&top)), ["libs::build"]);
}

#[test]
fn an_extensionless_path_is_taken_literally() {
    let dir = Dir::new();
    dir.write("libs", "task build {\n  echo libs\n}\n");
    dir.write("libs.chore", "task other {\n  echo other\n}\n");
    let top = dir.chorefile("include libs\n");

    // No `.chore` is guessed on: `include libs` names the file `libs`, even
    // with a `libs.chore` sitting beside it.
    assert_eq!(task_names(&resolve(&top)), ["build"]);
}

#[test]
fn a_missing_include_names_the_path_as_written() {
    let dir = Dir::new();
    let top = dir.chorefile("include libs/missing.chore\n");

    let message = error(&top);
    assert!(message.contains("libs/missing.chore"), "{message}");
    assert!(message.contains("cannot read"), "{message}");
    // Reported at the `include` line of the file that wrote it.
    assert!(message.contains("chorefile:"), "{message}");
}

// ---------------------------------------------------------------------------
// cycles
// ---------------------------------------------------------------------------

#[test]
fn a_file_including_itself_is_an_error() {
    let dir = Dir::new();
    let top = dir.chorefile("include ./chorefile\ntask a {\n  echo a\n}\n");

    // `./chorefile` and `chorefile` are the same file, so the spelling does
    // not get past the check.
    let message = error(&top);
    assert!(message.contains("include cycle"), "{message}");
}

#[test]
fn an_indirect_cycle_names_the_loop() {
    let dir = Dir::new();
    dir.write("a.chore", "include b.chore\n");
    dir.write("b.chore", "include a.chore\n");
    let top = dir.chorefile("include a.chore\n");

    let message = error(&top);
    assert!(message.contains("include cycle"), "{message}");
    // Paths are shown relative to the top-level chorefile's directory, and
    // the loop is shown whole rather than only its last hop.
    assert!(
        message.contains("a.chore -> b.chore -> a.chore"),
        "{message}"
    );
}

#[test]
fn a_cycle_through_a_directory_include_is_still_a_cycle() {
    let dir = Dir::new();
    dir.write("libs/chorefile", "include ../chorefile\n");
    let top = dir.chorefile("include libs\n");

    assert!(error(&top).contains("include cycle"));
}

// ---------------------------------------------------------------------------
// duplicates
// ---------------------------------------------------------------------------

#[test]
fn a_duplicate_task_in_a_flat_merge_is_an_error() {
    let dir = Dir::new();
    dir.write("libs.chore", "task build {\n  echo lib\n}\n");
    let top = dir.chorefile("include libs.chore\ntask build {\n  echo top\n}\n");

    let message = error(&top);
    assert!(message.contains("duplicate task `build`"), "{message}");
    // Names the file the earlier definition came from...
    assert!(message.contains("libs.chore"), "{message}");
    // ...and points at the later one, which is in the top-level file.
    assert!(message.starts_with(&format!("{}:", vars::display(&dir.path("chorefile")))));
    assert!(message.contains("as <namespace>"), "{message}");
}

#[test]
fn a_duplicate_within_one_file_is_left_to_check() {
    let dir = Dir::new();
    dir.write(
        "libs.chore",
        "task build {\n  echo one\n}\ntask build {\n  echo two\n}\n",
    );
    let top = dir.chorefile("include libs.chore\n");

    // `check` reports this one, naming the line of the earlier definition —
    // more use than anything the merge could say, and failing here would be
    // the only thing the user ever saw.
    let merged = resolve(&top);
    assert_eq!(task_names(&merged), ["build", "build"]);
}

#[test]
fn a_within_file_duplicate_still_collides_across_files() {
    let dir = Dir::new();
    dir.write(
        "a.chore",
        "task build {\n  echo one\n}\ntask build {\n  echo two\n}\n",
    );
    dir.write("b.chore", "task build {\n  echo b\n}\n");
    let top = dir.chorefile("include a.chore\ninclude b.chore\n");

    // And the file named is the first one to define it, not the repeat.
    let message = error(&top);
    assert!(message.contains("duplicate task `build`"), "{message}");
    assert!(message.contains("`a.chore`"), "{message}");
    assert!(message.starts_with(&format!("{}:", vars::display(&dir.path("b.chore")))));
}

#[test]
fn a_duplicate_global_in_a_flat_merge_is_an_error() {
    let dir = Dir::new();
    dir.write("libs.chore", "dist=lib\n");
    let top = dir.chorefile("include libs.chore\ndist=top\n");

    let message = error(&top);
    assert!(message.contains("duplicate global `dist`"), "{message}");
    assert!(message.contains("libs.chore"), "{message}");
}

#[test]
fn two_flat_includes_that_collide_are_an_error() {
    let dir = Dir::new();
    dir.write("a.chore", "task build {\n  echo a\n}\n");
    dir.write("b.chore", "task build {\n  echo b\n}\n");
    let top = dir.chorefile("include a.chore\ninclude b.chore\n");

    let message = error(&top);
    assert!(message.contains("duplicate task `build`"), "{message}");
    assert!(message.contains("a.chore"), "{message}");
    // The later definition is the one in `b.chore`.
    assert!(message.starts_with(&format!("{}:", vars::display(&dir.path("b.chore")))));
}

#[test]
fn a_namespace_makes_a_collision_legal() {
    let dir = Dir::new();
    dir.write("a.chore", "dist=a\ntask build {\n  echo a\n}\n");
    dir.write("b.chore", "dist=b\ntask build {\n  echo b\n}\n");
    let top = dir.chorefile("include a.chore as a\ninclude b.chore as b\n");

    let merged = resolve(&top);
    assert_eq!(task_names(&merged), ["a::build", "b::build"]);
    assert_eq!(global_names(&merged), ["a::dist", "b::dist"]);
}

// ---------------------------------------------------------------------------
// sources
// ---------------------------------------------------------------------------

#[test]
fn sources_hold_every_file_that_contributed() {
    let dir = Dir::new();
    dir.write("libs/lib.chore", "include util.chore\n");
    dir.write("libs/util.chore", "task util {\n  echo util\n}\n");
    let top = dir.chorefile("include libs/lib.chore\n");

    let merged = resolve(&top);
    let mut files: Vec<_> = merged.sources.files().map(Path::to_path_buf).collect();
    files.sort();
    let mut expected = vec![
        dir.path("chorefile"),
        dir.path("libs/lib.chore"),
        dir.path("libs/util.chore"),
    ];
    expected.sort();
    assert_eq!(files, expected);
    assert!(
        merged
            .sources
            .get(&dir.path("libs/util.chore"))
            .unwrap()
            .contains("echo util")
    );
}

#[test]
fn sources_key_matches_the_location_a_diagnostic_carries() {
    let dir = Dir::new();
    // A syntax error two lines into an included file: the location it reports
    // must key straight into `sources`, or `path:line:col` cannot be printed.
    dir.write("libs.chore", "task ok {\n  echo ok\n}\ntask {\n}\n");
    let top = dir.chorefile("include libs.chore\n");

    let Err(Error::Syntax { at, .. }) = resolve::resolve(&top) else {
        panic!("expected a syntax error from the included file");
    };
    assert_eq!(at.file, dir.path("libs.chore"));

    // The failing parse aborts the whole resolve, so the text has to be read
    // back the same way a caller would render it.
    let text = std::fs::read_to_string(dir.path("libs.chore")).unwrap();
    let rendered = at.render(&text);
    assert!(
        rendered.starts_with(&vars::display(&dir.path("libs.chore"))),
        "{rendered}"
    );
    assert!(rendered.contains(":4:"), "{rendered}");
}

#[test]
fn a_diagnostic_in_an_included_file_renders_through_sources() {
    let dir = Dir::new();
    dir.write("libs.chore", "task build {\n  echo lib\n}\n");
    let top = dir.chorefile("include libs.chore\n");

    let merged = resolve(&top);
    let build = task(&merged, "build");
    // The span belongs to `libs.chore`, and rendering it needs *that* file's
    // text — the top-level file is one line long.
    let at = chorefile::error::Location::new(dir.path("libs.chore"), build.span);
    assert_eq!(
        merged.sources.render(&at),
        format!("{}:1:1", vars::display(&dir.path("libs.chore")))
    );
}

// ---------------------------------------------------------------------------
// parts
// ---------------------------------------------------------------------------

#[test]
fn parts_hold_each_file_as_written() {
    let dir = Dir::new();
    dir.write("libs.chore", "dist=dist\ntask build {\n  echo $dist\n}\n");
    let top = dir.chorefile("include libs.chore as libs\ntask ship {\n  libs::build\n}\n");

    let merged = resolve(&top);
    let parts: Vec<_> = merged
        .parts
        .iter()
        .map(|p| (p.path.clone(), p.prefix.clone()))
        .collect();
    assert_eq!(
        parts,
        [
            (dir.path("chorefile"), None),
            (dir.path("libs.chore"), Some("libs".to_string())),
        ]
    );

    // Un-namespaced: the names are the ones the spans point at.
    let libs = &merged.parts[1].file;
    assert_eq!(libs.tasks[0].name, "build");
    assert_eq!(libs.globals[0].name, "dist");
    assert_eq!(reads(&libs.tasks[0].body), ["dist"]);
    // ...while the merged tree spells them the way the run will see them.
    assert_eq!(task_names(&merged), ["libs::build", "ship"]);
}

#[test]
fn a_parts_prefix_is_the_whole_chain_of_namespaces() {
    let dir = Dir::new();
    dir.write("inner.chore", "task build {\n  echo inner\n}\n");
    dir.write("mid.chore", "include inner.chore as inner\n");
    dir.write("flat.chore", "include plain.chore\n");
    dir.write("plain.chore", "task plain {\n  echo plain\n}\n");
    let top = dir.chorefile("include mid.chore as outer\ninclude flat.chore\n");

    let merged = resolve(&top);
    let prefix = |name: &str| {
        merged
            .parts
            .iter()
            .find(|p| p.path == dir.path(name))
            .unwrap_or_else(|| panic!("no part for {name}"))
            .prefix
            .clone()
    };
    assert_eq!(prefix("chorefile"), None);
    assert_eq!(prefix("mid.chore"), Some("outer".into()));
    // Every `as` on the way down, in order — the name the merged tree uses.
    assert_eq!(prefix("inner.chore"), Some("outer::inner".into()));
    // A flat include inherits its includer's prefix, which here is none.
    assert_eq!(prefix("flat.chore"), None);
    assert_eq!(prefix("plain.chore"), None);
    assert_eq!(task_names(&merged), ["outer::inner::build", "plain"]);
}

#[test]
fn a_parts_prefix_carries_through_a_flat_include() {
    let dir = Dir::new();
    dir.write("lib.chore", "include helper.chore\n");
    dir.write("helper.chore", "task help {\n  echo help\n}\n");
    let top = dir.chorefile("include lib.chore as libs\n");

    let merged = resolve(&top);
    // `helper.chore` was included flat, but into a file that was namespaced,
    // so its names are namespaced too — and its part says so.
    let helper = merged
        .parts
        .iter()
        .find(|p| p.path == dir.path("helper.chore"))
        .expect("helper part");
    assert_eq!(helper.prefix.as_deref(), Some("libs"));
    assert_eq!(task_names(&merged), ["libs::help"]);
}

#[test]
fn parts_keep_the_include_directives_the_merged_tree_drops() {
    let dir = Dir::new();
    dir.write("libs.chore", "task build {\n  echo build\n}\n");
    let top = dir.chorefile("include libs.chore as libs\n");

    let merged = resolve(&top);
    assert!(merged.file.includes.is_empty());
    assert_eq!(merged.parts[0].file.includes.len(), 1);
    assert_eq!(merged.parts[0].file.includes[0].path, "libs.chore");
}

#[test]
fn every_part_path_keys_into_sources() {
    let dir = Dir::new();
    dir.write("libs/lib.chore", "include util.chore\n");
    dir.write("libs/util.chore", "task util {\n  echo util\n}\n");
    let top = dir.chorefile("include libs/lib.chore\n");

    let merged = resolve(&top);
    assert_eq!(merged.parts.len(), 3);
    for part in &merged.parts {
        assert!(
            merged.sources.get(&part.path).is_some(),
            "no source for {}",
            part.path.display()
        );
    }
}
