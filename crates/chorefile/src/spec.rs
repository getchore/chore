//! `chore spec` — the language reference as JSON, for agents and editors.
//!
//! Emits the builtin commands and their flags, the builtin variables, the
//! control-flow forms, the comparison operators, and the reserved names, so a
//! tool can learn the language without scraping `help`.
//!
//! Everything here is `const` static data with `&'static str` fields: the
//! reference is baked into the binary, `help` reads it as Rust structs rather
//! than parsing JSON back out, and nothing allocates until [`json`] is
//! called.
//!
//! The JSON keys are exactly the struct field names, so a consumer that has
//! read one has read the other.

use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// the shapes
// ---------------------------------------------------------------------------

/// One builtin command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builtin {
    pub name: &'static str,
    /// The usage line, in the same spelling `help` prints.
    pub usage: &'static str,
    /// One line, for a listing.
    pub summary: &'static str,
    /// The details a caller has to know to use it correctly.
    pub description: &'static str,
    /// True when the command changes something outside the process.
    ///
    /// This is the `--dry` rule in one field: an effectful builtin is echoed
    /// and skipped, a read-only one still runs, because the conditions and
    /// captures downstream of it would otherwise be answered with nothing.
    pub effects: bool,
    pub flags: &'static [Flag],
}

/// One `--flag` a builtin accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag {
    pub name: &'static str,
    /// The value the flag takes, or `""` when it takes none.
    pub argument: &'static str,
    /// What happens when the flag is absent.
    pub default: &'static str,
    pub meaning: &'static str,
}

/// One builtin variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variable {
    pub name: &'static str,
    /// The value set, or the format, whichever describes it.
    pub values: &'static str,
    pub meaning: &'static str,
    /// `run` for a value fixed for the whole invocation, `task` for one that
    /// depends on where it is read from.
    pub scope: &'static str,
}

/// One statement form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Form {
    pub name: &'static str,
    /// The shape, with metavariables.
    pub syntax: &'static str,
    pub example: &'static str,
    pub meaning: &'static str,
}

/// One condition form: a comparison, or a command's exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Condition {
    pub syntax: &'static str,
    pub meaning: &'static str,
}

/// One chaining or redirection operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operator {
    pub symbol: &'static str,
    pub meaning: &'static str,
}

/// One rule that is easy to get wrong from the syntax alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub name: &'static str,
    pub rule: &'static str,
}

// ---------------------------------------------------------------------------
// the data
// ---------------------------------------------------------------------------

const NO_FLAGS: &[Flag] = &[];

/// Every builtin, in `builtins::NAMES` order. The `tests/spec.rs` test keeps
/// the two lists in step: a builtin added there without an entry here fails
/// the build's test run rather than silently missing from the reference.
static BUILTINS: &[Builtin] = &[
    Builtin {
        name: "download",
        usage: "download <url> <dest> [--retries n] [--timeout s] [--sha256 h]",
        summary: "fetch a URL to a file",
        description: "Accepts http(s) and gh://owner/repo/tag/asset, which expands to the \
GitHub release-asset URL. A `dest` ending in `/`, or naming an existing directory, means \
\"into this directory under the remote filename\"; anything else is the output path. The \
bytes stream into a temp file beside the destination and are renamed into place at the end, \
so an interrupted download never leaves a half-file a later run would trust. GITHUB_TOKEN or \
GH_TOKEN, if set, is sent as a bearer token. 5xx and 429 are retried; other 4xx are not.",
        effects: true,
        flags: &[
            Flag {
                name: "--retries",
                argument: "n",
                default: "3",
                meaning: "extra attempts after the first, with 1s/2s/4s/8s backoff",
            },
            Flag {
                name: "--timeout",
                argument: "s",
                default: "60",
                meaning: "seconds for the whole request, not per byte",
            },
            Flag {
                name: "--sha256",
                argument: "h",
                default: "no check",
                meaning: "64-character hex digest; a mismatch deletes the file and fails",
            },
        ],
    },
    Builtin {
        name: "extract",
        usage: "extract <archive> <dest> [--member name] [--strip n] [--flatten]",
        summary: "unpack a zip or tar, compressed or not",
        description: "Handles zip, tar, and .gz, .xz and .zst streams holding either a tar or \
a single file. The format comes from the extension, and from the leading bytes when the name \
says nothing. Entry names are checked before they become paths: an absolute name, or one that \
climbs out of `dest`, aborts the whole extraction. For a compressed single file, --member and \
--strip are an error, and a `dest` ending in `/` means \"into this directory, under the name \
with the compression extension removed\". Without --flatten an entry keeps the directory path it \
had inside the archive, so `--member sona` can land at `dest/pkg/bin/sona`.",
        effects: true,
        flags: &[
            Flag {
                name: "--member",
                argument: "name",
                default: "every entry",
                meaning: "extract only this entry, matched against its full path in the archive \
or just its filename, so `--member chore` finds `bin/chore`",
            },
            Flag {
                name: "--strip",
                argument: "n",
                default: "0",
                meaning: "drop n leading path components from each entry; entries with fewer \
components are skipped",
            },
            Flag {
                name: "--flatten",
                argument: "",
                default: "off",
                meaning: "write every entry directly into `dest` under its base name, dropping \
the directory path it had inside the archive; two entries that would collide is an error, and it \
cannot be combined with --strip",
            },
        ],
    },
    Builtin {
        name: "archive",
        usage: "archive <src...> <dest>",
        summary: "pack files and directories",
        description: "The last argument is the destination, and the format comes from its \
extension: .zip, .tar, .tar.gz or .tgz, .tar.xz or .txz, .tar.zst or .tzst. Anything else is an \
error. Each source's own name is a top-level entry, as with `tar cf` and `zip -r`, so extracting \
gives the directory back rather than its contents loose; a source written with a trailing `/` \
contributes its contents instead, with no directory of its own, the way `extract` reads a \
trailing slash on its `dest`. Several sources pack side by side into one archive, and two of \
them claiming the same top-level name is an error. Entries are sorted, so the same tree packs \
to the same bytes twice.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "copy",
        usage: "copy <src> <dest>",
        summary: "copy a file or a whole directory tree",
        description: "Missing parent directories of `dest` are created. When `dest` is an \
existing directory the source lands inside it under its own name, as `cp` does; otherwise \
`dest` is the exact path written.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "move",
        usage: "move <src> <dest>",
        summary: "rename or move a file or directory",
        description: "A rename where the filesystem allows one, a copy-then-delete across \
volumes. Same `dest`-is-a-directory rule as `copy`. A missing source is an error, unlike \
`remove`.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "remove",
        usage: "remove <path...>",
        summary: "delete paths, recursively",
        description: "Recursive, and silent about paths that are already gone, so a cleanup \
task is safe to run twice. Refuses the filesystem root and $ROOT itself; everything under \
$ROOT is fair game.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "mkdir",
        usage: "mkdir <path...>",
        summary: "create directories",
        description: "Always -p: parent directories are created, and a directory that already \
exists is not an error.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "chmod",
        usage: "chmod <mode> <path>",
        summary: "set permission bits",
        description: "An octal mode, with or without a leading 0 or 0o. Windows has no mode to \
set, so only the owner-write bit is read there: clearing it marks the file read-only, setting \
it clears the flag. The execute bit is ignored on Windows, where a file is executable by \
extension.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "which",
        usage: "which <name>",
        summary: "resolve a program on PATH",
        description: "Prints the resolved path and exits 0, or prints nothing and exits 1, so \
`if which cargo { }` works. A name containing `/` is treated as a path rather than a PATH \
lookup. On Windows every PATHEXT suffix is tried, the name as written first.",
        effects: false,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "find",
        usage: "find <root> <name...>",
        summary: "list matching paths under a directory",
        description: "Every entry under `root` whose filename matches one of the patterns, one \
per line, depth-first and sorted. Patterns support `*` and `?` and match the filename only, \
case-sensitively on every platform. Results are printed with `root` as it was written, so \
they can be fed straight back to another command. A `root` that is not a directory is an \
error. Symlinked directories are not followed.",
        effects: false,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "read",
        usage: "read <file>",
        summary: "print a file's contents",
        description: "Contents with surrounding whitespace removed, which is what `$(...)` \
would do to them anyway.",
        effects: false,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "write",
        usage: "write <file> <text>",
        summary: "write text to a file",
        description: "Overwrites, creating parent directories, and adds the trailing newline \
that `read` trims back off. Appending is the interpreter's `>>`, so there is no flag for it.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "sha256",
        usage: "sha256 <file>",
        summary: "print a file's SHA-256",
        description: "Lowercase hex, the spelling `download --sha256` and every checksum file \
use.",
        effects: false,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "exists",
        usage: "exists <path>",
        summary: "test whether a path exists",
        description: "Exits 0 when the path is there and 1 when it is not, and prints nothing: \
it exists to be the condition of an `if`. A broken symlink still exists.",
        effects: false,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "changed",
        usage: "changed <path...>",
        summary: "test whether inputs changed since the last run",
        description: "Exits 0 when any path differs from the last recorded run and 1 when \
every one of them is unchanged, so `if changed src Cargo.toml { }` skips work that is already \
done. A directory is hashed recursively, contents and filenames both, so a rename counts; a \
missing path counts as changed and is recorded as missing. Exit 0 records the new state and \
exit 1 records nothing. The record lives in $ROOT/.chore/state, keyed on the calling task and \
the exact argument list, so two tasks watching the same paths do not clobber each other. \
--force reports changed without consulting the state; --dry reads the state but never writes \
it, so a preview cannot make the next real run skip work it never did.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "echo",
        usage: "echo <text...>",
        summary: "print its arguments",
        description: "Arguments joined with single spaces and one newline. What separates the \
arguments is word splitting at the call site, not the original spacing.",
        effects: false,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "env",
        usage: "env <NAME> [value]",
        summary: "read or set an environment variable",
        description: "With one argument it prints the value, or exits 1 when the name is \
unset — a nonzero exit rather than an error, so `if env CI { }` works. With two it sets the \
variable for the rest of the run and for every process the run spawns. Reading still happens \
under --dry; setting does not.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "fail",
        usage: "fail <msg>",
        summary: "stop the task with a message",
        description: "Under `try`, or as a condition, it is just a command that exited \
nonzero. It has no effects, so --dry runs it and the preview stops where the real run would.",
        effects: false,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "sleep",
        usage: "sleep <seconds>",
        summary: "wait",
        description: "Fractional seconds are allowed. Skipped under --dry, which is meant to \
be instant.",
        effects: true,
        flags: NO_FLAGS,
    },
    Builtin {
        name: "parallel",
        usage: "parallel [--fail-fast] <task>...",
        summary: "run tasks concurrently and wait for them",
        description: "The arguments are task names, each run on its own thread. Run-once is kept across them: a task two siblings both call runs once, and the second waits for the first and reuses its result. Each task's output is collected into a block of its own and printed when everything has finished, in the order the tasks were named, so nothing interleaves. By default every task runs to the end and all the failures are reported; the call then fails with the first failing task's exit code. `exit` inside a task still ends the whole run, once the siblings have finished. Under --dry the tasks are previewed one after another rather than run concurrently.",
        effects: true,
        flags: &[Flag {
            name: "--fail-fast",
            argument: "",
            default: "off",
            meaning: "stop as soon as a task fails; the siblings already running are not killed, each stops before its next statement, and a task stopped this way is not recorded as having run",
        }],
    },
];

/// Every builtin variable, in `vars::BUILTIN_NAMES` order.
static VARIABLES: &[Variable] = &[
    Variable {
        name: "OS",
        values: "macos | linux | windows",
        meaning: "the host operating system",
        scope: "run",
    },
    Variable {
        name: "ARCH",
        values: "x86_64 | arm64",
        meaning: "the host architecture",
        scope: "run",
    },
    Variable {
        name: "ENV",
        values: "gnu | msvc | \"\"",
        meaning: "the Windows toolchain; empty everywhere else",
        scope: "run",
    },
    Variable {
        name: "PLATFORM",
        values: "$OS-$ARCH",
        meaning: "the pair, for naming release artifacts",
        scope: "run",
    },
    Variable {
        name: "TRIPLE",
        values: "e.g. aarch64-apple-darwin",
        meaning: "the rustc target triple, for anything handed to a toolchain",
        scope: "run",
    },
    Variable {
        name: "EXE",
        values: "\"\" | .exe",
        meaning: "the executable suffix, so `build/tool$EXE` is portable",
        scope: "run",
    },
    Variable {
        name: "HOME",
        values: "a path",
        meaning: "the user's home directory, from HOME or USERPROFILE; empty if unset",
        scope: "run",
    },
    Variable {
        name: "ROOT",
        values: "a path",
        meaning: "the directory holding the top-level chorefile; one per invocation, and \
unchanged inside included files",
        scope: "run",
    },
    Variable {
        name: "CWD",
        values: "a path",
        meaning: "the interpreter's current directory. `cd` moves it, and the move dies with \
the task that made it, so it is per-task rather than per-run",
        scope: "task",
    },
    Variable {
        name: "TASK",
        values: "a task name",
        meaning: "the task currently running; a called task sees its own name, not its \
caller's",
        scope: "task",
    },
    Variable {
        name: "NOW",
        values: "YYYY-MM-DDTHH:MM:SSZ",
        meaning: "UTC timestamp, taken once per invocation so two lines of one recipe cannot \
disagree",
        scope: "run",
    },
];

/// The statement forms. `include` is listed as deferred rather than omitted:
/// its semantics are settled, and a consumer should know the spelling it will
/// get rather than invent one.
static FORMS: &[Form] = &[
    Form {
        name: "assignment",
        syntax: "name=value",
        example: "target=$PLATFORM",
        meaning: "Set a variable. Inside a task it is local to that task; at the top level it \
is a global, evaluated once before the first task runs.",
    },
    Form {
        name: "interpolation",
        syntax: "$name  \"$name/lib\"  $1  $@  $#",
        example: "cargo build --target-dir \"$ROOT/target\"",
        meaning: "Substitute a variable. `$1`, `$2`, ... are a task's parameters, `$@` all of \
them as separate words, `$#` the count.",
    },
    Form {
        name: "capture",
        syntax: "$(cmd)",
        example: "version=$(read VERSION)",
        meaning: "Run the command and substitute its stdout with surrounding whitespace \
removed. A nonzero exit fails the statement unless it is wrapped in `try`.",
    },
    Form {
        name: "if",
        syntax: "if cond { } else if cond { } else { }",
        example: "if $OS == windows { echo msvc } else if $OS == macos { echo apple } else { \
echo other }",
        meaning: "Branch on a condition. `else if` and `else` are both optional.",
    },
    Form {
        name: "for",
        syntax: "for name in words { }",
        example: "for f in $(find src *.rs) { echo $f }",
        meaning: "Iterate. Each word is split on whitespace after interpolation, so one \
`$(find ...)` yields one iteration per match.",
    },
    Form {
        name: "try",
        syntax: "try cmd",
        example: "try remove build/stale.lock",
        meaning: "Run the command and continue even if it exits nonzero.",
    },
    Form {
        name: "exit",
        syntax: "exit [code]",
        example: "exit 1",
        meaning: "Stop the run with the given code, 0 by default. A called task's `exit` \
unwinds its caller too.",
    },
    Form {
        name: "return",
        syntax: "return [code]",
        example: "if exists $out { return }",
        meaning: "End the current task and hand control back to its caller, which carries on. \
An optional code becomes the task's exit status, so `&&`, `||`, `try`, a condition and a \
capture read it as they read any command's. Inside a `for` it leaves the task, not the loop.",
    },
    Form {
        name: "task",
        syntax: "task name [param[=default]...] { }",
        example: "# Build the release binary\ntask build target { cargo build --release \
--target $target }",
        meaning: "Define a task. Parameters bind to `$1`, `$2`, ... in the body. The comment \
line directly above the `task` is its description, which `chore list` prints.",
    },
    Form {
        name: "include",
        syntax: "include path [as name]",
        example: "include libs/chorefile as libs",
        meaning: "Pull another chorefile in and merge it. Paths resolve relative to the \
including file, a directory means the chorefile inside it, and `$ROOT` stays the top-level \
chorefile's directory. `as` namespaces both tasks and globals as `libs::build`; without it \
everything merges flat, where a duplicate name across two files is an error. A cycle names the \
whole loop.",
    },
];

static CONDITIONS: &[Condition] = &[
    Condition {
        syntax: "$a == $b",
        meaning: "string equality; compare against \"\" to test for empty",
    },
    Condition {
        syntax: "$a != $b",
        meaning: "string inequality",
    },
    Condition {
        syntax: "$a contains x",
        meaning: "substring",
    },
    Condition {
        syntax: "$a starts-with x",
        meaning: "prefix",
    },
    Condition {
        syntax: "$a ends-with x",
        meaning: "suffix",
    },
    Condition {
        syntax: "cmd",
        meaning: "any command, true when it exits 0. `exists path` and `which name` are the \
two written for this position.",
    },
    Condition {
        syntax: "!cond",
        meaning: "negation",
    },
    Condition {
        syntax: "cond && cond",
        meaning: "both",
    },
    Condition {
        syntax: "cond || cond",
        meaning: "either",
    },
];

static CHAINING: &[Operator] = &[
    Operator {
        symbol: "&&",
        meaning: "run the right side only if the left exited 0",
    },
    Operator {
        symbol: "||",
        meaning: "run the right side only if the left exited nonzero",
    },
    Operator {
        symbol: "|",
        meaning: "pipe the left side's stdout into the right side's stdin; the pipeline's \
status is the last command's",
    },
    Operator {
        symbol: ">",
        meaning: "write stdout to a file, truncating it",
    },
    Operator {
        symbol: ">>",
        meaning: "append stdout to a file",
    },
    Operator {
        symbol: "2>",
        meaning: "write stderr to a file",
    },
];

/// How a bare command name is resolved, most specific first.
static RESOLUTION: &[Rule] = &[
    Rule {
        name: "task",
        rule: "A task defined in the chorefile wins, so a project can name a task `build` and \
call it from another task.",
    },
    Rule {
        name: "builtin",
        rule: "Then a builtin. Builtins are reserved: `check` reports a task that shadows one.",
    },
    Rule {
        name: "PATH",
        rule: "Then a program on PATH, run directly — never through a shell.",
    },
    Rule {
        name: "^",
        rule: "A leading `^` skips straight to PATH: `^find src -name '*.rs'` runs the system \
find rather than the builtin.",
    },
    Rule {
        name: "cd",
        rule: "`cd` is neither a task nor a builtin: it moves the interpreter's directory, not \
the process's, and the move is undone when the task that made it returns.",
    },
];

/// The rules a reader will otherwise get wrong. Every one of these has a
/// plausible-looking wrong answer, which is why they are stated rather than
/// left to be inferred from the syntax.
static RULES: &[Rule] = &[
    Rule {
        name: "word splitting",
        rule: "A quoted word is always exactly one argument. An unquoted `$var` splits on \
whitespace, as in sh. There are no arrays and no quoting inside a variable, so an argument \
that contains a space must be written quoted at the call site: `-G \"MinGW Makefiles\"`, not \
`-G $generator`.",
    },
    Rule {
        name: "no shell",
        rule: "argv is handed to the OS directly. Nothing is re-quoted, re-expanded, or \
globbed by a shell, so shell metacharacters in an argument reach the program as literal \
characters.",
    },
    Rule {
        name: "run once",
        rule: "A task runs once per invocation, keyed on name AND arguments, so a \
parameterised task called twice with different arguments runs twice. `--force` disables it.",
    },
    Rule {
        name: "--dry",
        rule: "Echoes every command and skips the ones with effects, but captures and \
conditions still run: a `$(...)` that did not execute would leave every interpolated path \
downstream empty and the preview would describe a run that could never happen.",
    },
    Rule {
        name: "paths",
        rule: "Paths are always written with `/`, on every platform, and converted to `\\` \
only when handed to Windows. Paths printed back — by `find`, `which`, `$CWD`, `$ROOT` — use \
`/` too.",
    },
    Rule {
        name: "reserved tasks",
        rule: "`list`, `help`, `check`, `spec`, `completions` and `init` are subcommands, \
so a task may not take those names. `::` is reserved in a task name for include \
namespaces.",
    },
    Rule {
        name: "top-level statements",
        rule: "Top-level assignments are evaluated once, before the first task. No \
subcommand evaluates them, so `list` and `check` do no I/O and work even when a file a \
global reads is missing.",
    },
    Rule {
        name: "failure",
        rule: "Fail fast: a nonzero exit stops the task unless it is wrapped in `try` or being \
used as a condition. Output streams as it is produced rather than being buffered to the end.",
    },
    Rule {
        name: "portability",
        rule: "`check` flags non-portable commands — curl, wget, unzip, tar, cp, mv, rm, cat, \
shasum, test — and names the builtin that replaces each.",
    },
];

// ---------------------------------------------------------------------------
// the public reference
// ---------------------------------------------------------------------------

/// Every builtin command, in a stable order.
pub fn builtins() -> &'static [Builtin] {
    BUILTINS
}

/// One builtin by name, for `chore help <builtin>`.
pub fn builtin(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

/// Every builtin variable, in a stable order.
pub fn variables() -> &'static [Variable] {
    VARIABLES
}

/// Every statement form.
pub fn syntax() -> &'static [Form] {
    FORMS
}

/// Every condition form.
pub fn conditions() -> &'static [Condition] {
    CONDITIONS
}

/// The chaining and redirection operators.
pub fn chaining() -> &'static [Operator] {
    CHAINING
}

/// How a command name is resolved.
pub fn resolution() -> &'static [Rule] {
    RESOLUTION
}

/// The rules that surprise people.
pub fn rules() -> &'static [Rule] {
    RULES
}

/// The language version, which is the crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

/// A JSON value, only as much of one as this document needs. Building the
/// tree first keeps the printer — indentation, commas, escaping — in one
/// place instead of smeared across every section.
enum Json {
    Str(&'static str),
    Bool(bool),
    Arr(Vec<Json>),
    /// An object. A `Vec` rather than a map, because the key order is part of
    /// the output: `chore spec` must be diffable between builds.
    Obj(Vec<(&'static str, Json)>),
}

/// Render the reference as JSON: pretty-printed with two-space indent, in a
/// stable order, because `chore spec | less` is read by people about as often
/// as the output is parsed by tools.
pub fn json() -> String {
    let doc = Json::Obj(vec![
        ("version", Json::Str(version())),
        (
            "builtins",
            arr(BUILTINS, |b| {
                Json::Obj(vec![
                    ("name", Json::Str(b.name)),
                    ("usage", Json::Str(b.usage)),
                    ("summary", Json::Str(b.summary)),
                    ("description", Json::Str(b.description)),
                    ("effects", Json::Bool(b.effects)),
                    (
                        "flags",
                        arr(b.flags, |f| {
                            Json::Obj(vec![
                                ("name", Json::Str(f.name)),
                                ("argument", Json::Str(f.argument)),
                                ("default", Json::Str(f.default)),
                                ("meaning", Json::Str(f.meaning)),
                            ])
                        }),
                    ),
                ])
            }),
        ),
        (
            "variables",
            arr(VARIABLES, |v| {
                Json::Obj(vec![
                    ("name", Json::Str(v.name)),
                    ("values", Json::Str(v.values)),
                    ("meaning", Json::Str(v.meaning)),
                    ("scope", Json::Str(v.scope)),
                ])
            }),
        ),
        (
            "syntax",
            arr(FORMS, |f| {
                Json::Obj(vec![
                    ("name", Json::Str(f.name)),
                    ("syntax", Json::Str(f.syntax)),
                    ("example", Json::Str(f.example)),
                    ("meaning", Json::Str(f.meaning)),
                ])
            }),
        ),
        (
            "conditions",
            arr(CONDITIONS, |c| {
                Json::Obj(vec![
                    ("syntax", Json::Str(c.syntax)),
                    ("meaning", Json::Str(c.meaning)),
                ])
            }),
        ),
        (
            "chaining",
            arr(CHAINING, |o| {
                Json::Obj(vec![
                    ("symbol", Json::Str(o.symbol)),
                    ("meaning", Json::Str(o.meaning)),
                ])
            }),
        ),
        ("resolution", arr(RESOLUTION, rule)),
        ("rules", arr(RULES, rule)),
        (
            "reserved_tasks",
            Json::Arr(crate::RESERVED_TASKS.iter().map(|n| Json::Str(n)).collect()),
        ),
        ("namespace_separator", Json::Str(crate::NAMESPACE_SEP)),
    ]);

    let mut out = String::new();
    write_json(&doc, 0, &mut out);
    out.push('\n');
    out
}

fn rule(r: &Rule) -> Json {
    Json::Obj(vec![
        ("name", Json::Str(r.name)),
        ("rule", Json::Str(r.rule)),
    ])
}

fn arr<T>(items: &'static [T], f: impl Fn(&'static T) -> Json) -> Json {
    Json::Arr(items.iter().map(f).collect())
}

/// Write `value` at `depth`, assuming the caller has already indented the
/// opening position.
fn write_json(value: &Json, depth: usize, out: &mut String) {
    let pad = |out: &mut String, depth: usize| {
        for _ in 0..depth {
            out.push_str("  ");
        }
    };
    match value {
        Json::Str(s) => escape(s, out),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Arr(items) if items.is_empty() => out.push_str("[]"),
        Json::Arr(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                pad(out, depth + 1);
                write_json(item, depth + 1, out);
                out.push_str(if i + 1 == items.len() { "\n" } else { ",\n" });
            }
            pad(out, depth);
            out.push(']');
        }
        Json::Obj(fields) if fields.is_empty() => out.push_str("{}"),
        Json::Obj(fields) => {
            out.push_str("{\n");
            for (i, (key, val)) in fields.iter().enumerate() {
                pad(out, depth + 1);
                escape(key, out);
                out.push_str(": ");
                write_json(val, depth + 1, out);
                out.push_str(if i + 1 == fields.len() { "\n" } else { ",\n" });
            }
            pad(out, depth);
            out.push('}');
        }
    }
}

/// A JSON string literal. Only `"`, `\` and the control characters have to be
/// escaped; everything else, UTF-8 included, is written through unchanged.
fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            // The rest of C0, plus DEL's neighbours, have no short escape.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_quotes_backslashes_and_controls() {
        let mut out = String::new();
        escape("a\"b\\c\nd\te\u{1}", &mut out);
        assert_eq!(out, r#""a\"b\\c\nd\te\u0001""#);
    }

    #[test]
    fn empty_collections_stay_on_one_line() {
        let mut out = String::new();
        write_json(&Json::Arr(vec![]), 0, &mut out);
        assert_eq!(out, "[]");
    }
}

#[cfg(test)]
mod reserved_tests {
    use super::*;

    /// The rule text used to name four reserved tasks while the binary
    /// enforced six, so `chore check` rejected a task that `chore help` said
    /// was fine. Naming them in prose is worth it, but only if the prose
    /// cannot drift away from the list that is actually enforced.
    #[test]
    fn the_reserved_rule_names_every_reserved_task() {
        let rule = RULES
            .iter()
            .find(|r| r.name == "reserved tasks")
            .expect("a rule about reserved tasks");
        for name in crate::RESERVED_TASKS {
            assert!(
                rule.rule.contains(&format!("`{name}`")),
                "`{name}` is reserved but the rule does not mention it: {}",
                rule.rule
            );
        }
    }
}
