# chore

One static binary that runs your project's tasks from a `chorefile`.

Tasks run through a built-in POSIX-sh-subset interpreter — `chore` never spawns
`sh`, `cmd` or PowerShell — so a chorefile does the same thing on macOS, Linux
and Windows. Builtins cover what scripts normally shell out for: `download`,
`extract`, `archive`, `copy`, `move`, `remove`, `find`, `sha256`.

## It tells you what won't work on Windows

```sh
# fetch the SDK
task sdk {
    curl -L https://example.com/sdk.zip -o sdk.zip
    unzip sdk.zip -d vendor
}
```

```console
$ chore check
/tmp/myapp/chorefile:3:5: `curl` is not portable: it is missing, or spelled differently, on at least one platform this chorefile can run on
  help: use the `download` builtin — it speaks https and `gh://owner/repo/tag/asset`, takes `--retries`, `--timeout` and `--sha256`, and needs nothing installed on the machine
/tmp/myapp/chorefile:4:5: `unzip` is not portable: it is missing, or spelled differently, on at least one platform this chorefile can run on
  help: use the `extract` builtin — it unpacks zip, tar, `.gz`, `.xz` and `.zst` with the same flags everywhere, and Windows has no `unzip`

2 problems
```

`check` also reports syntax errors, unknown commands, undefined variables (with
a spelling suggestion), duplicate names, and tasks that shadow a builtin or a
subcommand. It only parses — no globals are evaluated, nothing is run.

## Install

```sh
curl -fsSL https://getchore.github.io/chore/install.sh | sh

# pin a version
curl -fsSL https://getchore.github.io/chore/install.sh | sh -s -- v0.1.0
```

```powershell
irm https://getchore.github.io/chore/install.ps1 | iex

# pin a version (iex takes no arguments, so run it as a block)
& ([scriptblock]::Create((irm https://getchore.github.io/chore/install.ps1))) v0.1.0
```

Both honour `CHORE_INSTALL_DIR` (default `~/.local/bin`) and verify the release
checksum; the scripts themselves are in [`installers/`](installers). Or download
an archive from [releases][releases]: macOS, Linux and Windows, on x86-64 and
arm64. The Linux builds are musl-static, so they run on any distro.

[releases]: https://github.com/getchore/chore/releases/latest

## Example

`chore` uses the `chorefile` in the working directory or the nearest parent.

```sh
VERSION=0.4.2
DIST=dist/$PLATFORM

# fetch the pinned LLVM and build the compiler
task build {
    download gh://sona-lang/deps/v3/llvm-$PLATFORM.tar.zst vendor/
    extract vendor/llvm-$PLATFORM.tar.zst vendor/llvm --strip 1
    cmake -B build -DLLVM_DIR=vendor/llvm/lib/cmake
    cmake --build build --parallel
}

# run the suite; extra arguments go straight to ctest
task test {
    ctest --test-dir build $@
}

# package the release archive for this platform
task package {
    build
    mkdir $DIST
    copy build/sona$EXE $DIST/sona$EXE
    archive $DIST sona-$VERSION-$PLATFORM.tar.gz
}
```

The comment above a task is its description:

```console
$ chore list
  build      fetch the pinned LLVM and build the compiler
  test       run the suite; extra arguments go straight to ctest
  package    package the release archive for this platform
```

`--dry` shows the whole run, fully expanded, without touching anything:

```console
$ chore package --dry
$ build
$ download gh://sona-lang/deps/v3/llvm-macos-arm64.tar.zst vendor/
$ extract vendor/llvm-macos-arm64.tar.zst vendor/llvm --strip 1
$ cmake -B build -DLLVM_DIR=vendor/llvm/lib/cmake
$ cmake --build build --parallel
$ mkdir dist/macos-arm64
$ copy build/sona dist/macos-arm64/sona
$ archive dist/macos-arm64 sona-0.4.2-macos-arm64.tar.gz
```

Arguments after the task name go to the task, flags included, so
`chore test -R parser` reaches `ctest` unchanged. A task that declares
parameters requires them: `task fetch url` must be called as `chore fetch <url>`.

## The language

[SPEC.md](SPEC.md) is the full reference, and `chore help` prints the same
material from the binary. In short: assignment, `$x` and `"$x/lib"`, `$(cmd)`,
`if` / `else if` / `else`, `for x in ...`, `try`, `exit`, and `&&`, `||`, `|`,
`>`, `>>`, `2>`. Conditions are `==`, `!=`, `contains`, `starts-with`,
`ends-with`, `exists`, `which`, or any command's exit code. `$OS`, `$ARCH`,
`$PLATFORM`, `$EXE`, `$ROOT`, `$TASK` and a few more are always set. Names
resolve task → builtin → `PATH`, and a leading `^` forces `PATH`: `^find`.

Three rules that catch people out:

- **A quoted word is always exactly one argument, and an unquoted `$var` splits
  on whitespace**, as in sh. There are no arrays and no quoting inside a
  variable, so an argument containing a space has to be written quoted at the
  call site: `cmake -B build -G "MinGW Makefiles"`.
- **A task runs once per invocation, keyed on its name *and* its arguments.**
  Calling `greet world` twice runs it once; `greet world` then `greet again`
  runs it twice. `--force` turns this off.
- **`--dry` skips effects but still evaluates captures and conditions.** A
  `$(git rev-parse HEAD)` really runs, so the preview shows the paths the real
  run would use rather than a run full of empty strings that could never happen.

## CLI

```
chore <task> [args...] [--dry] [--force]
chore list [--json]        tasks and descriptions
chore help [builtin]       syntax and builtins, or one builtin
chore check                lint without running
chore spec                 full reference as JSON, for agents
chore --version
```

`list`, `help`, `check` and `spec` are reserved: a task with one of those names
is unreachable, and `check` says so. `--dry` and `--force` are recognised
anywhere on the line; every other flag after the task name is passed to the
task, and `--` makes even those literal.

`help` and `spec` need no chorefile at all. `list`, `help`, `check` and `spec`
never evaluate top-level assignments, so they still work when a global would
read a file that is not there yet.

`chore list --json` and `chore spec` are the machine-readable pair — one for
what this project can do, one for what the language is:

```console
$ chore list --json
[
  {"name": "build", "description": "fetch the pinned LLVM and build the compiler", "params": []},
  {"name": "test", "description": "run the suite; extra arguments go straight to ctest", "params": []},
  {"name": "package", "description": "package the release archive for this platform", "params": []}
]
```

Exit codes:

| Code | Meaning                                                     |
| ---- | ----------------------------------------------------------- |
| 0    | success; `chore check` found nothing                          |
| 1    | the run failed, or `check` reported problems                  |
| 2    | usage: unknown option, unknown task, or no chorefile found    |
| _n_  | a task's own `exit n` becomes the process exit code           |

## Status

v1, with the edges stated plainly:

- `include` is parsed but not yet followed. A chorefile containing an `include`
  line loads and runs, the included file is ignored, and `check` does not
  complain about it. The semantics are settled in SPEC.md; it lands in v1.1.
- Releases carry six assets: macOS, Linux-musl and Windows-**msvc**, each on
  x86-64 and arm64. `x86_64-pc-windows-gnu` is built on every PR in CI, but it
  is not shipped as a release artifact.
- macOS binaries are unsigned, so a browser download is quarantined by
  Gatekeeper. `curl | sh` is not affected.
- Out of scope for v1: plugins, dependency syntax, arithmetic, arrays, globs
  beyond `find`, `while`, functions.

## License

MIT
