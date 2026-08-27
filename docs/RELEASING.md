# Releasing

## Targets

Six, one per platform anyone actually runs:

| Asset                                | Platform                          |
| ------------------------------------ | --------------------------------- |
| `chore-x86_64-apple-darwin.tar.gz`    | macOS, Intel                      |
| `chore-aarch64-apple-darwin.tar.gz`   | macOS, Apple silicon              |
| `chore-x86_64-unknown-linux-musl.tar.gz`  | Linux, x86-64                 |
| `chore-aarch64-unknown-linux-musl.tar.gz` | Linux, arm64                  |
| `chore-x86_64-pc-windows-msvc.zip`    | Windows, x86-64                   |
| `chore-aarch64-pc-windows-msvc.zip`   | Windows, arm64                    |

Each archive holds the binary plus `LICENSE` and `README.md`, ships with a
`.sha256` sidecar, and the release carries a combined `SHA256SUMS`. v1.0.0
shipped all thirteen assets — six archives, six sidecars, `SHA256SUMS` — and
`install.sh` installs from them and verifies the checksum, so the whole path
from tag to installed binary is known to work.

`x86_64-pc-windows-gnu` is deliberately not on this list. CI's `cross` job
builds it on every PR — it is the only target where `ring` and `zstd-sys`
compile against mingw, so a linking break there is worth catching early — but
it is not published as a release asset. Windows users get msvc.

Linux is musl on both arches. The result is statically linked, so one asset
per architecture covers every distro, glibc or not — which is the same
promise the binary makes about not needing a shell.

Both macOS targets build on the arm runner: Apple's clang cross-compiles, so
the C in `zstd-sys` and `ring` builds for either arch without a second
machine. Everything else builds natively, including Windows on arm.

## Cutting a release

By hand, a few times a year. There is no `chore` task for it: `chore` has no
builtin that edits a file, so it cannot rewrite `Cargo.toml`, and pushing a
tag is not something to leave one keystroke away.

```sh
# 1. bump the version -- both lines, see below
python3 - 1.1.0 <<'EOF'
import re, pathlib, sys
version = sys.argv[1]
p = pathlib.Path("Cargo.toml")
s = p.read_text()
# [workspace.package]: the first `version = "..."` line in the file.
s, n = re.subn(r'^version = ".*"$', f'version = "{version}"', s, count=1, flags=re.M)
assert n == 1, "no version line in Cargo.toml"
# [workspace.dependencies]: the chorefile path dependency carries its own pin.
s, d = re.subn(r'(path = "crates/chorefile", version = ")[^"]+(")', rf'\g<1>{version}\g<2>', s, count=1)
assert d == 1, "chorefile version pin not found"
p.write_text(s)
print(f'Cargo.toml: {version}')
EOF

# 2. refresh Cargo.lock, which every build reads with --locked
cargo update -w

# 3. the gates CI runs, in CI's order
chore ci

# 4. commit, tag, push
git commit -am "Release 1.1.0"
git tag v1.1.0
git push origin main v1.1.0
```

Two lines in `Cargo.toml` carry the version and both have to move: the one
under `[workspace.package]`, and the `version = "..."` pin on the `chorefile`
path dependency under `[workspace.dependencies]`. Cargo refuses to resolve
when that pin lags behind the crate it points at.

The script rewrites both and asserts on each, which is the point of it being a
script. The `sed` range it replaced matched under GNU and not under BSD, so on
a Mac it edited nothing, said nothing and exited zero — a release tool that
fails silently is worse than one that is not there.

The tag triggers `.github/workflows/release.yml`, which refuses to build if
the tag and `Cargo.toml` disagree. It builds all six targets, smoke-tests
each one it can execute, then creates the release with generated notes. This
is how v1.0.0 was published.

The smoke test writes a one-task chorefile and runs `--version`, `list`,
`list --json`, `check` and the task itself. `x86_64-apple-darwin` is skipped:
it is the one cross-built target and Rosetta may not be on the runner, so a
failure there would say nothing about the binary. `chore check` exits nonzero
only for errors — a command missing from the runner's `PATH` is a warning — so
the smoke test does not depend on what happens to be installed there.

A tag containing `-alpha`, `-beta` or `-rc` is published as a prerelease.

To rehearse without tagging, run the workflow manually with a tag name — it
builds everything and leaves the release as a **draft**.

## Installers

`install.sh` and `install.ps1` live in `installers/`, and the website build
copies them into `dist/` — so they are served from the Pages site, not from a
release asset. A change to either is live once the `website` workflow deploys
the merge to `main`. CI shellchecks and parse-checks both on every PR.

There is no staged or versioned copy: whatever is on `main` is what the next
person to run the one-liner executes, which is why the CI check is not
optional. Pinning the scripts themselves — serving `install.sh` from a tag
rather than from the tip of `main` — is the obvious next step and is not done
yet.

Neither calls the GitHub API. `…/releases/latest/download/<asset>` resolves
server-side, so there is no JSON to parse and no rate limit to hit.

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

Both are live on the Pages site and both serve `200`. Both take an optional
release tag as their first argument, which wins over
`CHORE_VERSION`; both also honour `CHORE_INSTALL_DIR` (default `~/.local/bin`).
Checksum verification is best-effort on a missing sidecar and fatal on a
mismatch. `install.ps1` sets the user PATH; `install.sh` only prints the line
to add, rather than guessing which of your rc files is the right one.

## Not yet wired up

- **Signing and notarization.** macOS assets are unsigned, so Gatekeeper
  quarantines a download from a browser. `curl | sh` is unaffected — the
  quarantine bit comes from the browser, not the file. Notarizing needs an
  Apple Developer account and four secrets.
- **crates.io.** `cargo publish -p chorefile && cargo publish -p chore`,
  in that order. Not automated until the crate names are claimed.
- **Homebrew / Scoop / winget.** Worth adding once the asset names are
  stable through a release or two.
