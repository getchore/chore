# chore

A single static binary that runs project tasks from a `chorefile`.

It finds `chorefile` in the working directory or the nearest parent and runs
tasks through a built-in POSIX-sh-subset interpreter. It never spawns the host
shell, so behavior is identical on macOS, Linux and Windows.

```sh
# build the project
task build {
    mkdir dist
    cargo build --release
    copy target/release/app$EXE dist/
}

task fetch url {
    download $1 dist/asset --sha256 $SHA
}
```

```
chore build
chore list            # tasks and descriptions
chore check           # lint without running
chore help            # syntax and builtins
```

## Install

```sh
curl -fsSL https://getchore.github.io/chore/install.sh | sh
```

```powershell
irm https://getchore.github.io/chore/install.ps1 | iex
```

Or download a binary from [releases][releases] — macOS, Linux and Windows, on
x86-64 and arm64. The Linux builds are static, so they run on any distro.

[releases]: https://github.com/getchore/chore/releases/latest

## Why

- One binary, no shell, no dependencies.
- Same behavior on every platform. Paths use `/` everywhere.
- Builtins for the things scripts usually shell out for: `download`, `extract`,
  `archive`, `copy`, `move`, `remove`, `find`, `sha256`.
- `--dry` previews a run; `chore check` catches errors before it starts.

## License

MIT
