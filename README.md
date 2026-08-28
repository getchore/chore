<div align="center">

<img src="website/public/brand/logo-256.png" alt="" width="84" height="84">

# chore

**One binary. Every task. Every OS.**

Run your project's tasks from a `chorefile`, through a shell that lives inside
the binary. The same file does the same thing on macOS, Linux and Windows.

[**Get started**](https://getchore.github.io/chore/) &nbsp;·&nbsp; [**Reference**](https://getchore.github.io/chore/reference) &nbsp;·&nbsp; [**Releases**](https://github.com/getchore/chore/releases/latest)

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

## A chorefile

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

`download`, `extract`, `archive` and `copy` are builtins, so this needs nothing
installed and behaves the same everywhere.

## It tells you what won't work on Windows

```console
$ chore check
chorefile:3:5: `curl` is not portable: it is missing, or spelled differently, on at least one platform this chorefile can run on
  help: use the `download` builtin — it speaks https and `gh://owner/repo/tag/asset`, takes `--retries`, `--timeout` and `--sha256`, and needs nothing installed on the machine

1 problem
```

Nothing runs. Errors exit nonzero; a command missing only from *this* machine
is a warning, so a CI-only tool doesn't break the gate.

## More

- [**Guide and reference**](https://getchore.github.io/chore/) — and [`llms.txt`](https://getchore.github.io/chore/llms-full.txt) for agents
- [**setup-chore**](https://github.com/getchore/setup-chore) — `- uses: getchore/setup-chore@v1`
- [**VS Code extension**](https://github.com/getchore/chorefile-vscode) — highlighting for `chorefile` and `.chore`
- `chore completions --write` — tab completion for bash, zsh, fish and powershell
- [docs/SPEC.md](docs/SPEC.md) · [docs/RELEASING.md](docs/RELEASING.md)

`chore` builds itself with its own [`chorefile`](chorefile) — `chore ci` is the
gate CI runs.

## License

MIT
