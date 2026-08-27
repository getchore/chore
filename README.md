# chore

One static binary that runs your project's tasks from a `chorefile`. Tasks run
through a built-in POSIX-sh-subset interpreter — `chore` never spawns `sh`,
`cmd` or PowerShell — so a chorefile does the same thing on macOS, Linux and
Windows. Builtins cover what scripts shell out for: `download`, `extract`,
`archive`, `copy`, `move`, `remove`, `find`, `sha256`.

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

`check` only parses — nothing runs. It exits nonzero for errors like these; a
command it cannot find on this machine's `PATH` is a warning instead, so a tool
that exists solely in CI does not break the gate.

## Install

```sh
curl -fsSL https://getchore.github.io/chore/install.sh | sh

# pin a version
curl -fsSL https://getchore.github.io/chore/install.sh | sh -s -- v1.0.0
```

```powershell
irm https://getchore.github.io/chore/install.ps1 | iex

# pin a version (iex takes no arguments, so run it as a block)
& ([scriptblock]::Create((irm https://getchore.github.io/chore/install.ps1))) v1.0.0
```

Both honour `CHORE_INSTALL_DIR` (default `~/.local/bin`) and verify the
published checksum when the release carries one. Prebuilt archives for macOS,
Linux (musl-static) and Windows, x86-64 and arm64, are on the
[releases page](https://github.com/getchore/chore/releases/latest).

## Example

`chore` uses the `chorefile` in the working directory or the nearest parent, and
the comment above a task is its description.

```sh
# build the compiler
task build {
    cmake --build build --parallel
}

# package the release archive for this platform
task package {
    build
    archive build sona-$PLATFORM.tar.gz
}
```

```console
$ chore list
  build      build the compiler
  package    package the release archive for this platform
```

## Docs

[getchore.github.io/chore](https://getchore.github.io/chore/) is the guide, and
[docs/SPEC.md](docs/SPEC.md) is the full language reference — `chore help` and
`chore spec` print the same material from the binary. Release and installer
mechanics are in [docs/RELEASING.md](docs/RELEASING.md).

`chore` builds itself with its own `chorefile`: `chore list` shows the tasks,
and `chore ci` is the gate CI runs.

## Status

v1. `x86_64-pc-windows-gnu` is built on every PR in CI but is not shipped as a
release artifact; Windows users get msvc.

## License

MIT
