#!/bin/sh
# chore installer.
#
#   curl -fsSL https://getchore.github.io/chore/install.sh | sh
#
# CHORE_VERSION      version to install, e.g. 0.1.0  (default: latest)
# CHORE_INSTALL_DIR  where the binary lands          (default: ~/.local/bin)
set -eu

REPO="getchore/chore"
DIR="${CHORE_INSTALL_DIR:-$HOME/.local/bin}"

err() { printf 'error: %s\n' "$*" >&2; exit 1; }

arch=$(uname -m)
case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) err "unsupported architecture: $arch" ;;
esac

case "$(uname -s)" in
    Darwin) target="$arch-apple-darwin" ;;
    # Always musl: statically linked, so it runs on glibc distros too.
    Linux) target="$arch-unknown-linux-musl" ;;
    MINGW* | MSYS* | CYGWIN*)
        err "on Windows: irm https://getchore.github.io/chore/install.ps1 | iex" ;;
    *) err "unsupported OS: $(uname -s)" ;;
esac

if [ -n "${CHORE_VERSION:-}" ]; then
    base="https://github.com/$REPO/releases/download/v${CHORE_VERSION#v}"
else
    # Resolves server-side, so there is no API call and no JSON to parse.
    base="https://github.com/$REPO/releases/latest/download"
fi

archive="chore-$target.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "downloading $archive"
curl -fsSL "$base/$archive" -o "$tmp/$archive" || err "no release asset for $target"

# Best effort: a release without sidecars still installs, a mismatch does not.
if curl -fsSL "$base/$archive.sha256" -o "$tmp/sum" 2>/dev/null; then
    want=$(cut -d' ' -f1 < "$tmp/sum")
    got=$(sha256sum "$tmp/$archive" 2>/dev/null || shasum -a 256 "$tmp/$archive")
    case "$got" in
        "$want"*) ;;
        *) err "checksum mismatch for $archive" ;;
    esac
fi

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$DIR"
mv -f "$tmp/chore" "$DIR/chore"
chmod 755 "$DIR/chore"

echo "installed $("$DIR/chore" --version 2>/dev/null || echo chore) to $DIR"
case ":$PATH:" in
    *":$DIR:"*) ;;
    *) echo "add it to your PATH: export PATH=\"$DIR:\$PATH\"" ;;
esac
