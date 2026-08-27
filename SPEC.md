# chore

A single static binary that runs project tasks from a `chorefile`. It finds
`chorefile` in the working directory or the nearest parent, and runs tasks
through a built-in POSIX-sh-subset interpreter. It never spawns the host
shell, so behavior is identical on macOS, Linux and Windows (gnu and msvc).

## CLI

```
chore <task> [args...] [--dry] [--force]
chore list [--json]        # tasks and descriptions
chore help [builtin]       # syntax and builtins, or one builtin
chore check                # lint without running
chore spec                 # full reference as JSON, for agents
```

`list`, `help`, `check` and `spec` are reserved task names.

- `--dry` echoes commands without side effects.
- `--force` disables run-once.

## Syntax

```
x=value                      assignment
$x  "$x/lib"                 interpolation
$(cmd)                       capture stdout, trimmed; nonzero fails unless `try`
if cond { } else if cond { } else { }
for x in a b c { }
for f in $(find src *.rs) { }        space-split
try cmd                      don't fail on nonzero
exit [code]
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

## Resolution

task → builtin → `PATH`. A leading `^` forces `PATH`: `^find`.

## Builtin variables

Read-only, always set.

```
$OS        macos | linux | windows
$ARCH      x86_64 | arm64
$ENV       gnu | msvc | ""
$PLATFORM  $OS-$ARCH
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
extract  <archive> <dest> [--member name] [--strip n]
                              zip, tar, .gz .xz .zst
archive  <src> <dest>         format from the extension
copy     <src> <dest>         file or dir
move     <src> <dest>
remove   <path...>            recursive, no error if missing
mkdir    <path...>            -p semantics
chmod    <mode> <path>
which    <name>               prints path, or fails
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

## Runtime

- Echo each command before running it.
- Fail fast; stream output as it is produced.
- Paths are written with `/` everywhere and converted on Windows.
- argv is passed to the OS directly. Nothing is re-quoted or re-expanded.
- A task runs once per invocation, keyed on **name and arguments**, so a
  parameterised task called twice with different arguments still runs twice.
  `--force` disables this.
- `cd` changes the interpreter's directory, not the process's.

### `--dry`

Echoes commands and skips the ones with effects. **Captures and conditions
still run**: a `$(...)` that did not execute would leave every interpolated
path downstream empty, and the preview would describe a run that could never
happen.

### Top-level statements

Top-level assignments are evaluated once, before the first task. `list`,
`help`, `check` and `spec` never evaluate them — they only need the parse
tree, so `chore list` does no I/O and works even when a file a global reads is
missing.

## include

Deferred to v1.1; sona's port does not need it. Settled semantics, so adding
it later is not a breaking change:

- Paths resolve relative to the **including file**, not the working directory.
  A directory argument means the `chorefile` inside it.
- `$ROOT` stays the top-level chorefile's directory in included files. One
  root per invocation.
- `as` namespaces **both tasks and globals** (`libs::build`). Without `as`
  everything merges flat, and any duplicate name — task or global — is a
  `check` error.
- `::` is reserved in task names. Include cycles are a `check` error.

## check

Reports syntax errors, reserved names, unknown commands, undefined variables,
duplicate names, include cycles, and non-portable commands (`curl`, `unzip`,
`tar`, `cp`, `rm`) with the builtin that replaces them.

## Out of scope for v1

Plugins, dependency syntax, arithmetic, arrays, globs beyond `find`, `while`,
functions.
