<div align="center">

<img src="website/public/brand/logo-256.png" alt="" width="84" height="84">

# chore

**One binary. Every task. Every OS.**

Run your project's tasks from a `chorefile`, through a shell that lives inside
the binary. The same file does the same thing on macOS, Linux and Windows.

[**Get started**](https://getchore.github.io/chore/) &nbsp;·&nbsp; [**Language reference**](docs/SPEC.md) &nbsp;·&nbsp; [**Releases**](https://github.com/getchore/chore/releases/latest)

[![CI](https://img.shields.io/github/actions/workflow/status/getchore/chore/ci.yml?branch=main&style=flat-square&label=ci)](https://github.com/getchore/chore/actions/workflows/ci.yml) [![Release](https://img.shields.io/github/v/release/getchore/chore?style=flat-square&color=f97316)](https://github.com/getchore/chore/releases/latest) [![License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

</div>

---

## Install

```sh
curl -fsSL https://getchore.github.io/chore/install.sh | sh
```

```powershell
irm https://getchore.github.io/chore/install.ps1 | iex
```

Append `-s -- v1.1.0` (or run the PowerShell script as a block) to pin a
version. Both honour `CHORE_INSTALL_DIR`, default `~/.local/bin`, and verify
the published checksum. Prebuilt archives for macOS, Linux (musl-static) and
Windows on x86-64 and arm64 are on the
[releases page](https://github.com/getchore/chore/releases/latest).

## Tab completion

```sh
# ~/.zshrc
source <(chore completions zsh)
```

`chore completions --write` adds that line for you, to the startup file of
whichever shell `$SHELL` names, and running it a second time changes nothing.
bash, zsh, fish and powershell have scripts, and `chore completions <shell>`
prints one to stdout for a package manager to redirect. PowerShell resolves
`$PROFILE` for itself, so there the line goes in by hand. Names come from
`chore list --names` in the current directory, so completion works in every
project with nothing to set up per repo.

## In GitHub Actions

```yaml
- uses: getchore/setup-chore@v1
- run: chore ci
```

Installs `chore` and puts it on `PATH` on `ubuntu-*`, `macos-*` and
`windows-*` runners, so a matrix leg no longer needs a `runner.os` branch to
install its tools. Pin a version with `with: {version: v1.1.0}`.

## A chorefile

`chore` reads the `chorefile` in the working directory or the nearest parent,
and the comment above a task is its description.

```sh
DIST=$ROOT/dist/$PLATFORM

# build the compiler
task build {
    if !exists vendor/llvm {
        download $LLVM vendor/llvm.tar.zst --sha256 4f9c2a
        extract vendor/llvm.tar.zst vendor/llvm --strip 1
    }
    cmake --build build --parallel
    copy build/sona$EXE $DIST/sona$EXE
}

# package the release archive for this platform
task package {
    build
    archive $DIST sona-$PLATFORM.tar.gz
}
```

```console
$ chore list
  build      build the compiler
  package    package the release archive for this platform
```

`download`, `extract`, `archive`, `copy`, `move`, `remove`, `find` and
`sha256` are builtins, so the tasks above need nothing installed on the
machine and behave the same on every platform.

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

`check` only parses, and nothing runs. It exits nonzero for errors like these.
A command it cannot find on this machine's `PATH` is a warning instead, so a
tool that exists solely in CI does not break the gate.

## Docs

[getchore.github.io/chore](https://getchore.github.io/chore/) is the guide, and
[docs/SPEC.md](docs/SPEC.md) is the full language reference. `chore help` and
`chore spec` print the same material from the binary. Release and installer
mechanics are in [docs/RELEASING.md](docs/RELEASING.md).

`chore` builds itself with its own `chorefile`: `chore list` shows the tasks,
and `chore ci` is the gate CI runs.

## License

MIT
