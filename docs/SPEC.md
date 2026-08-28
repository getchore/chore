# chore

A single static binary that runs project tasks from a `chorefile`. It finds
`chorefile` in the working directory or the nearest parent, and runs tasks
through a built-in POSIX-sh-subset interpreter. It never spawns the host
shell, so behavior is identical on macOS, Linux and Windows (gnu and msvc).

## CLI

```
chore <task> [args...] [--dry] [--force]
chore list [--json|--names]         # tasks and descriptions
chore help [builtin]                # syntax and builtins, or one builtin
chore check                         # lint without running
chore spec                          # full reference as JSON, for agents
chore completions [shell] [--write] # tab completion for task names
```

`list`, `help`, `check`, `spec` and `completions` are reserved task names.
`completions` joined that list after the others, so a chorefile that already
had a task of that name is now reported by `chore check`, and the subcommand
is what `chore completions` runs.

- `--dry` echoes commands without side effects.
- `--force` disables run-once.

### `list --names`

One task per line, `name<TAB>description`, in the same order as `chore list`.
It is the format a completion script reads: no padding to strip, no JSON, and
so no dependency on `jq`. A task with no comment above it prints its name, the
tab, and nothing after it, so every line has the same two fields.

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
```

The comment line directly above a `task` is its description.

### Conditions

```
$a == $b     $a != $b     $a == ""
$a contains x    $a starts-with x    $a ends-with x
exists path      which name      any command's exit code
!cond      cond && cond      cond || cond
```

### Chaining

```
a && b     a || b     a | b     a > f     a >> f     a 2> f
```

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

`env=` with nothing after it defaults to the empty string, exactly as the
assignment `env=` does. `env=""` says the same thing more clearly.

A task taking a variable number of arguments declares none and reads `$@`:

```sh
# Run the tests, passing any extra arguments through
task test {
    cargo test --workspace $@
}
```

## Resolution

task → builtin → `PATH`. A leading `^` forces `PATH`: `^find`.

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
echo     <text...>
env      <NAME> [value]       get or set
fail     <msg>
sleep    <seconds>
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

**which**, **exists** and **env <NAME>** report a miss as exit 1 rather than a
hard failure, which is what lets them drive an `if`. Under fail-fast a bare
`which foo` still stops the task. Put it in a condition or a `try`.
**env NAME value** sets the variable for the rest of the run, including
spawned children.

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
- `cd` changes the interpreter's directory, not the process's.

### `--dry`

Echoes commands and skips the ones with effects. `fail` still fails, since a
preview that swallowed a hard stop would describe a run that cannot happen.
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
`exists`, `which` and `env <NAME>` are the only builtins that cannot fail. A
miss is their answer, a nonzero exit, so `if exists build/version.txt`
previews `else` while `if read build/version.txt` previews `then`. The choice
is made over the condition as a whole, so a failure anywhere inside `&&`, `||`
or `!` still previews `then`, and `!` has no truth value to flip. A program on
`PATH` that ran and exited nonzero answered, and is believed; one that could
not be spawned did not.

### Top-level statements

Top-level assignments are evaluated once, before the first task. `list`,
`help`, `check` and `spec` never evaluate them, since they only need the parse
tree, so `chore list` does no I/O and works even when a file a global reads is
missing.

## include

```sh
include ffmpeg.chore
include libs/chorefile as libs     # its tasks become libs::build
```

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

## check

Builtins are reserved by convention, not by the interpreter: at runtime a task
wins over a builtin of the same name, and it is `check` that reports it. The
same is true of a task named after a subcommand. `chore list` is always the
subcommand, so the task is unreachable.

Reports syntax errors, reserved names, unknown commands, undefined variables
(in a parameter's default as much as in a body), duplicate names, a parameter
declared twice in one header, a parameter read as `$name` where parameters are
positional, include cycles, and non-portable commands (`curl`, `unzip`, `tar`,
`cp`, `rm`) with the builtin that replaces them. A default is checked in the
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

Only the `PATH` lookup is affected. An undefined variable and a non-portable
command are wrong on every platform, so `check` reports them inside a platform
guard exactly as it does outside one.

Warnings do not fail the run: `chore check` exits nonzero only for errors, so a
tool that exists solely in CI does not break the gate.

## Out of scope for v1

Plugins, dependency syntax, arithmetic, arrays, globs beyond `find`, `while`,
functions.
