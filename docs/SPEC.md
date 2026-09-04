# chore

A single static binary that runs project tasks from a `chorefile`. It finds
`chorefile` in the working directory or the nearest parent, and runs tasks
through a built-in POSIX-sh-subset interpreter. It never spawns the host
shell, so behavior is identical on macOS, Linux and Windows (gnu and msvc).

## CLI

```
chore <task> [args...] [--dry] [--force]
chore list [--json|--names]         # tasks and descriptions
chore help [topic]                  # the language, or one builtin, statement or `files`
chore check                         # lint, unless a task takes the name
chore --check                       # lint, whatever the chorefile says
chore spec                          # full reference as JSON, for agents
chore completions [shell] [--write] # tab completion for task names
chore init                          # write a starter chorefile here
```

`list`, `help`, `spec`, `completions` and `init` are reserved task names.
`completions` joined that list after the others, so a chorefile that already
had a task of that name is now reported by the lint, and the subcommand
is what `chore completions` runs. `check` went the other way and left the list:
every Cargo project wants `task check { cargo check }`, and nothing depends on
the word the way completion scripts and tooling depend on `chore list`. So
`chore check` runs the task where the chorefile defines one and lints where it
does not, and `chore --check` is the lint either way — which is the spelling
for a script or a CI job, since it does not change meaning when someone adds a
task later.

Every form above that reads a chorefile also takes `--file <path>` before the
task name, and reads that file instead of discovering one.

### Files

| name | examples | what it is |
| --- | --- | --- |
| `chorefile` | `chorefile` | The project's tasks. Found by walking up from the working directory to the first file with exactly this name, lowercase, no extension; nothing else is ever discovered, and the directory holding it is `$ROOT`. `chore init` writes one. |
| `<name>.chore` | `rust.chore`, `release.chore`, `docker.chore` | A fragment of this project, named for what it covers and pulled in with `include rust.chore` or `include tasks/rust.chore`. Discovery never finds one, so it only means anything merged into the chorefile that includes it. `chore --file rust.chore` reads one on its own. |
| `<dir>/` | `include web`, `include libs as libs` | A directory in an `include` means the `chorefile` inside it: a subproject that also runs from inside, where `$ROOT` is that directory. |
| `.chore/` | `.chore/state` | State the `changed` builtin records, under `$ROOT`. Belongs in `.gitignore`. |

The same table is `chore help files`, the `files` block of `chore --help`, and
the `files` array of `chore spec`. The name is matched against the directory
listing rather than by opening the path, so `Chorefile` is not found on macOS
any more than on Linux, and the error names it: "`Chorefile` is here, but only
the exact lowercase name `chorefile` is read". A lone `ci.chore` gets the same
treatment, and an empty directory is pointed at `chore init`. An `include`
that misses says what it was reaching for: the `.chore` left off a name, a
directory that holds fragments but no `chorefile`, or the capital.

`--file <path>` reads the named file as written, whatever it is called, with
`$ROOT` at its directory. It is the one way to run or lint a fragment alone.
It is a usage error with `help`, `spec`, `completions` and `init`, which read
no chorefile: a flag that is silently ignored teaches that it does nothing.

- `--dry` echoes commands without side effects.
- `--force` disables run-once.
- `--check` lints without running. It stands alone: nothing runs, so `--dry`,
  `--force` and a task name beside it are usage errors rather than ignored.

### `list --names`

One task per line, `name<TAB>description`, in the same order as `chore list`.
It is the format a completion script reads: no padding to strip, no JSON, and
so no dependency on `jq`. A task with no description prints its name, the tab,
and nothing after it, so every line has the same two fields. The description is
the one line described under [Descriptions](#descriptions), never the whole
comment block.

### init

```
chore init
```

Writes a starter `chorefile` in the working directory and prints what it
wrote. If one is already there it refuses, says so, and exits 2: `init` never
overwrites. The starter uses only `echo`, so `chore check` on a fresh project
reports nothing.

### completions

```
chore completions            # what to add, and to which file
chore completions <shell>    # the script itself, on stdout
chore completions --write    # add it, once
```

The shells are `bash`, `zsh`, `fish` and `powershell`, which also answers to
`pwsh`.

Bare `chore completions` reads `$SHELL`, names the file to edit and prints the
line to put in it, and changes nothing. `$SHELL` is the login shell rather than
the running one, so it is a guess; naming a shell is how to override it. With a
shell named, the output is the script itself, for redirecting into a file or
into a package manager's completion directory.

`--write` does the edit: it appends the line, under a `# chore completions`
comment, to `~/.bashrc` or `~/.zshrc`, and prints what it changed. It is
idempotent, so a second run reports that the file already has it and leaves the
file alone. fish reads a directory rather than a startup file, so there
`--write` writes the script to `~/.config/fish/completions/chore.fish`.
PowerShell resolves `$PROFILE` for itself, and where it lands moves between
Windows, PowerShell 7 and the ISE, so chore will not guess it: `--write` says
it cannot and stops, and `chore completions` prints the line to add by hand.

The scripts embed no task list. They call `chore list --names` in whatever
directory the shell is standing in, and that finds the nearest chorefile the
way any other invocation does, so completion follows a person between projects
with nothing to configure per repository.

## GitHub Actions

```yaml
- uses: getchore/setup-chore@v1
- run: chore ci
```

Installs `chore` and puts it on `PATH` on Linux, macOS and Windows runners.
Pin the major tag: it moves with fixes, so a pinned patch goes stale in a
repository nobody is watching. `chore spec` reports the same under
`github_action`, so a tool does not have to know it from here.

Source and inputs: <https://github.com/getchore/setup-chore>

## Syntax

```
x=value                      assignment
$x  "$x/lib"                 interpolation
$(cmd)                       capture stdout, trimmed; nonzero fails unless `try`
if cond { } else if cond { } else { }
for x in a b c { }
for f in $(find src *.rs) { }        space-split
try cmd                      don't fail on nonzero
exit [code]                  end the whole run
return [code]                end the current task; the caller carries on
task name { }
task name arg1 arg2 { }      $1 $2, $@ all, $# count
include other.chore
include libs/chorefile as libs       tasks become libs::build
require 1.4.0                        the oldest chore that can run this file
$env::NAME                   an environment variable
dotenv .env [optional]       load a .env file
```

### Descriptions

A task's description is the **first line** of the contiguous block of `#` lines
directly above it. A block is a run of comment lines with nothing between them;
a blank line or any statement ends one, so a file header separated by a blank
line describes the file rather than the first task.

```sh
# Run the app under the debugger.
# In CI, where CI=true, skips that styling;
# else falls back to ad-hoc.
task run { ... }               # description: "Run the app under the debugger."
```

First line rather than last because a block that says more than one thing says
the summary first and the caveats after it, and a listing has room for one line.
A single comment above a task is its own first line, so the common case reads
the way it always did.

A blank `#` inside the block is skipped rather than read as a paragraph break:
the rule is the first **non-empty** line of the block, which needs no second
concept. A block of nothing but blank `#` lines leaves the task with no
description, since there was never a line to show.

That line is cut at the end of its first sentence, terminator kept:
`# Type-check the workspace. Runs clippy too, so it is slow.` lists as
"Type-check the workspace." A sentence ends at `.`, `!` or `?` followed by a
space or the end of the line, so `1.4.0` and `foo.bar` pass through, and a
period after a single letter is not an end, so `e.g. aarch64-apple-darwin`
stays whole.

The description is one line wherever it appears: `chore list`, `list --json`
and `list --names` all carry that one line, and the rest of the block stays in
the file for whoever opens it.

### Conditions

```
$a == $b     $a != $b     $a == ""
$a contains x    $a starts-with x    $a ends-with x
exists path      which name      changed path...
any command's exit code
!cond      cond && cond      cond || cond
```

### Chaining

```
a && b     a || b     a | b     a > f     a >> f     a 2> f     a 2>&1
```

#### `2>&1`

Send stderr wherever stdout is going. It is spelled exactly `2>&1` — no
spaces, nothing else after the `&` — and takes no filename, since it names a
stream rather than a place.

**Where "wherever stdout is going" is decided once, at the end.** After every
redirect on the command has been read, not while reading them:

```sh
cargo build > log 2>&1        # both streams into log
cargo build 2>&1 > log        # the same thing
x=$(cargo build 2>&1)         # both streams into x
cargo build 2>&1 | grep error # both streams into the pipe
cargo build 2>&1              # stdout is the terminal, where stderr already is
```

The second line is the one sh reads differently: there the dup happens where
it is written, while stdout is still the terminal, so `2>&1 > log` leaves
stderr on the terminal and puts only stdout in `log`. That order-dependence is
the thing everyone gets wrong about `2>&1`, and reproducing it would buy a
chorefile nothing.

Into a file, both streams share one open file — one handle, one offset — so
two lines written at the same moment do not land on top of each other. `2> f`
and `2>&1` on the same command is an error, in `chore check` as well as at run
time: it asks for stderr in two places at once, and picking one by the order
they were typed is exactly the guess above.

It works for a program on `PATH`, for a task (whose commands' diagnostics all
go the same way for as long as the call lasts), for a builtin, and for a
`script` block. On `spawn` it is accepted and means what a bare `> log`
already means there — see [spawn](#spawn).

### Word splitting

A quoted word is always exactly one argument. An unquoted `$var` splits on
whitespace, as in sh. There are no arrays and no quoting inside a variable, so
an argument containing a space must be written quoted at the call site:

```sh
if $OS == windows && $ENV == gnu { cmake -B build $flags -G "MinGW Makefiles" }
else                             { cmake -B build $flags }
```

### `return`

`return` ends the task it is written in and hands control back to whoever
called it, which carries on with its next statement. `exit` ends the whole run.
That is the entire difference, and it is what lets a task stop early without
taking its caller down:

```sh
task setup {
    if exists $bin/sona-$TRIPLE { echo "already in place"  return }
    download ...
}

# `setup` returns, and this task still gets to run
task dev {
    setup
    pnpm exec tauri dev
}
```

An optional code becomes the **task's** exit status, so `&&`, `||`, `try`, an
`if` condition and a `$( )` capture read it exactly as they read any other
command's: `return 1` is a task answering "no", not a failed run, and the
caller decides what that means. Left off, the code is 0. A nonzero return that
nothing handles stops the caller fail-fast, like any other nonzero command.

`return` is not a loop control. Inside a `for` it leaves the **task**, not the
loop; there is no `break`.

A task that printed something before returning still produced that value, so a
`$(task)` gets what it echoed, and run-once records it for the next capture.

In the task named on the command line there is no caller left, so `return`
ends the run, successfully unless it named a code. `return` outside a task is
a syntax error: there is no task to leave, and `exit` is what ends a run.

### script

The escape hatch. It hands a block of raw text to another interpreter on that
program's stdin.

```sh
version=$(script uv run - {
    import tomllib, pathlib
    print(tomllib.loads(pathlib.Path("Cargo.toml").read_text())["workspace"]["package"]["version"])
})
```

The command in front is expanded like any other, so `script $PYTHON -`
interpolates. **The block is not.** No variables, no escapes, no quoting rules:
a `$`, a backslash or a quote inside it means whatever the interpreter you
handed it to says it means. Chore values reach a block through the
environment, which the block's interpreter is spawned with:

```sh
env TARGET $TRIPLE
script uv run - {
    import os
    print(os.environ["TARGET"])
}
```

It composes like any other command — captured as above, piped, redirected,
joined with `&&`. The text arrives on stdin rather than in argv, which is what
lets `uv run -`, `python3 -`, `node -` and `nu --stdin` all work without chore
knowing anything about any of them, and keeps quoting out of it entirely.

**Where the block ends.** At the first line that begins with the indentation of
the line the `script` sits on, followed by `}`. Everything before it is the
body, whatever it holds: a dict closing on its own line, a `}` inside a string,
another language's braces — chore never looks inside.

The unit is the line, not the keyword's column, so nesting changes nothing: in
the capture above, `script` does not start its line and the block still closes
at the `})` written where `version=` was, which is the alignment you would have
used anyway. It costs one restriction — a body line may not be outdented as far
as its `script`'s line — and that is the price of chore never parsing the body.
The body starts on the line after `{`; nothing else may follow that brace.

The shared indentation is then removed, so a block indented inside a task
reaches Python as a program at column zero; relative indentation is kept
exactly.

**Everything chore does for you stops at the opening brace.** Under `--dry` a
block is skipped, so a captured one yields the empty string — the same as any
capture a preview could not evaluate. `check` reads
nothing inside a block — an undefined variable, a non-portable command or a
missing program in there is never reported — and `--dry` skips it rather than
running it, because nothing can say what it would do. That is the trade: the
rest of the language stays small enough to be checked and previewed, and the
work that needs a real language has somewhere to live that is not a separate
file. `check` says so once per file, so the hole is visible rather than
assumed.

Prefer an interpreter that behaves the same wherever it is installed — `uv`,
`python3`, `node`, `nu`. `script sh -` gets a warning of its own: `sh` is a
different program from platform to platform and Windows has none of them,
which is the thing chore exists to remove. The warning is guard-aware: put the
block behind `if $OS == macos || $OS == linux { ... } else { ... }` — the shape
the help text asks for — and it goes quiet, on every host.

### Task parameters

A declared parameter is **required** unless it carries a default. A default
makes it optional, and is written like an assignment:

```sh
# Cross-compile: CI passes a triple, a developer runs it bare
task setup target=$TRIPLE {
    rustup target add $1
    cargo build --release --target $1
}
```

The default is an ordinary word, so quoting, `$name` and `$( )` all work. It is
evaluated **at call time, in the called task's scope, and only when the caller
left the argument out**, so a `$( )` default costs nothing on a call that
supplies a value. It can read globals, the builtin variables, and the
parameters declared to its left (`task t a b=$1`), but not one to its right,
which is not bound yet.

**Required parameters come first.** Arguments are positional, so a required
parameter after an optional one could only be reached by supplying the optional
one too, and its default could never apply. The parser rejects the declaration
rather than letting it fail at every call site.

A default fills exactly one parameter even when it expands to several words: if
it could spill, `$2` would mean different things depending on what `$1`
happened to contain. An argument written at a *call site* still splits, because
there it is an argument.

Both kinds bind to `$1`, `$2`, …; a parameter's name is not a variable. `$#`
counts what the call bound, defaults included, and `$@` is that list, which is
how a task forwards itself to another.

An **empty** value written unquoted disappears rather than becoming an empty
argument, as in sh: with `$1` empty, `install.py $1 /usr/local/bin` passes two
arguments, not three, and the program reads the destination as its first one.
Quote it — `"$1"` — wherever the position is what the command counts. `--dry`
prints the argv that survived, which is where this is visible.

`env=` with nothing after it defaults to the empty string, exactly as the
assignment `env=` does. `env=""` says the same thing more clearly. "Nothing
after it" means nothing *touching* it: the default is the word written against
the `=`, so `task install force= bin=/usr/local/bin` declares two optional
parameters rather than giving `force` a default of `bin`. It is still an
optional parameter, so a required one may not follow it.

An error anywhere in a header names the parameter it came from:

```
in parameter `force` of task `install`: parameters are numbered from `$1`
```

The parser otherwise stops at the first token that is not a parameter and
reports a missing `{`, which is true and says nothing about which of four
parameters is the wrong one.

A task taking a variable number of arguments declares none and reads `$@`:

```sh
# Run the tests, passing any extra arguments through
task test {
    cargo test --workspace $@
}
```

## Resolution

task → builtin → `PATH`. A leading `^` forces `PATH`: `^find`.

## Environment

```sh
require 1.9.0
dotenv .env                      # error if it is missing
dotenv .env.local optional       # skipped if it is missing

registry=$env::REGISTRY          # a global may read one

task deploy {
    dotenv deploy/.env.prod      # this call only
    ./deploy $env::REGION
}
```

### `$env::NAME`

Reads an environment variable, anywhere a `$name` can be written, quoted
(`"$env::HOME/bin"`) or braced (`${env::HOME}`) like any other.

It is read through the interpreter's own environment first and the process
environment second, so it sees everything the chorefile put there —
`env NAME value`, `env NAME=value <cmd>`, a `dotenv` — and falls back to what
`chore` was started with. There is no way for the two to disagree: `$env::CI`
and `env CI` answer from the same place.

**An unset name is a run error**, like any other undefined variable, because
it is a name in the middle of a command line and an empty one silently builds
a wrong path. Where a name may legitimately be absent, use the forms whose
miss is an *answer*:

```sh
if env GITHUB_TOKEN { download gh://... }
token=$(try env GITHUB_TOKEN)
```

`env` is a **reserved namespace**. `include x as env` is an error — it would
make `$env::PATH` mean one thing in one file and another elsewhere — and
`env::X=...` is not an assignment; the message points at `env X <value>`,
which is the statement that actually sets one. Nothing else changes about
`::`: it still joins an include's namespace to a name, and `$libs::dist` is
still something only an include can construct.

`check` treats every `$env::NAME` as defined. Whether this machine has it set
is a fact about the machine, not about the chorefile, and a finding that
disappears in CI is not a finding. The other direction is worth help, though:
a bare `$FOO` that is undefined *and* set in the environment `check` is
running in gets told so — "`FOO` is set in the environment: write `$env::FOO`"
— instead of a did-you-mean.

Under `--dry` it reads the real environment and yields a real value, not one
of the marked values a preview invents: nothing had to be skipped to answer
it.

### `dotenv`

Loads a file of `KEY=value` lines. **A name that is already set wins over the
file** — the process environment, an earlier `env`, an earlier `dotenv` — so
`REGION=us-east-1 chore deploy` and a variable a CI job exports still override
a `.env` that is checked in. Load order is therefore the whole of the
precedence rule: the first file to name a variable is the one that answers,
and a later file only fills in what nothing had.

`optional` is the literal word after the path, and means a missing file is
skipped. Without it a missing file fails the run — which is what makes
`dotenv .env` a statement about what the project needs rather than a wish.

**At the top level** it is a directive, like `include`: the path is a literal
and resolves relative to the file that wrote it, so a `dotenv .env` in
`libs/rust.chore` means `libs/.env` wherever `chore` was invoked from. Files
load once, before the top-level assignments — so a global may read
`$env::NAME` — and after `require` is checked. An included file's `dotenv`
loads too, after every one written in the file that included it, which is the
order that keeps a subproject's `.env` from deciding what the project runs
with.

`list`, `help`, `check` and `spec` never load one, for the same reason they
never evaluate a global: they need the parse tree and nothing else, so they
still work on a checkout where every `.env` is gitignored and absent.

**Inside a task** it is a builtin. The path is relative to the task's current
directory, like every other builtin path, and what it binds lasts exactly as
long as `env NAME value` would — the task, everything it calls, every process
spawned inside it — and is gone when the task returns.

Under `--dry` it loads. Reading a file is an input, not an effect, and a
preview whose `if $env::REGION == eu` answered from the developer's own shell
would be describing a different run.

### The file format

```sh
# a comment
export API_URL=https://example.test    # `export` is allowed, and ignored
PORT=8080            # a trailing comment, after whitespace
COLOR=#ff0000        # a `#` touching the value belongs to it
LITERAL='no \n escapes here'
TEXT="a\tb\nc"       # \n \t \\ \" only
EMPTY=
```

Blank lines and `#` lines are skipped. A key must be a name — a letter or `_`
followed by letters, digits and `_` — and spaces around the `=` are allowed. A
bare value is trimmed and loses a trailing ` # comment`; a single-quoted value
is literal; a double-quoted value takes `\n`, `\t`, `\\` and `\"` and nothing
else, so a Windows path keeps its backslashes. CRLF line endings are fine.

**There is no `${OTHER}` expansion.** The dialects disagree about what it
reads, what an unset name expands to and how to escape the `$`, and a
chorefile has a language of its own for building a value out of another. A `$`
in a value is a `$`.

A malformed line is an error naming the file and the line, never a line
quietly skipped: a run that ignored `DATABASE_URL "postgres://..."` would fail
later with nothing to connect the two. On Windows, names match
case-insensitively, the same rule `env` follows.

## Builtin variables

Read-only, always set.

```
$OS        macos | linux | windows
$ARCH      x86_64 | arm64
$ENV       gnu | msvc | ""
$PLATFORM  $OS-$ARCH
$TRIPLE    rustc target triple, e.g. aarch64-apple-darwin
$EXE       "" or ".exe"
$HOME      user home
$ROOT      dir containing the top-level chorefile
$CWD       current dir
$TASK      name of the running task
$NOW       ISO timestamp
```

## Builtin commands

Reserved; a task may not shadow one.

```
download <url> <dest> [--retries n] [--timeout s] [--sha256 h]
                              http(s), and gh://owner/repo/tag/asset
extract  <archive> <dest> [--member name] [--strip n] [--flatten]
                              zip, tar, .gz .xz .zst
archive  <src...> <dest>      format from the extension; `src/` packs contents
copy     <src> <dest>         file or dir
move     <src> <dest>
remove   <path...>            recursive, no error if missing
mkdir    <path...>            -p semantics
chmod    <mode> <path>
which    <name>               prints path, or exits 1
find     <root> <name...>     every match, recursive, one per line
read     <file>               prints contents, trimmed
write    <file> <text>        `>>` appends
sha256   <file>
exists   <path>               exit 0/1, for `if`
changed  <path...>            exit 0/1, for `if`; records what it saw
echo     <text...>
env      <NAME> [value]       get, or set for the rest of the call
env      NAME=value <cmd>     set for one command only
dotenv   <path> [optional]    load a .env file for the rest of the call
fail     <msg>
sleep    <seconds>
spawn    <cmd> [args...]      start a program, do not wait; see below
parallel [--fail-fast] <task>...
                              run tasks at once; see below
```

Everything else resolves on `PATH`.

### Details that are easy to guess wrong

**download.** A `dest` ending in `/`, or naming a directory that exists, means
"into this directory, under the remote filename"; anything else is the output
path. Defaults are `--retries 3` and `--timeout 60` (whole request). 5xx and
429 are retried, other 4xx are fatal. `GITHUB_TOKEN` or `GH_TOKEN`, when set,
is sent as a bearer token, which is what makes `gh://` work against a private
repo. The download lands in a temp file and is renamed into place, so an
interrupted transfer never leaves a truncated file that looks complete, and a
failed `--sha256` leaves nothing behind.

**extract.** Also reads `.tgz`, `.txz`, `.tzst` and `.lzma`, and sniffs the
leading bytes when the name says nothing, so an archive with a lying
extension still unpacks. `--member` matches an entry's full path or just its
filename, so `--member chore` finds `bin/chore`. On a compressed *single* file
(not a tar), `--member` and `--strip` are an error. An entry with an absolute
path, or one that climbs out of `dest`, aborts the extraction.

`--flatten` writes every entry directly into `dest` under its base name,
discarding the directory path it had inside the archive; directory entries are
dropped, since a flattened tree has none. It exists for `--member`: without it,
`extract out.tar.gz got --member sona` lands `got/pkg/bin/sona`, a path you
cannot predict without opening the archive first, and every such call has to be
followed by a `move`. Two entries that would flatten to the same name is an
error naming both. The alternative is a run that succeeds and whose result
depends on the order entries sit in the archive. `--strip` and `--flatten`
cannot be combined, since `--flatten` already drops every directory.

**archive.** The last argument is the destination, and its extension picks the
format: `.zip`, `.tar`, `.tar.gz` or `.tgz`, `.tar.xz` or `.txz`, `.tar.zst` or
`.tzst`. Anything else, a bare `.gz` included, is an error. Each source's own
name becomes a top-level entry, as `tar cf` and `zip -r` do, so extracting
gives the directory back rather than its contents loose. A source written with
a trailing `/` contributes its **contents** instead, with no directory of its
own, the same reading of the slash that `extract` gives its `dest`. Several
sources pack side by side into one archive, and two of them claiming the same
top-level name is an error. Entries are sorted within each source, and the
sources themselves appear in the order they were written, so the same command
over the same tree packs to the same bytes twice.

```sh
archive pkg  dist/pkg.tar.gz     # pkg/lib/..., pkg/include/...
archive pkg/ dist/pkg.tar.gz     # lib/..., include/...
archive sona ffmpeg dist/bundle.tar.gz   # two entries at the root
```

**remove** refuses the filesystem root and `$ROOT` itself, comparing resolved
paths. Anything below `$ROOT` is fair game. A typo should not wipe a machine.

**find** requires `<root>` to be a directory. Patterns support `*` and `?` and
match the filename only, never the path. Matching is case-sensitive on every
platform, so a chorefile cannot come to depend on a case-insensitive
filesystem. Symlinked directories are not descended into, which is what keeps
a cycle from hanging a run. Output is relative to `<root>` as written, so it
feeds straight back into another command.

**write** ends the file with a newline, and `read` trims it back off, so a
read/write round trip is lossless and successive `>>` lines stack.

**changed** answers "did any of these paths change since the last time this
task asked?". Exit 0 means yes, and it records the new state, so the next run
sees "no"; exit 1 means every path is unchanged, and nothing is recorded. A
file is hashed by its contents, a directory recursively over its entries in
sorted order, with each entry's path in the hash as well as its bytes, so a
rename is a change even when no byte moved. A path that does not exist counts
as changed and is recorded as missing, which makes a delete a change and a
re-creation a change again. Symlinked directories are not descended into, the
same rule `find` follows.

The record lives in `$ROOT/.chore/state`, one line per record, keyed on the
calling task **and** the exact argument list, so two tasks watching the same
paths keep separate answers and one task can hold several. Delete the file to
force everything to rebuild; `.chore/` belongs in `.gitignore`, since the
state describes one machine's tree. The file starts with a version line and a
run that does not recognise it starts from scratch, which costs one extra
build and never a wrong answer.

`--force` reports changed without consulting the state, since a forced run is
a request to do the work anyway, and it records what it saw. `--dry` reads
the state but never writes it: a preview that recorded would leave the next
real run skipping work that was only ever previewed.

```sh
task build {
    if changed src Cargo.toml {
        cargo build --release
    }
}
```

**which**, **exists** and **env <NAME>** report a miss as exit 1 rather than a
hard failure, which is what lets them drive an `if`. Under fail-fast a bare
`which foo` still stops the task. Put it in a condition or a `try`.
**env NAME value** sets the variable for the rest of the *call* — the task
that set it, everything that task calls, and every process spawned inside it —
and it is gone when the task returns. That is the scope `cd` and a local
already have, and it is why a `run` task that sets `TERRA_SOCKET` no longer
sets it for whatever runs after it. Chore never changes its own process
environment: the bindings are layered onto each child as it is spawned, and
onto the builtins that read the environment, so `env HTTPS_PROXY ...` reaches
the `download` on the next line.

**env NAME=value \<cmd\> [args...]** is the per-command form — the shell's
`FOO=x cmd`, which a chorefile cannot write because `FOO=x` at the start of a
statement is an assignment. The leading `NAME=value` words are the bindings and
what follows is a command, resolved task → builtin → `PATH` like any other; a
task called this way keeps the bindings for its whole call. A `^` may only
prefix a statement's command name, so it cannot appear here. **If the first
argument contains an `=`, it is this form**, which is what keeps
`env NAME value` and `env NAME` meaning exactly what they always did.
`env NAME=value` with no command is an error.

```sh
task build {
    env CGO_ENABLED=0 go build ./...
    go test ./...             # CGO_ENABLED is whatever it was
}
```

To read a variable into a *word* rather than onto stdout, write `$env::NAME`;
`dotenv` fills the same environment from a file. Both are in
[Environment](#environment).

**sleep** accepts fractional seconds.

**chmod** on Windows can only reach the read-only flag: the owner-write bit
clears or sets it and every other bit, execute included, is ignored, because a
Windows file is executable by extension rather than by mode.

**cd** is not in this list. It changes the interpreter's directory rather than
the process's, so it is handled before builtin and `PATH` lookup, and `^` does
not apply to it.

**$TRIPLE** is the rustc target triple for the host, spelled the way `rustc
-vV` and `cargo --target` spell it. It is not derivable from `$OS` and `$ARCH`
(`linux` alone cannot say `gnu` or `musl`, and `windows` alone cannot say
`msvc` or `gnu`), which is why chore reports it rather than leaving every Rust
chorefile to write the same mapping table. Use `$PLATFORM` for naming your own
release artifacts and `$TRIPLE` for anything handed to a toolchain. On a target
chore has no triple for it is empty: a guessed triple is accepted by cargo and
then builds the wrong thing, where an empty one fails somewhere you can see it.

**$HOME** is empty when neither `HOME` nor `USERPROFILE` is set.

### spawn

```sh
task dev {
    cargo build
    spawn ./target/debug/app > app.log
    echo "app restarted; logs in app.log"
}
```

The replacement for `nohup ./app > log 2>&1 &`. It starts the program and
returns immediately, and the child outlives the run: on Unix it gets a process
group of its own, on Windows `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`, so
closing the terminal that ran `chore dev` does not take the server with it.
The pid is reported on stderr: `spawned ./target/debug/app (pid 41207)`.

**A program, and only a program.** `spawn` resolves on `PATH` — never a task,
never a builtin — because what it starts has to keep running after chore has
exited, and both of those *are* chore. `spawn build` is an error, and `chore
check` says so before the run gets that far.

**Its output.** stdin is null, and so are stdout and stderr unless the
statement redirects them: a process that outlives the run must not be writing
to a terminal chore has handed back. `> log` and `>> log` take **both**
streams — `> log 2> log` would be two handles racing for one file — so a bare
`>` means "everything this thing says goes here". A `2> err` beside it splits
them again, and a `2> err` on its own keeps the errors and drops the rest.

`spawn ./app > log 2>&1` is accepted and means exactly what the bare `> log`
already meant, down to the same code path: the file is opened once and the
child's stderr is a clone of its stdout. It is spelled out because
`nohup ./app > log 2>&1 &` is the line people arrive with, and there is
nothing to gain from making them delete half of it.

Under `--dry` the line is echoed and nothing is spawned; no redirect file is
opened either, so a preview cannot truncate yesterday's log.

Nothing waits for the child and nothing reports on it later: its exit code is
not the run's, and a `spawn` that started something which dies a second later
still succeeded. When the result matters, run the program instead.

### parallel

```sh
task ci {
    parallel lint test installers
}
```

Runs the named **tasks** concurrently, one thread each, waits for all of them,
and fails if any of them failed. Its arguments are task names, never commands:
`parallel cargo test` is an error, and `chore check` says so before the run
gets that far.

**Run once still means once.** The run-once record is shared by every sibling,
so a task two of them call runs one time and the second waits for the first
and gets its result. `parallel build test` where both call `deps` runs `deps`
once, even when the two calls land in the same instant: a task is claimed
before its body starts, not after it ends, so the second caller finds the
claim and blocks on it rather than starting a second copy. A capture works the
same way across siblings: the value `$(platform-id)` recorded in one is
replayed in the other. `--force` switches all of this off, as it does
everywhere else.

**Output does not interleave.** Each task's output is collected into a block
of its own and the blocks are printed once everything has finished, in the
order the tasks were named, not the order they finished. `parallel lint test`
prints exactly what `lint` then `test` would have printed: concurrency changes
the timing, not the transcript. A task's own stdout and stderr keep separate
blocks, so the interleaving between those two streams is the one thing that is
not preserved. A shared task's output appears in the block of whichever
sibling actually ran it, since that is where the work happened; the sibling
that waited shows the call and not the work. Nothing appears while the tasks
are running, which is the price of never having to read two builds woven
together line by line.

**Every failure is reported.** By default each task runs to its end and all of
the failures are named on stderr, so one run tells you about all of them; the
call then fails with the first failing task's exit code, first in the order
the tasks were written, so the code does not depend on which thread lost.
`try parallel ...` swallows it like any other nonzero command.

**--fail-fast** stops as soon as a task fails. The siblings already running are
not killed: a thread cannot be interrupted safely, and killing one in the
middle of a `download` or an `extract` would leave a half-written tree behind.
Each sibling instead stops **before its next statement**, so a command already
running finishes and nothing after it starts. A task stopped this way is
reported as stopped rather than failed, and it is *not* recorded as having
run, so a later call to it does the work rather than believing it is done.

**exit** in a task means what it means anywhere else: it ends the whole run,
not just that task. It cannot unstart the siblings, so it takes effect once
they have all finished and their output has been printed. `return` ends only
the task that wrote it, and its code becomes that task's status.

**cd**, locals and `env` are per-call, as ever. A task starts in the directory
the `parallel` was called from, and with the environment the `parallel` was
called with, exactly as it would have if it had been called directly; its own
`cd` and its own `env NAME value` die with it. Siblings cannot see each other's
sets — each has its own interpreter and its own copy — so two of them binding
the same name is not a race. `$ROOT`, `$NOW` and the globals are facts about
the invocation and are shared.

**--dry** previews the tasks one after another instead of running them
concurrently. A preview describes the work, and the same work is described
either way; running it concurrently would only add a preview whose captures
and conditions raced each other.

## Runtime

- Echo each command before running it.
- Fail fast; stream output as it is produced.
- Paths are written with `/` everywhere and converted on Windows.
- argv is passed to the OS directly. Nothing is re-quoted or re-expanded.
- A task runs once per invocation, keyed on **name and arguments**, so a
  parameterised task called twice with different arguments still runs twice.
  `--force` disables this.
- When a task's output is captured (`$(task)`, `task | cmd`, `task > file`),
  the value it printed is remembered, and a later capture of the same task with
  the same arguments gets that value back without running the body again. So a
  task can serve as a function: `platform=$(platform-id)` answers the same
  thing every time it is asked, and the body still executes exactly once. A
  task whose output was only ever streamed to the terminal has no remembered
  value, so capturing it afterwards runs it again. An empty string would
  otherwise be interpolated into a path.
- `cd` changes the interpreter's directory, not the process's, and
  `env NAME value` changes the interpreter's environment, not the process's.
  Both last for the call that made them.
- `parallel` runs tasks concurrently and shares that record with them, so a
  task two siblings call still runs once.

### `--dry`

Echoes commands and skips the ones with effects. `fail` still fails, since a
preview that swallowed a hard stop would describe a run that cannot happen.
`env NAME value` is not skipped: it sets nothing outside the run, and a preview
that carried it out is a preview whose later `if env NAME` tells the truth
about the chorefile rather than about the shell it was previewed from. `dotenv`
is not skipped either — reading a file is an input, not an effect — and
`$env::NAME` reads the real environment, so it is never one of the invented
values below.
**Captures and conditions still run**: a `$(...)` that did not execute would
leave every interpolated path downstream empty, and the preview would describe
a run that could never happen.

Because the effects are skipped, a read-only command sees the world *before*
the run, not the one the recipe builds as it goes. `find build/` runs before
the `mkdir build` that would create it. Such a command is reported on stderr
and treated as a command that exited nonzero; it does not stop the preview. A
statement moves on, `&&` and `||` branch as usual, and a `$(...)` yields the
empty string. A preview is a preview of the commands, not a claim that the run
would succeed.

A condition is believed only when its command actually **answered**. Any
command that *fails* inside an `if` condition leaves the condition undecided,
and an undecided condition previews the `then` branch, since previewing the
work beats previewing nothing. The rule is positional, not per-builtin:
`exists`, `which`, `env <NAME>` and `changed` are the only builtins that
cannot fail. A miss is their answer, a nonzero exit, so
`if exists build/version.txt`
previews `else` while `if read build/version.txt` previews `then`. The choice
is made over the condition as a whole, so a failure anywhere inside `&&`, `||`
or `!` still previews `then`, and `!` has no truth value to flip. A program on
`PATH` that ran and exited nonzero answered, and is believed; one that could
not be spawned did not.

A capture the preview could not evaluate leaves something behind.
`size=$(script uv run - { ... })` binds the empty string, so `if $size == ""`
is a perfectly **decidable** comparison — nothing failed at the `if`, and the
undecided rule never fires. The preview would walk into `fail "wasm missing"`
and report a problem that exists only because it declined to look.

It does not take a different branch: the value does not exist, so every verdict
about it is a guess. It **explains the decision instead**. A variable assigned
from something `--dry` could not evaluate remembers what is missing, reading it
marks whatever is computed from it — through a second assignment, a loop
variable, and a task's `$1` and `$@` — and a condition that reads one prints a
note on stderr beside the branch it took:

```console
--dry: took the `then` branch on `$size`, a value this preview invented because
it could not evaluate `script uv run - { ... }`; a real run may go the other way
```

Notes go to stderr, never stdout, since stdout may be a capture's value, and
each distinct note is printed once per run — a decision inside a `for` body is
explained once, not once per iteration. An ordinary assignment over the
variable clears the mark. `fail` still aborts, but one reached from inside a
branch chosen this way says so. None of this exists in a real run.

### Top-level statements

Top-level assignments are evaluated once, before the first task, and a
top-level `dotenv` is loaded once before *them*, so a global may read
`$env::NAME`. `list`, `help`, `check` and `spec` never evaluate an assignment
or load a `dotenv`, since they only need the parse tree, so `chore list` does
no I/O and works even when a file a global reads — or a `.env` — is missing.

## require

```sh
require 1.4.0
```

The oldest `chore` that can run this file. Top level only, conventionally the
first line, and at most one per file. It means "at least this": a chorefile
that uses a 1.2.0 feature says `require 1.2.0` and runs on 1.2.0 and anything
after it.

The version is a bare `major.minor.patch` and nothing else. `v1.4.0`, `1.4`,
`^1.4.0` and `1.4.0-rc1` are all syntax errors that name the shape they should
have had. There are no ranges and no operators, because a floor is the only
question a task runner has to answer; components are compared as numbers, so
1.10.0 is newer than 1.9.0.

An included file may state its own `require`, and every one is checked. A run
reports the strictest failure, since that is the version that satisfies all of
them at once, and names the file that asked for it. `chore check` reports each
one as an error, with its line and column.

The check happens before any task runs and before top-level assignments are
evaluated. `chore list` warns on stderr and still prints the list: it exists
partly to answer "what is here", which an old binary can still answer, and its
stdout is unchanged. `chore help`, `chore spec`, `chore init` and `chore
completions` read no chorefile and are unaffected.

A `chore` older than `require` itself does not know the keyword and reports it
as an unrecognised top-level statement. Nothing can change that after the
fact, so that message says a newer `chore` may be needed.

## include

```sh
include tasks/rust.chore           # a fragment of this project
include website as web             # a subproject, as `web::build`
```

### Fragment or subproject

Two different things can be included, and the **filename** is what says which.
`chore` finds its chorefile by walking up from the working directory to the
first file named exactly `chorefile`, and nothing else is ever discovered.

A **fragment** is a piece of this project that only means anything merged into
it. Name it `something.chore`. Discovery never finds it, so there is one
project and one `$ROOT` however you reach it.

A **subproject** has its own `chorefile`, and is standalone on purpose: its own
package manager, its own lockfile, worth running from inside. `cd website &&
chore build` then works, and `$ROOT` is `website/` for that run.

That second one is a real choice with a consequence, so make it deliberately: a
task reading `download vendor/thing` puts the file under the project root when
run from the root, and under `website/` when run from there. Neither is wrong,
and nothing in between tells you which happened. If the directory is not
genuinely its own project, give the file the `.chore` extension, named for
what it covers — `website/web.chore` — and the question never comes up.
`check` warns when an `include` points at a `chorefile` inside your
project, for exactly this reason. It is a warning, once per include, and it
says both halves: how to make the question go away, and that keeping it is a
legitimate choice when the directory really is its own project.

- Paths resolve relative to the **including file**, not the working directory.
  A directory argument means the `chorefile` inside it.
- `$ROOT` stays the top-level chorefile's directory in included files. One
  root per invocation, so a `download ... third_party/` in an included file
  lands where the project's author expects rather than beside that file.
- `as` namespaces **both tasks and globals** (`libs::build`). Without `as`
  everything merges flat, and a duplicate name, task or global, across two
  files is an error.
- `::` is reserved in task names. A cycle names the whole loop and is an
  error.
- `as env` is an error: `$env::NAME` reads the environment in every file, so
  the namespace cannot also be an include's.
- A `dotenv` in an included file loads too, relative to *that* file, after
  every `dotenv` in the file that included it. See
  [Environment](#environment).

### What `as` renames, and what it leaves alone

`as ns` is applied to a whole subtree, meaning the included file and everything
it included, once that subtree has resolved. It renames the definitions, and
inside that subtree's own bodies it renames every reference that **resolved
within it**: a command whose name is a single literal word naming one of the
subtree's tasks, and a `$x` naming one of its globals, wherever they appear.

Everything else is left bare, which is what lets an included file still reach a
builtin, a program on `PATH`, or, under a flat merge, a name its includer
defines. Bare names are resolved late, against the merged table.

Two consequences worth stating:

- **A computed command name is not rewritten.** `$cmd` or `"$prefix-build"` is
  not a literal, so there is nothing in the tree to rename; the interpreter
  expands the word before it looks a task up. A namespaced file that dispatches
  through a variable has to spell its own namespace.
- **A name a task assigns anywhere in its body is local for the whole body**,
  and is never rewritten to a global. This matches the interpreter, where an
  assignment inside a task writes a frame-local. The cost is that a task which
  reads a namespaced global and *later* assigns the same name reads its own
  local throughout. The alternative would make the meaning of `$x` depend on a
  line further down, which is worse.

Because `as` namespaces globals too, `$libs::dist` is a variable name that no
chorefile can write; only an include can construct it.

### Order of included globals

Includes are followed depth-first in source order, and each file's own
assignments are evaluated **after** everything it included. An including file
can therefore build on what it pulled in (`toolchain=$libs::default`), while an
included file cannot name its includer and has nothing to gain from running
later.

Shadowing is not a concern in either order: a flat merge makes a duplicate
global an error, and `as` gives it a different name. The order only decides
what a global can read.

### Paths, twice-included files, and cycles

An include path is taken literally: `include libs` names a file or directory
called `libs`, and no `.chore` is guessed, so what a chorefile means cannot
change with what happens to exist on disk. A directory means the `chorefile`
inside it.

Including the same file twice on different branches is not a cycle and is not
deduplicated. Flat, it fails as a duplicate name; under two different `as`
namespaces, its tasks exist twice under two prefixes, which is what asking for
them twice means. A true cycle, a file that includes itself either directly or
through others, is an error naming the whole loop.

### Which chorefile answered

`chore` uses the nearest file named `chorefile` at or above the working
directory, so a project with one in a subdirectory has more than one, and which
answers depends on where you stand. `chore list` says which it used, on a line
above the tasks:

```
using ../../chorefile, $ROOT = /Users/me/repo
```

The chorefile is spelled relative to the working directory, which is what tells
you at a glance that you walked up; `$ROOT` is absolute, because it is what
every relative path in the run resolves against, and a relative `$ROOT` would
print `.` from both the project root and a subproject. The line is always
printed, and bare `chore` prints it too.

`chore list --json` is an array of tasks and stays one — a tool reads each
task's own `file` field. `chore list --names` is unchanged.

## check

`chore --check` always lints. `chore check` lints too, unless the chorefile
defines a task named `check`, in which case it runs the task — the name is not
reserved.

Builtins are reserved by convention, not by the interpreter: at runtime a task
wins over a builtin of the same name, and it is the lint that reports it. The
same is true of a task named after a reserved subcommand. `chore list` is
always the subcommand, so the task is unreachable.

Reports syntax errors, reserved names, unknown commands, undefined variables
(in a parameter's default as much as in a body), duplicate names, a parameter
declared twice in one header, a parameter read as `$name` where parameters are
positional, include cycles, an `include` pointing at a file named `chorefile`
inside the project, an unmet `require`, and non-portable commands (`curl`,
`unzip`, `tar`, `cp`, `rm`, `nohup`) with the builtin that replaces them. A
`spawn` whose first word names a task or a builtin is an error, and one that
names a program this machine has never heard of is a warning like any other
`PATH` miss. A `dotenv` path built from a variable or a capture is an error,
and so is a misspelt `optional`, which is named; a *missing* `.env` is not a
finding, since it is gitignored and absent on every clean checkout — that is
what `optional`, and the run, are for. A default is checked in the
scope it will be evaluated in, so it may read `$1`…`$(n-1)` but not its own
slot or a later one.

### Platform guards and `PATH`

`check` looks each command up on the `PATH` of the machine it runs on, and
reports a miss as a warning. Inside an `if` whose condition this machine's
platform decides against, it does not: a command in a branch the host never
enters cannot be expected to exist there, and its absence is not a fact about
the chorefile.

```sh
task dylib {
    if $OS == windows && $ENV == gnu {
        gendef $dist/foo.dll      # not looked up on a macOS or Linux host
        dlltool -d $dist/foo.def
    } else {
        echo nothing to do        # reachable here, so still checked
    }
}
```

A condition counts as decided only when every operand is literal text or one of
the read-only platform variables (`$OS`, `$ARCH`, `$ENV`, `$PLATFORM`, `$EXE`)
combined with the comparison operators, `!`, `&&` and `||`. Nested `if`s and
a `for` body inherit the guard around them. Anything else keeps the warning: a
command's exit code, a `$( ... )` capture, a task argument, a global, and a
platform name the chorefile has assigned over are all treated as unknown, and
an unknown condition is never taken as proof that a branch is skipped.

Only the `PATH` lookup is affected by *this machine's* platform. An undefined
variable and a non-portable command are wrong on every platform, so `check`
reports them inside a platform guard exactly as it does outside one.

The `script sh -` warning reads the same guards, but asks the other question:
not "does this host enter the branch" but "which platforms does the chorefile
still allow here". A shell block guarded off every platform the shell is
missing from is silent — on every host, since that is a fact about the file:

```sh
if $OS == macos || $OS == linux {
    script sh - {            # silent: windows is excluded
        ./configure && make
    }
} else {
    fail "no shell here"
}

if $OS == windows {
    script sh - {            # still warned about: windows has no `sh`
        ./configure && make
    }
}
```

Only `$OS` narrows it, plus the `$EXE` and `$ENV` values `$OS` alone fixes;
`$ARCH`, `$PLATFORM` and anything the chorefile assigned are unknown here and
narrow nothing, so an unreadable guard silences nothing.

Warnings do not fail the run: `chore check` exits nonzero only for errors, so a
tool that exists solely in CI does not break the gate.

## Out of scope for v1

Plugins, dependency syntax, arithmetic, arrays, globs beyond `find`, `while`,
functions.
