//! `check` over real chorefile snippets.
//!
//! Every case parses with `parse::parse` first, so a change to the grammar
//! that breaks these snippets breaks these tests too.

use std::path::{Path, PathBuf};

use chorefile::ast;
use chorefile::check::{self, Diagnostic, Severity};
use chorefile::error::Span;
use chorefile::parse;
use chorefile::vars;

fn file_path() -> PathBuf {
    PathBuf::from("chorefile")
}

fn run(source: &str) -> Vec<Diagnostic> {
    check::check_source(source, &file_path())
}

fn messages(source: &str) -> Vec<String> {
    run(source).into_iter().map(|d| d.message).collect()
}

/// Every message that mentions `needle`, so a test asserts on the finding it
/// cares about and not on the rest of the file.
fn matching(source: &str, needle: &str) -> Vec<Diagnostic> {
    run(source)
        .into_iter()
        .filter(|d| d.message.contains(needle))
        .collect()
}

fn errors(source: &str) -> Vec<Diagnostic> {
    run(source)
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect()
}

// --- 1. syntax errors ------------------------------------------------------

#[test]
fn syntax_error_becomes_a_diagnostic() {
    let found = run("task build {\n    echo hi\n");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert_eq!(found[0].at.file, Path::new("chorefile"));
}

#[test]
fn syntax_error_points_into_the_file() {
    let source = "task build {\n    echo hi\n";
    let found = run(source);
    let (line, _) = found[0].at.line_col(source);
    assert!(line >= 1);
}

// --- 2. reserved names -----------------------------------------------------

#[test]
fn task_named_after_a_subcommand() {
    for name in ["list", "help", "check", "spec"] {
        let found = errors(&format!("task {name} {{\n    echo hi\n}}\n"));
        assert!(
            found.iter().any(|d| d.message.contains("subcommand")),
            "{name}: {found:#?}"
        );
    }
}

#[test]
fn task_named_after_a_builtin() {
    let found = matching("task download url {\n    echo $1\n}\n", "shadows");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("`download`"));
    assert!(found[0].help.as_ref().unwrap().contains("reserved"));
}

/// The task wins at runtime, so the builtin is what is lost. Saying the task
/// "would never run" here would be exactly backwards.
#[test]
fn a_shadowed_builtin_is_the_thing_that_becomes_unreachable() {
    let found = matching("task write {\n    echo hi\n}\n", "shadows");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].message.contains("runs the task instead"),
        "{found:#?}"
    );
    assert!(!found[0].message.contains("never run"), "{found:#?}");
}

/// A subcommand name is the other way round: the task really is dead code.
#[test]
fn a_reserved_subcommand_reads_as_dead_code_not_as_shadowing() {
    let found = matching("task list {\n    echo hi\n}\n", "subcommand");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("can never run"), "{found:#?}");
    assert!(!found[0].message.contains("shadows"), "{found:#?}");
}

#[test]
fn task_named_cd() {
    assert!(!matching("task cd {\n    echo hi\n}\n", "interpreter itself").is_empty());
}

#[test]
fn namespace_separator_in_a_task_name() {
    assert!(!matching("task libs::build {\n    echo hi\n}\n", "contains `::`").is_empty());
}

// --- 3. duplicates ---------------------------------------------------------

#[test]
fn duplicate_tasks_and_globals() {
    let source = "\
version=1
version=2

task build {
    echo one
}

task build {
    echo two
}
";
    let found = matching(source, "duplicate");
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found[0].message.contains("global `version`"));
    assert!(found[1].message.contains("task `build`"));
    // The help points back at the definition that is being shadowed.
    assert!(found[0].help.as_ref().unwrap().contains("line 1"));
}

#[test]
fn findings_are_sorted_by_position() {
    let source = "\
task build {
    echo $late
}

task rm {
    echo $early
}
";
    let found = run(source);
    let lines: Vec<usize> = found.iter().map(|d| d.at.line_col(source).0).collect();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "{found:#?}");
}

// --- 4. unknown commands ---------------------------------------------------

#[test]
fn unknown_command_is_a_warning_not_an_error() {
    let found = matching(
        "task build {\n    definitely-not-a-real-program --x\n}\n",
        "not found on `PATH`",
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Warning);
    // The message has to say why a miss is not fatal.
    assert!(found[0].help.as_ref().unwrap().contains("CI"));
}

#[test]
fn tasks_and_builtins_resolve_before_path() {
    let source = "\
task build {
    helper
    echo done
}

task helper {
    mkdir dist
}
";
    assert_eq!(messages(source), Vec::<String>::new());
}

#[test]
fn forced_path_skips_tasks_and_builtins() {
    // `^echo` is a PATH lookup even though `echo` is a builtin.
    let found = matching(
        "task t {\n    ^definitely-not-real\n}\n",
        "^definitely-not-real",
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test]
fn a_typo_suggests_the_real_name() {
    let found = matching(
        "task t {\n    donwload url dest\n}\n",
        "not found on `PATH`",
    );
    assert!(
        found[0].help.as_ref().unwrap().contains("`download`"),
        "{found:#?}"
    );
}

#[test]
fn a_dynamic_command_name_is_not_guessed_at() {
    let source = "\
tool=cargo

task build {
    $tool build
}
";
    assert_eq!(messages(source), Vec::<String>::new());
}

// --- 5. non-portable commands ----------------------------------------------

#[test]
fn non_portable_commands_name_their_replacement() {
    let cases = [
        ("curl -L $url -o out", "download"),
        ("unzip out.zip dist", "extract"),
        ("tar xzf out.tgz", "extract"),
        ("cp a b", "copy"),
        ("rm -rf dist", "remove"),
        ("cat file", "read"),
    ];
    for (line, replacement) in cases {
        let source = format!("url=x\n\ntask t {{\n    {line}\n}}\n");
        let found = matching(&source, "not portable");
        assert_eq!(found.len(), 1, "{line}: {found:#?}");
        assert_eq!(found[0].severity, Severity::Error);
        let help = found[0].help.as_ref().unwrap();
        assert!(help.contains(replacement), "{line}: {help}");
    }
}

#[test]
fn a_builtin_of_the_same_name_is_not_flagged() {
    // `mkdir -p` is in REPLACEMENTS, but `mkdir` resolves to the builtin.
    assert!(matching("task t {\n    mkdir -p dist\n}\n", "not portable").is_empty());
    // Forcing PATH is what actually reaches the non-portable program.
    let found = matching("task t {\n    ^mkdir -p dist\n}\n", "not portable");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].help.as_ref().unwrap().contains("-p"));
}

// --- 6. undefined variables ------------------------------------------------

#[test]
fn undefined_named_variable() {
    let found = matching("task t {\n    echo $nope\n}\n", "undefined variable");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("$nope"));
}

#[test]
fn globals_builtins_locals_and_loop_variables_are_defined() {
    let source = "\
dist=target/release

task build target {
    out=$dist/$1
    echo $out $OS $PLATFORM
    for f in a b c {
        echo $f $out
    }
}
";
    assert_eq!(messages(source), Vec::<String>::new());
}

#[test]
fn a_loop_variable_does_not_escape_its_loop() {
    let source = "\
task t {
    for f in a b {
        echo $f
    }
    echo $f
}
";
    let found = matching(source, "undefined variable");
    assert_eq!(found.len(), 1, "{found:#?}");
}

#[test]
fn a_variable_used_before_it_is_assigned() {
    let source = "\
task t {
    echo $later
    later=1
}
";
    assert_eq!(matching(source, "undefined variable").len(), 1);
}

#[test]
fn a_global_sees_only_the_globals_above_it() {
    assert_eq!(matching("a=$b\nb=2\n", "undefined variable").len(), 1);
    assert_eq!(matching("b=2\na=$b\n", "undefined variable").len(), 0);
}

#[test]
fn a_near_miss_suggests_the_variable_that_exists() {
    let found = matching("target=x\n\ntask t {\n    echo $targe\n}\n", "undefined");
    assert!(
        found[0].help.as_ref().unwrap().contains("$target"),
        "{found:#?}"
    );
}

#[test]
fn positional_beyond_the_declared_parameters() {
    let source = "\
task t one {
    echo $1 $2
}
";
    let found = matching(source, "$2");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("1 parameter(s) (one)"));
    assert!(found[0].help.as_ref().unwrap().contains("task t one arg2"));
}

#[test]
fn arguments_at_the_top_level() {
    let found = messages("a=$1\nb=$@\nc=$#\n");
    assert_eq!(found.len(), 3, "{found:#?}");
    assert!(
        found
            .iter()
            .all(|m| m.contains("only defined inside a task"))
    );
}

#[test]
fn arguments_inside_a_task_are_fine() {
    assert_eq!(
        messages("task t a b {\n    echo $1 $2 $@ $#\n}\n"),
        Vec::<String>::new()
    );
}

#[test]
fn variables_inside_conditions_captures_and_redirects() {
    let source = "\
task t {
    if $missing_a == \"\" {
        echo $(echo $missing_b) > $missing_c
    }
}
";
    let found = matching(source, "undefined variable");
    assert_eq!(found.len(), 3, "{found:#?}");
}

#[test]
fn each_interpolation_gets_its_own_position() {
    let source = "task t {\n    echo \"$a/$b\"\n}\n";
    let found = matching(source, "undefined variable");
    assert_eq!(found.len(), 2, "{found:#?}");

    // Both `$`s are in one word; each finding points at its own.
    let places: Vec<(usize, usize)> = found.iter().map(|d| d.at.line_col(source)).collect();
    assert_eq!(places[0].0, places[1].0, "same line");
    assert_ne!(places[0].1, places[1].1, "{places:?}");

    assert!(found[0].message.contains("$a"));
    assert!(found[1].message.contains("$b"));
    assert_eq!(&source[found[0].at.span.range()], "$a");
    assert_eq!(&source[found[1].at.span.range()], "$b");
}

#[test]
fn a_positional_finding_points_at_the_parameter() {
    let source = "task t {\n    echo \"v$2\"\n}\n";
    let found = matching(source, "$2");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(&source[found[0].at.span.range()], "$2");
}

// --- 7. control flow -------------------------------------------------------

#[test]
fn a_condition_with_a_forgotten_dollar_is_constant() {
    let source = "task t {\n    if OS == windows {\n        echo hi\n    }\n}\n";
    let found = matching(source, "always");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("always false"));
    // The header, not the body.
    assert_eq!(&source[found[0].at.span.range()], "if OS == windows");
}

#[test]
fn a_condition_that_can_vary_is_not_reported() {
    let source = "task t {\n    if $OS == windows {\n        echo hi\n    }\n}\n";
    assert!(matching(source, "always").is_empty());
    let source = "task t {\n    if exists Makefile {\n        echo hi\n    }\n}\n";
    assert!(matching(source, "always").is_empty());
}

#[test]
fn a_loop_over_nothing_never_runs() {
    let source = "task t {\n    for f in {\n        echo $f\n    }\n}\n";
    let found = matching(source, "never runs");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(&source[found[0].at.span.range()], "for f in");
}

// --- 7a. platform guards ---------------------------------------------------
//
// A `PATH` miss inside a branch this host provably never enters says nothing
// about the chorefile, so it is not reported. Everything here is written
// against `vars::OS` rather than a hard-coded name, so the same assertions
// hold whichever platform runs the tests.

/// An `$OS` value that is not this machine's.
fn other_os() -> &'static str {
    if vars::OS == "windows" {
        "linux"
    } else {
        "windows"
    }
}

/// A command name no machine has on `PATH`.
const MISSING: &str = "definitely-not-a-real-program";

fn path_misses(source: &str) -> Vec<Diagnostic> {
    matching(source, "not found on `PATH`")
}

#[test]
fn a_command_guarded_off_this_platform_is_not_reported() {
    // The case this exists for: MinGW tools under `$OS == windows && $ENV ==
    // gnu`, linted on a machine that is not Windows.
    let source = format!(
        "\
task build {{
    if $OS == {} && $ENV == gnu {{
        gendef-not-real libfoo.dll
        dlltool-not-real -d libfoo.def
    }}
}}
",
        other_os()
    );
    assert!(path_misses(&source).is_empty(), "{:#?}", run(&source));
}

#[test]
fn a_guard_that_does_not_exclude_this_host_still_reports() {
    // Same shape, but the guard names this machine, so the command really is
    // expected here.
    let source = format!(
        "task t {{\n    if $OS == {} {{\n        {MISSING}\n    }}\n}}\n",
        vars::OS
    );
    let found = path_misses(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test]
fn a_guard_on_something_unknowable_still_reports() {
    // `exists` is decided at run time, and a global could hold anything: with
    // no proof the branch is skipped here, the finding stays.
    for guard in [
        "exists Makefile".to_string(),
        "$flavor == gnu".to_string(),
        format!("$OS == {} || exists Makefile", other_os()),
    ] {
        let source =
            format!("flavor=gnu\n\ntask t {{\n    if {guard} {{\n        {MISSING}\n    }}\n}}\n");
        let found = path_misses(&source);
        assert_eq!(found.len(), 1, "{guard}: {found:#?}");
    }
}

#[test]
fn a_shadowed_platform_variable_is_not_trusted() {
    // `OS=` binds a name of the chorefile's own over the read-only one, so
    // `check` cannot say what `$OS` holds and must keep the finding.
    let source = format!(
        "OS=weird\n\ntask t {{\n    if $OS == {} {{\n        {MISSING}\n    }}\n}}\n",
        other_os()
    );
    assert_eq!(path_misses(&source).len(), 1, "{:#?}", run(&source));
}

#[test]
fn the_else_branch_of_a_platform_guard_is_still_checked() {
    // `else` is exactly the branch that does run here.
    let source = format!(
        "\
task t {{
    if $OS == {} {{
        gendef-not-real
    }} else {{
        {MISSING}
    }}
}}
",
        other_os()
    );
    let found = path_misses(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains(MISSING), "{found:#?}");
}

#[test]
fn the_else_branch_of_a_guard_that_always_holds_is_skipped() {
    // The mirror image: `if $OS == <this host>` means the `else` never runs.
    let source = format!(
        "task t {{\n    if $OS == {} {{\n        echo hi\n    }} else {{\n        {MISSING}\n    }}\n}}\n",
        vars::OS
    );
    assert!(path_misses(&source).is_empty(), "{:#?}", run(&source));
}

#[test]
fn a_negated_platform_guard_is_understood() {
    // `!=` and `!( ... )` against this host both exclude the branch.
    for guard in [
        format!("$OS != {}", vars::OS),
        format!("!($OS == {})", vars::OS),
    ] {
        let source = format!("task t {{\n    if {guard} {{\n        {MISSING}\n    }}\n}}\n");
        assert!(
            path_misses(&source).is_empty(),
            "{guard}: {:#?}",
            run(&source)
        );
    }
}

#[test]
fn else_if_chains_narrow_one_arm_at_a_time() {
    let source = format!(
        "\
task t {{
    if $OS == {other} {{
        gendef-not-real
    }} else if $OS == {host} {{
        {MISSING}
    }}
}}
",
        other = other_os(),
        host = vars::OS
    );
    let found = path_misses(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains(MISSING), "{found:#?}");
}

#[test]
fn a_guard_covers_nested_statements() {
    // Nested `if`s and a `for` body inherit the guard: nothing under a branch
    // that never runs here is looked up on this machine's `PATH`.
    let source = format!(
        "\
task t {{
    if $OS == {} {{
        for f in a b {{
            gendef-not-real $f
            if exists $f {{
                dlltool-not-real $f
            }}
        }}
    }}
}}
",
        other_os()
    );
    assert!(path_misses(&source).is_empty(), "{:#?}", run(&source));
}

#[test]
fn a_guard_silences_only_the_path_lookup() {
    // Undefined variables and non-portable commands are wrong on every
    // platform, so a platform guard does not excuse them.
    let source = format!(
        "task t {{\n    if $OS == {} {{\n        ^cp $nope b\n    }}\n}}\n",
        other_os()
    );
    let found = run(&source);
    assert!(
        found
            .iter()
            .any(|d| d.message.contains("undefined variable")),
        "{found:#?}"
    );
    assert!(
        found.iter().any(|d| d.message.contains("not portable")),
        "{found:#?}"
    );
}

#[test]
fn a_guard_does_not_leak_past_its_block() {
    // The command after the `if` is unguarded and must still be reported.
    let source = format!(
        "task t {{\n    if $OS == {} {{\n        gendef-not-real\n    }}\n    {MISSING}\n}}\n",
        other_os()
    );
    let found = path_misses(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains(MISSING), "{found:#?}");
}

#[test]
fn the_condition_itself_is_checked_at_the_outer_platform() {
    // `which` in the condition runs wherever the `if` does, so a miss there is
    // a real finding even though the branch it guards is not entered here.
    let source = format!(
        "task t {{\n    if $OS == {} && {MISSING} {{\n        gendef-not-real\n    }}\n}}\n",
        other_os()
    );
    let found = path_misses(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains(MISSING), "{found:#?}");
}

#[test]
fn a_platform_word_resolves_whole() {
    // `$PLATFORM` and `$OS-$ARCH` are the same string, and both decide the
    // guard.
    for guard in [
        format!("$PLATFORM == {}-nonsense", vars::OS),
        format!("$OS-$ARCH == {}-nonsense", vars::OS),
    ] {
        let source = format!("task t {{\n    if {guard} {{\n        {MISSING}\n    }}\n}}\n");
        assert!(
            path_misses(&source).is_empty(),
            "{guard}: {:#?}",
            run(&source)
        );
    }
}

#[test]
fn the_windows_gnu_chorefile_is_clean_everywhere() {
    // End to end, in the shape the bug was reported in.
    let source = format!(
        "\
dist=dist

# build the windows-gnu import libraries
task dylib {{
    mkdir $dist
    if $OS == {} && $ENV == gnu {{
        gendef $dist/foo.dll
        dlltool -d $dist/foo.def -l $dist/foo.lib
    }}
}}
",
        other_os()
    );
    assert_eq!(
        messages(&source),
        Vec::<String>::new(),
        "{:#?}",
        run(&source)
    );
}

// --- 8. include ------------------------------------------------------------

#[test]
fn self_include_is_a_cycle() {
    let found = matching("include chorefile\n", "itself");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
}

#[test]
fn the_same_file_included_twice() {
    let found = matching(
        "include libs.chore\ninclude ./libs.chore\n",
        "more than once",
    );
    assert_eq!(found.len(), 1, "{found:#?}");
}

#[test]
fn namespace_collides_with_a_task() {
    let source = "\
include libs.chore as build

task build {
    echo hi
}
";
    assert_eq!(matching(source, "also the name of a task").len(), 1);
}

#[test]
fn namespace_containing_the_separator() {
    // The parser already rejects `as a::b`, so this arrives as a syntax
    // diagnostic rather than a name diagnostic.
    let found = run("include libs.chore as a::b\n");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("namespace"), "{found:#?}");

    // A tree that does hold such a namespace — an include resolver could
    // build one — is still reported.
    let file = ast::File {
        includes: vec![ast::Include {
            path: "libs.chore".into(),
            namespace: Some("a::b".into()),
            span: Span::new(0, 1),
        }],
        ..ast::File::default()
    };
    let found = check::check(&file, "include libs.chore as a::b\n", &file_path());
    assert!(
        found.iter().any(|d| d.message.contains("contains `::`")),
        "{found:#?}"
    );
}

#[test]
fn a_namespace_used_twice() {
    let source = "include a.chore as libs\ninclude b.chore as libs\n";
    assert_eq!(matching(source, "used twice").len(), 1);
}

// --- clean -----------------------------------------------------------------

/// The worst failure this linter has is a false positive on a chorefile that
/// is fine, so the shape of a real port gets its own test.
const SONA: &str = "\
# where builds land
dist=dist
version=0.1.0

# build the project
task build {
    mkdir $dist
    cargo build --release
    copy target/release/sona$EXE $dist/
}

# download and unpack the toolchain
task deps {
    mkdir vendor
    download gh://sona/toolchain/v1/toolchain-$PLATFORM.tar.gz vendor/tc.tar.gz --sha256 $SHA
    extract vendor/tc.tar.gz vendor/tc --strip 1
}

# run the tests
task test {
    build
    for f in $(find tests *.sona) {
        cargo run -- $f
    }
}

# package a release for one platform
task package target {
    build
    archive $dist $dist/sona-$1-$version.zip
    sha256 $dist/sona-$1-$version.zip > $dist/sona-$1-$version.zip.sha256
}

task clean {
    remove $dist vendor
}
";

#[test]
fn a_real_chorefile_has_no_errors() {
    let source = SONA.replace("$SHA", "0000");
    let found: Vec<_> = errors(&source);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_builtin_only_chorefile_is_completely_silent() {
    // No PATH lookups at all, so this holds on any machine.
    let source = "\
dist=dist

# build
task build {
    mkdir $dist
    write $dist/version 1.0
}

task clean {
    remove $dist
}

task all {
    clean
    build
}
";
    assert_eq!(messages(source), Vec::<String>::new());
}

#[test]
fn the_readme_example_parses_and_is_clean() {
    let source = "\
SHA=0000

# build the project
task build {
    mkdir dist
    cargo build --release
    copy target/release/app$EXE dist/
}

task fetch url {
    download $1 dist/asset --sha256 $SHA
}
";
    parse::parse(source, &file_path()).expect("README example must parse");
    let found = errors(source);
    assert!(found.is_empty(), "{found:#?}");
}
