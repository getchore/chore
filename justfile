# Development tasks for chore itself.
#
# `chore` is a task runner, so this justfile is temporary: once the binary
# runs its own chorefile, these recipes move there and this file goes away.

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

bin := "chore"

# List the recipes.
default:
    @just --list --unsorted

# --- build ----------------------------------------------------------------

# Debug build of the whole workspace.
[group('build')]
build:
    cargo build --workspace --all-targets

# Optimised build, the profile releases actually ship.
[group('build')]
release:
    cargo build --release --locked --bin {{bin}}

# Run the binary. `just run list`, `just run build --dry`.
[group('build')]
run *ARGS:
    cargo run --quiet --bin {{bin}} -- {{ARGS}}

# Install into ~/.cargo/bin, for dogfooding on real projects.
[group('build')]
install:
    cargo install --path crates/{{bin}} --locked

[group('build')]
[doc('Remove the target directory.')]
clean:
    cargo clean

# --- check ----------------------------------------------------------------

# Everything CI runs, in CI's order. Run this before pushing.
[group('check')]
ci: fmt-check clippy test installers

# The two that fail most often, fast.
[group('check')]
check: fmt-check clippy

[group('check')]
fmt:
    cargo fmt --all

[group('check')]
fmt-check:
    cargo fmt --all --check

[group('check')]
clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# `just test` for all of them, `just test parse` for one file.
[group('check')]
test FILTER="":
    cargo test --workspace --locked {{ FILTER }}

# Watch the tests. Needs `cargo install cargo-watch`.
[group('check')]
watch FILTER="":
    cargo watch -x "test --workspace {{ FILTER }}"

# --- cross ----------------------------------------------------------------

# Build one target. `just cross x86_64-unknown-linux-musl`.
[group('cross')]
cross TARGET:
    rustup target add {{TARGET}}
    cargo build --release --locked --target {{TARGET}} --bin {{bin}}

# The promise chore makes: a Linux binary with no interpreter and no shared
# objects. Linux only -- macOS always links libSystem.
[group('cross')]
[linux]
static-check TARGET="x86_64-unknown-linux-musl": (cross TARGET)
    file target/{{TARGET}}/release/{{bin}}
    ldd target/{{TARGET}}/release/{{bin}} 2>&1 | grep -q 'not a dynamic executable'

# What the release workflow builds, minus the Windows targets, which need
# their own runners.
[group('cross')]
[unix]
[doc('Build every target the release workflow builds that a Mac can build.')]
cross-all:
    for t in x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-musl; do \
        just cross "$t" || exit 1; \
    done

# --- installers -----------------------------------------------------------

# Lint both install scripts, the way CI does. They ship from main, so a
# broken one is live the moment it merges.
[group('installers')]
[unix]
[doc('Lint install.sh and install.ps1, the way CI does.')]
installers:
    shellcheck --shell=sh installers/install.sh
    @command -v pwsh >/dev/null && pwsh -NoLogo -Command '$e=$null; [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path ./installers/install.ps1), [ref]$null, [ref]$e) | Out-Null; if ($e) { $e | ForEach-Object { Write-Error $_.Message }; exit 1 }; Write-Host "install.ps1 parses"' || echo "pwsh not installed, skipping install.ps1"

# --- website --------------------------------------------------------------

[group('website')]
[working-directory('website')]
[doc('Run the website dev server.')]
web:
    pnpm install
    pnpm dev

# Build the site exactly as Pages does, including the project-page base path.
[group('website')]
[working-directory('website')]
web-build:
    pnpm install --frozen-lockfile
    VITE_BASE=/chore/ pnpm build

# --- release --------------------------------------------------------------

# Set the workspace version. `just version 0.2.0`. The release workflow
# refuses to build when the tag and Cargo.toml disagree, so this comes first.
[group('release')]
[unix]
[doc('Set the workspace version. `just version 0.2.0`.')]
version VERSION:
    sed -i '' -e '0,/^version = ".*"$/s//version = "{{VERSION}}"/' Cargo.toml
    cargo update --workspace
    @grep -m1 '^version' Cargo.toml

# Tag and push, which is what triggers the release build.
[group('release')]
[unix]
[confirm("tag and push a release? this publishes to GitHub. [y/N]")]
tag:
    #!/usr/bin/env sh
    set -eu
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
    just ci
    git tag "v$version"
    git push origin "v$version"
    echo "pushed v$version"
