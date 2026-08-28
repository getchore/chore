//! `check` over real chorefile snippets.
//!
//! Every case parses with `parse::parse` first, so a change to the grammar
//! that breaks these snippets breaks these tests too.

use std::path::{Path, PathBuf};

use chorefile::ast;
use chorefile::check::{self, Diagnostic, Severity};
use chorefile::error::Span;
use chorefile::parse;
use chorefile::resolve::{Merged, Part, Sources};
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

#[test]
fn a_task_of_the_same_name_is_not_flagged_either() {
    // `test` is in REPLACEMENTS (the POSIX `test` program), and it is also
    // the most common task name there is. Resolution is task -> builtin ->
    // PATH, so a bare `test` inside another task calls the task and never
    // reaches the program. chore's own chorefile does exactly this.
    let source = "# t\ntask test {\n    cargo test\n}\n\n# c\ntask ci {\n    test\n}\n";
    assert!(matching(source, "not portable").is_empty());

    // `^test` skips the task and does reach the non-portable program.
    let forced = "# t\ntask test {\n    cargo test\n}\n\n# c\ntask ci {\n    ^test -f x\n}\n";
    let found = matching(forced, "not portable");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].help.as_ref().unwrap().contains("exists"));
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
    assert!(
        found[0]
            .message
            .contains("1 parameter(s): 1 required (one)"),
        "{found:#?}"
    );
    assert!(found[0].help.as_ref().unwrap().contains("task t one arg2"));
}

#[test]
fn the_arity_message_separates_required_from_optional() {
    let source = "\
task t src dest=out {
    echo $3
}
";
    let found = matching(source, "$3");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0]
            .message
            .contains("2 parameter(s): 1 required (src), 1 optional (dest)"),
        "{found:#?}"
    );
    // The suggested header keeps the default rather than proposing to drop it.
    assert!(
        found[0]
            .help
            .as_ref()
            .unwrap()
            .contains("task t src dest=out arg3"),
        "{found:#?}"
    );
}

#[test]
fn an_optional_parameter_is_bound_like_any_other() {
    // `$2` is set whether or not the caller passes it, so reading it is fine
    // and calling the task bare is fine.
    let source = "\
task greet who when=today {
    echo $1 $2
}

task hello {
    greet world
}
";
    assert_eq!(messages(source), Vec::<String>::new());
}

// --- 6b. parameter defaults ------------------------------------------------

#[test]
fn a_default_referencing_an_undefined_variable_is_reported() {
    let source = "task t x=$nope {\n    echo $1\n}\n";
    let found = matching(source, "undefined variable");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    // At the default, not at the task.
    assert_eq!(&source[found[0].at.span.range()], "$nope");
}

#[test]
fn a_default_sees_globals_and_the_builtin_variables() {
    let source = "\
root=/tmp

task build target=$TRIPLE out=$root {
    echo $1 $2
}
";
    assert_eq!(messages(source), Vec::<String>::new());
}

#[test]
fn a_default_may_read_the_parameters_before_it() {
    let source = "task t a b=$1 {\n    echo $1 $2\n}\n";
    assert_eq!(messages(source), Vec::<String>::new());
}

#[test]
fn a_default_cannot_read_its_own_parameter_or_a_later_one() {
    let source = "task t a b=$2 c=$3 {\n    echo $1\n}\n";
    let found = matching(source, "not bound yet");
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found[0].message.contains("`$2`"), "{found:#?}");
    assert!(found[0].message.contains("`b`"), "{found:#?}");
    assert_eq!(&source[found[0].at.span.range()], "$2");
    assert!(
        found[0].help.as_ref().unwrap().contains("`$1`"),
        "{found:#?}"
    );
}

#[test]
fn the_first_default_cannot_read_a_parameter_at_all() {
    let source = "task t a=$1 {\n    echo $1\n}\n";
    let found = matching(source, "not bound yet");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0]
            .help
            .as_ref()
            .unwrap()
            .contains("globals and the builtin variables"),
        "{found:#?}"
    );
}

#[test]
fn a_default_beyond_the_declared_parameters_is_the_ordinary_finding() {
    let source = "task t a=$4 {\n    echo $1\n}\n";
    let found = matching(source, "$4");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("is never set"), "{found:#?}");
}

#[test]
fn a_default_is_checked_for_unknown_commands_too() {
    let source = "task t x=$(definitely-not-a-real-command) {\n    echo $1\n}\n";
    let found = matching(source, "definitely-not-a-real-command");
    assert_eq!(found.len(), 1, "{found:#?}");
}

// --- 6c. parameter names ---------------------------------------------------

#[test]
fn a_duplicate_parameter_name() {
    let source = "task sync src src {\n    echo $1 $2\n}\n";
    let found = matching(source, "twice");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("`sync`"), "{found:#?}");
    assert!(found[0].message.contains("`src`"), "{found:#?}");
    // Points at the second one.
    assert_eq!(found[0].at.span.start, source.rfind("src").unwrap());
    let help = found[0].help.as_ref().unwrap();
    assert!(help.contains("$2") && help.contains("$1"), "{help}");
}

#[test]
fn distinct_parameter_names_are_fine() {
    assert_eq!(
        messages("task sync src dest {\n    echo $1 $2\n}\n"),
        Vec::<String>::new()
    );
}

#[test]
fn a_parameter_read_by_name_is_undefined() {
    let source = "task build target {\n    echo $target\n}\n";
    let found = matching(source, "undefined variable");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    let help = found[0].help.as_ref().unwrap();
    assert!(help.contains("parameter 1"), "{help}");
    assert!(help.contains("$1"), "{help}");
}

#[test]
fn a_parameter_read_by_name_that_is_also_a_global_reads_the_global() {
    let source = "\
target=x86_64

task build target {
    echo $target
}
";
    let found = matching(source, "reads the global");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Warning);
    assert_eq!(&source[found[0].at.span.range()], "$target");
    assert!(found[0].help.as_ref().unwrap().contains("$1"), "{found:#?}");
}

#[test]
fn a_parameter_sharing_a_globals_name_is_not_reported_on_its_own() {
    // Declaring it is not the mistake; reading it by name is.
    let source = "\
target=x86_64

task build target {
    echo $1 $target_dir
}

target_dir=out
";
    assert!(matching(source, "reads the global").is_empty());
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

// --- 9. include trees ------------------------------------------------------
//
// `check_merged` checks a chorefile together with everything it included.
// The trees below are built by hand rather than by `resolve::resolve`, which
// keeps these tests honest about what they cover: the merging is the contract
// `resolve` implements — each file's tasks and globals renamed with the
// namespace it was included under, all in one `ast::File`, every contributing
// file's text in `Sources` — and what is being tested is `check`'s behaviour
// given such a tree, not `resolve`'s ability to produce one.
//
// The handful of cases that genuinely need `resolve` — a cycle, a missing
// file, a duplicate across a flat merge — are at the end, `#[ignore]`d, and
// they assert `resolve` owns the finding rather than `check`.

/// One contributing file: path, the namespace it was included under, source.
type Contribution<'a> = (&'a str, Option<&'a str>, &'a str);

/// Merge as `resolve` will: the first entry is the top-level chorefile.
fn merge(files: &[Contribution]) -> Merged {
    let mut sources = Sources::default();
    let mut merged = ast::File::default();
    let mut parts = Vec::new();
    for (path, namespace, source) in files {
        sources.insert(PathBuf::from(path), (*source).to_string());
        let parsed = parse::parse(source, Path::new(path)).expect("test source must parse");
        for task in parsed.tasks {
            merged.tasks.push(ast::Task {
                name: qualify(*namespace, &task.name),
                ..task
            });
        }
        for global in parsed.globals {
            merged.globals.push(ast::Assign {
                name: qualify(*namespace, &global.name),
                ..global
            });
        }
        // The merged tree's own `includes` stay empty, exactly as `resolve`
        // leaves them: they have been followed, and `parts` is where a
        // per-file include finding has to come from now.
        parts.push(Part {
            path: PathBuf::from(path),
            // The file as written — un-namespaced, spans into its own text.
            // Parsed a second time rather than shared with the merge above,
            // since that one was consumed to build the namespaced names.
            file: parse::parse(source, Path::new(path)).expect("test source must parse"),
            prefix: namespace.map(str::to_string),
        });
    }
    Merged {
        file: merged,
        parts,
        // The directory of the top-level file, which the tests write as a
        // bare `chorefile`.
        root: PathBuf::new(),
        sources,
    }
}

fn qualify(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(ns) => format!("{ns}::{name}"),
        None => name.to_string(),
    }
}

fn merged_messages(files: &[Contribution]) -> Vec<String> {
    check::check_merged(&merge(files))
        .into_iter()
        .map(|d| d.message)
        .collect()
}

fn merged_errors(files: &[Contribution]) -> Vec<Diagnostic> {
    check::check_merged(&merge(files))
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect()
}

/// The whole point of the merged entry point: a finding in an included file
/// must say so, because `line:col` against the top-level file's text points at
/// an unrelated line — or at no line at all.
#[test]
fn a_finding_in_an_included_file_carries_that_file() {
    let libs = "task build {\n    echo $missing\n}\n";
    let found = merged_errors(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        ("libs.chore", Some("libs"), libs),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("$missing"), "{found:#?}");
    assert_eq!(found[0].at.file, Path::new("libs.chore"));
    assert_eq!(found[0].at.line_col(libs), (2, 10));
}

/// And rendering it uses that file's text, which is the reason `Sources`
/// exists at all.
#[test]
fn a_finding_renders_against_the_file_it_points_into() {
    let files: &[Contribution] = &[
        ("chorefile", None, "include libs.chore as libs\n"),
        (
            "libs.chore",
            Some("libs"),
            "# a comment\n# another\ntask build {\n    echo $missing\n}\n",
        ),
    ];
    let tree = merge(files);
    let found = check::check_merged(&tree);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(tree.sources.render(&found[0].at), "libs.chore:4:10");
}

#[test]
fn a_namespaced_call_resolves_against_the_merged_tasks() {
    let found = merged_messages(&[
        (
            "chorefile",
            None,
            "include libs.chore as libs\n\ntask all {\n    libs::build\n}\n",
        ),
        ("libs.chore", Some("libs"), "task build {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_namespaced_call_to_a_task_that_is_not_there() {
    let found = merged_errors(&[
        (
            "chorefile",
            None,
            "include libs.chore as libs\n\ntask all {\n    libs::package\n}\n",
        ),
        ("libs.chore", Some("libs"), "task build {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0]
            .message
            .contains("namespace `libs` has no task `package`"),
        "{found:#?}"
    );
    assert_eq!(found[0].at.file, Path::new("chorefile"));
}

/// The message has to be about the namespace, not about `PATH`: `::` is not
/// how any program on `PATH` is spelled.
#[test]
fn a_namespace_no_include_defines() {
    let found = merged_errors(&[
        ("chorefile", None, "task all {\n    libs::build\n}\n"),
        ("libs.chore", None, "task build {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].message.contains("no `include` defines"),
        "{found:#?}"
    );
    assert!(!found[0].message.contains("PATH"), "{found:#?}");
}

/// A task in an included file calls its neighbours by their bare names — the
/// namespace is how the *outside* reaches them.
#[test]
fn a_bare_sibling_call_inside_an_included_file() {
    let found = merged_messages(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        (
            "libs.chore",
            Some("libs"),
            "task build {\n    echo hi\n}\n\ntask all {\n    build\n}\n",
        ),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

/// Without `as`, everything merges flat and the top level calls it by name.
#[test]
fn a_flat_include_merges_its_tasks_into_scope() {
    let found = merged_messages(&[
        (
            "chorefile",
            None,
            "include libs.chore\n\ntask all {\n    helper\n}\n",
        ),
        ("libs.chore", None, "task helper {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn a_global_from_a_flat_include_is_defined() {
    let found = merged_messages(&[
        (
            "chorefile",
            None,
            "include libs.chore\n\ntask build {\n    echo $dist\n}\n",
        ),
        ("libs.chore", None, "dist=dist\n"),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

/// A namespaced file's own global is `libs::dist` after the merge, and `$dist`
/// inside that file still means it.
#[test]
fn a_namespaced_file_still_sees_its_own_globals() {
    let found = merged_messages(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        (
            "libs.chore",
            Some("libs"),
            "dist=dist\n\ntask build {\n    mkdir $dist\n}\n",
        ),
    ]);
    assert_eq!(found, Vec::<String>::new());
}

/// Cross-file globals are in scope everywhere, but a file that assigns its own
/// global below the line that reads it is still wrong.
#[test]
fn use_before_assignment_survives_the_merge() {
    let found = merged_errors(&[
        (
            "chorefile",
            None,
            "include libs.chore\n\nout=$dist/x\ndist=dist\n",
        ),
        ("libs.chore", None, "task helper {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("$dist"), "{found:#?}");
}

/// A variable that is nowhere in the merged tree is still undefined — the
/// cross-file scope must not become a blanket amnesty.
#[test]
fn an_undefined_variable_is_still_undefined_across_files() {
    let found = merged_errors(&[
        ("chorefile", None, "include libs.chore\n"),
        ("libs.chore", None, "task helper {\n    echo $nowhere\n}\n"),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("$nowhere"), "{found:#?}");
}

#[test]
fn a_non_portable_command_in_an_included_file_is_reported_there() {
    let found = merged_errors(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        (
            "libs.chore",
            Some("libs"),
            "task fetch {\n    curl -L https://example.com/x -o x\n}\n",
        ),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("not portable"), "{found:#?}");
    assert_eq!(found[0].at.file, Path::new("libs.chore"));
    assert!(found[0].help.as_ref().unwrap().contains("download"));
}

/// The guard analysis is per-statement and cares nothing for which file it is
/// in, but the merged walk is a new caller of it, so it gets a test.
#[test]
fn a_platform_guard_still_silences_a_lookup_in_an_included_file() {
    let source = format!(
        "task dylib {{\n    if $OS == {} {{\n        definitely-not-a-real-program\n    }}\n}}\n",
        other_os()
    );
    let found = merged_messages(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        ("libs.chore", Some("libs"), &source),
    ]);
    assert_eq!(found, Vec::<String>::new());

    // The same body without the guard is reported, so the test above is not
    // passing for the wrong reason.
    let unguarded = "task dylib {\n    definitely-not-a-real-program\n}\n";
    let found = merged_messages(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        ("libs.chore", Some("libs"), unguarded),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
}

/// `chore list` is the subcommand, but `chore libs::list` is not — so the
/// name is only dead at the top level.
#[test]
fn a_subcommand_name_is_reachable_inside_a_namespace() {
    let found = merged_messages(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        ("libs.chore", Some("libs"), "task list {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found, Vec::<String>::new());

    let found = merged_errors(&[("chorefile", None, "task list {\n    echo hi\n}\n")]);
    assert!(
        found.iter().any(|d| d.message.contains("subcommand")),
        "{found:#?}"
    );
}

/// A task named after a builtin still shadows it inside an included file: the
/// file's own bare calls reach the task, and there is no spelling left that
/// reaches the builtin.
#[test]
fn a_builtin_name_is_still_shadowed_inside_a_namespace() {
    let found = merged_errors(&[
        ("chorefile", None, "include libs.chore as libs\n"),
        ("libs.chore", Some("libs"), "task write {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("shadows"), "{found:#?}");
}

#[test]
fn findings_are_grouped_by_file() {
    let found = check::check_merged(&merge(&[
        (
            "chorefile",
            None,
            "include libs.chore as libs\n\ntask a {\n    echo $one\n}\n",
        ),
        ("libs.chore", Some("libs"), "task b {\n    echo $two\n}\n"),
    ]));
    let files: Vec<&Path> = found.iter().map(|d| d.at.file.as_path()).collect();
    assert_eq!(files, vec![Path::new("chorefile"), Path::new("libs.chore")]);
}

// --- 9a. did-you-mean across namespaces ------------------------------------

const TWO_NAMESPACES: &str = "\
include libs.chore as libs
include tools.chore as tools
";

/// The wrong project's `build` is the wrong answer, however short the edit
/// distance: several namespaces holding a `build` is the normal shape of a
/// monorepo, not a corner case.
#[test]
fn a_typo_is_answered_from_its_own_namespace() {
    let found = merged_errors(&[
        (
            "chorefile",
            None,
            &format!("{TWO_NAMESPACES}\ntask all {{\n    libs::buld\n}}\n"),
        ),
        ("libs.chore", Some("libs"), "task build {\n    echo hi\n}\n"),
        (
            "tools.chore",
            Some("tools"),
            "task build {\n    echo hi\n}\n",
        ),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    let help = found[0].help.as_ref().unwrap();
    assert!(help.contains("libs::build"), "{help}");
    assert!(!help.contains("tools::build"), "{help}");
}

/// The other half of the same rule: when the namespace is the typo and the
/// task name is exact, the namespace is what gets corrected.
#[test]
fn a_misspelled_namespace_is_corrected() {
    let found = merged_errors(&[
        (
            "chorefile",
            None,
            "include libs.chore as libs\n\ntask all {\n    lib::build\n}\n",
        ),
        ("libs.chore", Some("libs"), "task build {\n    echo hi\n}\n"),
    ]);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].help.as_ref().unwrap().contains("libs::build"),
        "{found:#?}"
    );
}

/// A bare call is answered by a bare task. `libs::build` is not something an
/// author mistypes their way into from `biuld`.
#[test]
fn a_bare_typo_is_not_answered_with_a_namespaced_task() {
    let found = merged_errors(&[
        (
            "chorefile",
            None,
            "include libs.chore as libs\n\ntask all {\n    biuld\n}\n",
        ),
        ("libs.chore", Some("libs"), "task build {\n    echo hi\n}\n"),
    ]);
    // Not a task and not a builtin, so this is the `PATH` warning rather than
    // an error — but whatever it says, it must not name another project's task.
    assert!(found.is_empty(), "{found:#?}");
    let all = check::check_merged(&merge(&[
        (
            "chorefile",
            None,
            "include libs.chore as libs\n\ntask all {\n    biuld\n}\n",
        ),
        ("libs.chore", Some("libs"), "task build {\n    echo hi\n}\n"),
    ]));
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0].severity, Severity::Warning);
    assert!(
        !all[0].help.as_ref().unwrap().contains("libs::build"),
        "{all:#?}"
    );
}

// --- 9b. `$ROOT` -----------------------------------------------------------

/// The interpreter answers `$ROOT` from the run, not from the variable map, so
/// assigning it changes nothing — and an included file that could move the
/// project root would put every builtin's idea of it out of step with the
/// chorefile's.
#[test]
fn assigning_root_has_no_effect_and_is_reported() {
    let found = matching("ROOT=/tmp\n", "assigning `ROOT`");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);

    let inside = matching("task build {\n    ROOT=/tmp\n}\n", "assigning `ROOT`");
    assert_eq!(inside.len(), 1, "{inside:#?}");
}

#[test]
fn reading_root_is_fine() {
    assert_eq!(
        messages("task build {\n    mkdir $ROOT/dist\n}\n"),
        Vec::<String>::new()
    );
}

/// Every other read-only name stays assignable: the interpreter allows it, and
/// the platform-guard analysis is built around a chorefile that does.
#[test]
fn the_platform_variables_are_still_assignable() {
    let found = matching("OS=windows\nPLATFORM=x\nEXE=.exe\n", "no effect");
    assert!(found.is_empty(), "{found:#?}");
}

// --- 9c. through the real resolver -----------------------------------------
//
// The tests above hand `check` a tree; these hand it a directory and let
// `resolve` build one, which is the only way to test the division of labour
// between the two. Each of the last three asserts that `check` says *nothing*,
// because `resolve` refuses to produce a merged tree at all: an include cycle,
// a missing or unreadable included file, and a duplicate name across a flat
// merge are errors there, not findings here.

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chore-check-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// End to end: a real include, a real merge, and a finding that has to come
/// back pointing into the included file and rendering against its text.
#[test]
fn a_resolved_tree_reports_into_the_file_the_finding_is_in() {
    let dir = scratch("resolved");
    std::fs::write(
        dir.join("chorefile"),
        "include libs.chore as libs\n\ntask all {\n    libs::build\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("libs.chore"),
        "task build {\n    echo $missing\n}\n",
    )
    .unwrap();

    let (found, merged) = check::check_path(&dir.join("chorefile"));
    let merged = merged.expect("the tree merges");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("$missing"), "{found:#?}");
    assert_eq!(found[0].at.file, dir.join("libs.chore"));
    assert!(
        merged
            .sources
            .render(&found[0].at)
            .ends_with("libs.chore:2:10"),
        "{}",
        merged.sources.render(&found[0].at)
    );
}

/// The same tree without the mistake: a namespaced call, a bare sibling call
/// and a shared global all have to survive the round trip in silence.
#[test]
fn a_resolved_tree_with_nothing_wrong_is_silent() {
    let dir = scratch("clean");
    std::fs::write(
        dir.join("chorefile"),
        "dist=dist\n\ninclude libs.chore as libs\n\ntask all {\n    libs::package\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("libs.chore"),
        "out=out\n\ntask build {\n    mkdir $out\n}\n\ntask package {\n    build\n    archive $out \
         $dist/p.zip\n}\n",
    )
    .unwrap();

    let (found, merged) = check::check_path(&dir.join("chorefile"));
    assert!(merged.is_some());
    let messages: Vec<&str> = found.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(messages, Vec::<&str>::new());
}

/// A diamond: one file two includes both reach. `resolve` loads it twice —
/// once per arrival, with the prefix each one gave it — and its own mistakes
/// are the same mistakes both times, so they are reported once.
#[test]
fn a_file_reached_twice_is_reported_once() {
    let dir = scratch("diamond");
    std::fs::write(
        dir.join("chorefile"),
        "include a.chore as a
include b.chore as b
",
    )
    .unwrap();
    std::fs::write(
        dir.join("a.chore"),
        "include common.chore
",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.chore"),
        "include common.chore
",
    )
    .unwrap();
    std::fs::write(
        dir.join("common.chore"),
        "task helper {\n    echo $missing\n}\n",
    )
    .unwrap();

    let (found, merged) = check::check_path(&dir.join("chorefile"));
    let merged = merged.expect("two namespaces, so no duplicate");
    assert!(
        merged.parts.len() > merged.sources.files().count(),
        "the fixture must actually load a file twice"
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].at.file, dir.join("common.chore"));
}

/// An include finding in an included file: `merged.file.includes` is empty, so
/// this can only come from that file's own parse.
#[test]
fn an_include_finding_inside_an_included_file() {
    let dir = scratch("nested-include");
    std::fs::write(
        dir.join("chorefile"),
        "include a.chore as a
",
    )
    .unwrap();
    std::fs::write(
        dir.join("a.chore"),
        "include c.chore as build

task build {\n    echo hi\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("c.chore"), "task inner {\n    echo hi\n}\n").unwrap();

    let (found, _) = check::check_path(&dir.join("chorefile"));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].message.contains("also the name of a task"),
        "{found:#?}"
    );
    assert_eq!(found[0].at.file, dir.join("a.chore"));
}

#[test]
fn a_missing_included_file_is_a_diagnostic_not_a_panic() {
    let dir = scratch("missing");
    let top = dir.join("chorefile");
    std::fs::write(&top, "include nope.chore\n").unwrap();

    let (found, merged) = check::check_path(&top);
    assert!(
        merged.is_none(),
        "a tree that cannot be read must not merge"
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("nope.chore"), "{found:#?}");
}

#[test]
fn an_include_cycle_is_reported_once() {
    let dir = scratch("cycle");
    std::fs::write(dir.join("chorefile"), "include a.chore\n").unwrap();
    std::fs::write(dir.join("a.chore"), "include chorefile\n").unwrap();

    let (found, _) = check::check_path(&dir.join("chorefile"));
    assert_eq!(found.len(), 1, "a cycle is one finding, not one per file");
    assert!(
        found[0].message.contains("cycle") || found[0].message.contains("itself"),
        "{found:#?}"
    );
}

#[test]
fn a_duplicate_across_a_flat_merge_is_resolves_error() {
    let dir = scratch("duplicate");
    std::fs::write(
        dir.join("chorefile"),
        "include a.chore\ntask build {\n    echo hi\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("a.chore"), "task build {\n    echo hi\n}\n").unwrap();

    let (found, merged) = check::check_path(&dir.join("chorefile"));
    assert!(
        merged.is_none(),
        "a tree with a duplicate name must not merge"
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("build"), "{found:#?}");
}

// --- 12. `parallel` names tasks --------------------------------------------

#[test]
fn parallel_names_that_are_not_tasks_are_errors() {
    let found = errors(
        "task ci {\n    parallel lint tests --fail-fast\n}\ntask lint {\n    echo hi\n}\n\
task test {\n    echo hi\n}\n",
    );
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].message.contains("`tests` is not a task"),
        "{found:#?}"
    );
    assert_eq!(found[0].help.as_deref(), Some("did you mean `test`?"));
}

#[test]
fn parallel_over_real_tasks_is_quiet() {
    let found = run(
        "task ci {\n    parallel lint test --fail-fast\n}\ntask lint {\n    echo hi\n}\n\
task test {\n    echo hi\n}\n",
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_task_may_not_be_called_parallel() {
    let found = errors("task parallel {\n    echo hi\n}\n");
    assert!(
        found.iter().any(|d| d.message.contains("parallel")),
        "{found:#?}"
    );
}

// --- 13. `require` ---------------------------------------------------------

#[test]
fn an_unmet_require_is_an_error_finding() {
    // A version no build will ever be, so the test says the same thing on
    // every release.
    let found = errors("require 99.0.0\n\ntask build {\n    echo hi\n}\n");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].message.contains("requires chore 99.0.0 or newer"),
        "{found:#?}"
    );
    assert!(
        found[0].message.contains(chorefile::spec::version()),
        "the running version has to be in the message: {found:#?}"
    );
    // The remedy, which is the point of the whole feature.
    let help = found[0].help.as_deref().expect("no help line");
    assert!(help.contains("install.sh"), "help was: {help}");
    assert!(help.contains("install.ps1"), "help was: {help}");
    // The `require` line itself, and it is the first line, so it sorts first.
    assert_eq!(found[0].at.line_col("require 99.0.0\n"), (1, 1));
}

#[test]
fn a_met_require_is_quiet() {
    // The oldest version there is, which every build satisfies.
    let found = run("require 0.0.0\n\ntask build {\n    echo hi\n}\n");
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_require_of_the_running_version_is_met() {
    // "At least this" includes this: the version that shipped the feature is
    // the version a chorefile using it names.
    let source = format!(
        "require {}\ntask build {{\n    echo hi\n}}\n",
        chorefile::spec::version()
    );
    assert!(run(&source).is_empty(), "{:#?}", run(&source));
}

// --- 14. script blocks -----------------------------------------------------
//
// A `script` block hands raw text to another interpreter. The command in front
// of it is an ordinary command and is checked like one; the body is another
// language and is checked for *nothing*. The tests below pin both halves, and
// the second half is the one that matters: a checker that went looking inside
// the body would report confident nonsense about text it cannot parse.

/// The `check` warnings, which is where every script finding lives.
fn warnings(source: &str) -> Vec<Diagnostic> {
    run(source)
        .into_iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect()
}

/// The once-per-file summary: `script` blocks are unchecked and unpreviewable.
fn unchecked(source: &str) -> Vec<Diagnostic> {
    matching(source, "--dry")
}

fn shell_findings(source: &str) -> Vec<Diagnostic> {
    matching(source, "host shell")
}

#[test]
fn a_script_block_with_a_clean_command_is_not_an_error() {
    let source = "\
task gen {
    script python3 - {
        print('hello')
    }
}
";
    assert!(errors(source).is_empty(), "{:#?}", run(source));
}

#[test]
fn an_undefined_variable_in_the_command_is_reported() {
    // The command words are expanded like any other command's, so a `$nope`
    // in the argv is exactly the finding it would be anywhere else.
    let source = "\
task gen {
    script python3 $nope {
        print('hello')
    }
}
";
    let found = matching(source, "undefined variable");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("$nope"), "{found:#?}");
}

#[test]
fn the_body_is_never_checked() {
    // The test that pins the whole rule. Every line of this body would be a
    // finding if it were chorefile source: an undefined variable, two
    // non-portable commands, a command no `PATH` has. None of it is chorefile
    // source, so none of it is reported — a `$PATH` in a shell string is not a
    // chore variable and a `curl` in a shell block is not a chore command.
    let source = "\
task gen {
    script python3 - {
        echo $nope
        curl https://example.com -o out.txt
        rm -rf build
        cp a b
        definitely-not-a-real-program --x
        exit 1
    }
}
";
    for needle in [
        "undefined variable",
        "not portable",
        "definitely-not-a-real-program",
        "nope",
        "curl",
        "rm",
    ] {
        assert!(
            matching(source, needle).is_empty(),
            "the body was read: {needle} in {:#?}",
            run(source)
        );
    }
    assert!(errors(source).is_empty(), "{:#?}", run(source));
    // And the one thing that *is* said about it is said about the block, not
    // about anything inside it.
    assert_eq!(unchecked(source).len(), 1, "{:#?}", run(source));
}

#[test]
fn a_missing_interpreter_is_a_warning() {
    let source = format!("task gen {{\n    script {MISSING} - {{\n        whatever\n    }}\n}}\n");
    let found = path_misses(&source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Warning);
    // A missing interpreter is a `PATH` miss like any other: this machine is
    // not necessarily the machine that runs the task.
    assert!(found[0].help.as_ref().unwrap().contains("CI"), "{found:#?}");
    // It points at the block, starting at the keyword.
    assert!(
        source[found[0].at.span.range()].starts_with("script"),
        "{found:#?}"
    );
}

#[test]
fn an_interpreter_guarded_off_this_platform_is_not_reported() {
    // The MinGW rule, applied to `script`: a block that this host never enters
    // says nothing about whether this host has the interpreter.
    let source = format!(
        "\
task gen {{
    if $OS == {} {{
        script {MISSING} - {{
            whatever
        }}
    }}
}}
",
        other_os()
    );
    assert!(path_misses(&source).is_empty(), "{:#?}", run(&source));
}

#[test]
fn a_guard_that_names_this_host_still_reports_the_interpreter() {
    let source = format!(
        "\
task gen {{
    if $OS == {} {{
        script {MISSING} - {{
            whatever
        }}
    }}
}}
",
        vars::OS
    );
    assert_eq!(path_misses(&source).len(), 1, "{:#?}", run(&source));
}

#[test]
fn a_task_or_builtin_named_as_the_interpreter_resolves_first() {
    // task → builtin → `PATH`, exactly as for any other command.
    let source = "\
task run-it {
    echo hi
}

task gen {
    script run-it {
        whatever
    }
}
";
    assert!(path_misses(source).is_empty(), "{:#?}", run(source));
}

// --- 14a. the once-per-file summary ----------------------------------------
//
// A reader has to be told that `check` and `--dry` stop at the opening brace,
// or they will assume the usual guarantees hold everywhere. They are told
// once, with a count — not once per block, which would make a chorefile with
// ten legitimate blocks emit ten permanent warnings nobody can act on.

#[test]
fn a_script_block_is_reported_as_unchecked() {
    let source = "\
task gen {
    script python3 - {
        print('hello')
    }
}
";
    let found = unchecked(source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Warning);
    // Both guarantees, named.
    assert!(found[0].message.contains("`check`"), "{found:#?}");
    assert!(found[0].message.contains("--dry"), "{found:#?}");
    // Never an error: a `script` block is a documented feature, not a mistake.
    assert!(errors(source).is_empty(), "{:#?}", run(source));
}

#[test]
fn many_script_blocks_are_reported_once_with_a_count() {
    let source = "\
task a {
    script python3 - {
        print(1)
    }
}

task b {
    script python3 - {
        print(2)
    }
}

task c {
    script python3 - {
        print(3)
    }
}
";
    let found = unchecked(source);
    assert_eq!(
        found.len(),
        1,
        "one per file, whatever the count: {found:#?}"
    );
    assert!(found[0].message.contains('3'), "{found:#?}");
    // At the first block, so a reader has somewhere to start.
    assert_eq!(found[0].at.line_col(source).0, 2);
}

#[test]
fn a_file_without_a_script_block_pays_nothing() {
    let source = "task build {\n    echo hi\n}\n";
    assert!(unchecked(source).is_empty(), "{:#?}", run(source));
    assert!(run(source).is_empty(), "{:#?}", run(source));
}

// --- 14b. a host shell as the interpreter ----------------------------------
//
// `script sh -` reintroduces the one thing `chore` exists to remove. A
// warning, per block, and never an error: a deliberate author is allowed to
// do it.

#[test]
fn a_shell_interpreter_is_warned_about() {
    for shell in ["sh", "bash", "zsh", "cmd", "powershell"] {
        let source = format!("task gen {{\n    script {shell} - {{\n        echo hi\n    }}\n}}\n");
        let found = shell_findings(&source);
        assert_eq!(found.len(), 1, "{shell}: {:#?}", run(&source));
        assert_eq!(found[0].severity, Severity::Warning);
        assert!(found[0].message.contains(shell), "{found:#?}");
        // Never an error, however deliberate or otherwise.
        assert!(errors(&source).is_empty(), "{shell}: {:#?}", run(&source));
        // The help names the shape a deliberate author is aiming for.
        let help = found[0].help.as_deref().expect("no help line");
        assert!(help.contains("$OS"), "help was: {help}");
    }
}

#[test]
fn a_shell_by_path_is_still_a_shell() {
    let source = "\
task gen {
    script /bin/sh - {
        echo hi
    }
}
";
    assert_eq!(shell_findings(source).len(), 1, "{:#?}", run(source));
}

#[test]
fn a_portable_interpreter_is_not_warned_about() {
    // The whole point of the distinction: these behave the same wherever they
    // are installed, so there is nothing to say about them.
    for command in ["python3 -", "node -", "nu --stdin", "uv run -"] {
        let source =
            format!("task gen {{\n    script {command} {{\n        print(1)\n    }}\n}}\n");
        assert!(
            shell_findings(&source).is_empty(),
            "{command}: {:#?}",
            run(&source)
        );
    }
}

#[test]
fn a_platform_guard_does_not_silence_the_shell_finding() {
    // Unlike a `PATH` miss, this is not a fact about the machine running
    // `check`, so it must not depend on which machine that is: a chorefile
    // checked on a Mac and on Windows says the same thing about `script sh`.
    let source = format!(
        "\
task gen {{
    if $OS == {} {{
        script sh - {{
            echo hi
        }}
    }}
}}
",
        other_os()
    );
    assert_eq!(shell_findings(&source).len(), 1, "{:#?}", run(&source));
}

#[test]
fn shell_blocks_still_get_exactly_one_summary() {
    // Two findings about two different things: `sh` per block, the unchecked
    // summary once.
    let source = "\
task a {
    script sh - {
        echo one
    }
}

task b {
    script sh - {
        echo two
    }
}
";
    assert_eq!(shell_findings(source).len(), 2, "{:#?}", run(source));
    assert_eq!(unchecked(source).len(), 1, "{:#?}", run(source));
    // Nothing else: `sh` may or may not be on this machine's `PATH`, and that
    // miss is the only other finding a host is allowed to contribute.
    let other = warnings(source).len() - shell_findings(source).len() - unchecked(source).len();
    assert_eq!(other, path_misses(source).len(), "{:#?}", run(source));
}

// --- 14c. blocks in every position a chain can take -------------------------
//
// A `script` block is a chain, not a statement: it composes like anything else
// that runs, and `x=$(script uv run - { ... })` — computing a value in another
// language and using it in the task — is the main reason to reach for one. So
// every rule above has to hold in the new positions too, and none of them may
// be reached by a path that skips the block. The walk goes through one place
// (`Checker::chain`), and these are what pin that.

#[test]
fn a_block_in_a_capture_reports_an_undefined_variable_in_its_argv() {
    // The command words are expanded wherever the block is written, so the
    // finding is the same one a statement block gives.
    let source = "\
task gen {
    x=$(script python3 $nope {
        print('hello')
    })
    echo $x
}
";
    let found = matching(source, "undefined variable");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].severity, Severity::Error);
    assert!(found[0].message.contains("$nope"), "{found:#?}");
}

#[test]
fn a_body_in_a_capture_is_still_never_checked() {
    // `the_body_is_never_checked`, moved into a capture. The position decides
    // what happens to the block's *value*; it decides nothing about whether
    // the body is read, and the answer there is still never.
    let source = "\
task gen {
    version=$(script python3 - {
        echo $nope
        curl https://example.com -o out.txt
        rm -rf build
        cp a b
        definitely-not-a-real-program --x
        exit 1
    })
    echo $version
}
";
    for needle in [
        "undefined variable",
        "not portable",
        "definitely-not-a-real-program",
        "nope",
        "curl",
        "rm",
    ] {
        assert!(
            matching(source, needle).is_empty(),
            "the body was read: {needle} in {:#?}",
            run(source)
        );
    }
    assert!(errors(source).is_empty(), "{:#?}", run(source));
    assert_eq!(unchecked(source).len(), 1, "{:#?}", run(source));
}

#[test]
fn a_body_in_a_capture_in_a_condition_is_still_never_checked() {
    // The deepest nesting there is: a capture inside an `if` condition. The
    // walk reaches the block through the condition, through the word and
    // through the capture, and still reads none of the body.
    let source = "\
task gen {
    if $(script python3 - {
        echo $nope
        curl https://example.com
    }) == yes {
        echo ok
    }
}
";
    for needle in ["undefined variable", "not portable", "nope", "curl"] {
        assert!(
            matching(source, needle).is_empty(),
            "the body was read: {needle} in {:#?}",
            run(source)
        );
    }
    assert!(errors(source).is_empty(), "{:#?}", run(source));
    // And it is still counted: a block a reader cannot see from the statement
    // list is exactly as unread as one they can.
    assert_eq!(unchecked(source).len(), 1, "{:#?}", run(source));
}

#[test]
fn the_count_includes_blocks_in_captures_and_pipes() {
    // Three blocks, three positions, one summary carrying `3`. A count that
    // only saw statement blocks would understate how much of the file is
    // outside the analysis, which is the one thing the summary is for.
    let source = "\
task gen {
    script python3 - {
        print(1)
    }
    x=$(script python3 - {
        print(2)
    })
    script python3 - {
        print(3)
    } | echo
}
";
    let found = unchecked(source);
    assert_eq!(found.len(), 1, "one per file: {found:#?}");
    assert!(found[0].message.contains('3'), "{found:#?}");
    // Still the first one in the file, which here is the statement block.
    assert_eq!(found[0].at.line_col(source).0, 2);
}

#[test]
fn the_summary_points_at_the_first_block_even_inside_a_capture() {
    // File order, not walk order: the earliest block is the one in the
    // capture, and that is where a reader is sent.
    let source = "\
task gen {
    x=$(script python3 - {
        print(1)
    })
    script python3 - {
        print(2)
    }
}
";
    let found = unchecked(source);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains('2'), "{found:#?}");
    assert_eq!(found[0].at.line_col(source).0, 2);
}

#[test]
fn a_shell_in_a_capture_is_still_a_host_shell() {
    // The form the feature exists for is also the form that makes reaching for
    // `sh` tempting, so the warning has to survive the move into a capture.
    let source = "\
task gen {
    x=$(script sh - {
        echo hi
    })
    echo $x
}
";
    let found = shell_findings(source);
    assert_eq!(found.len(), 1, "{:#?}", run(source));
    assert_eq!(found[0].severity, Severity::Warning);
    assert!(found[0].message.contains("sh"), "{found:#?}");
    assert!(errors(source).is_empty(), "{:#?}", run(source));
}

#[test]
fn a_missing_interpreter_is_still_a_warning_on_the_far_side_of_a_pipe() {
    let source =
        format!("task gen {{\n    echo hi | script {MISSING} - {{\n        whatever\n    }}\n}}\n");
    let found = path_misses(&source);
    assert_eq!(found.len(), 1, "{:#?}", run(&source));
    assert_eq!(found[0].severity, Severity::Warning);
    assert!(
        source[found[0].at.span.range()].starts_with("script"),
        "{found:#?}"
    );
}

#[test]
fn a_platform_guard_still_silences_an_interpreter_miss_in_a_capture() {
    // The suppression follows the scope, not the statement form: a capture
    // inside a branch this host never enters is off-platform like anything
    // else in it.
    let source = format!(
        "\
task gen {{
    if $OS == {} {{
        x=$(script {MISSING} - {{
            whatever
        }})
        echo $x
    }}
}}
",
        other_os()
    );
    assert!(path_misses(&source).is_empty(), "{:#?}", run(&source));
}
