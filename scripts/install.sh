#!/bin/sh
# Installs prrit and git-remote-prrit from GitHub Releases.
#
#   curl -fsSL https://github.com/sunya9/prrit/releases/latest/download/install.sh | sh
#
# Environment:
#   PRRIT_VERSION      tag to install (default: latest)
#   PRRIT_INSTALL_DIR  destination directory (default: ~/.local/bin)
set -eu

repo="sunya9/prrit"
version="${PRRIT_VERSION:-latest}"
dir="${PRRIT_INSTALL_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) os_part="apple-darwin" ;;
  Linux) os_part="unknown-linux-musl" ;;
  *) echo "unsupported OS: $os" >&2; exit 1 ;;
esac
case "$arch" in
  arm64 | aarch64) arch_part="aarch64" ;;
  x86_64 | amd64) arch_part="x86_64" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac
target="${arch_part}-${os_part}"
archive="prrit-${target}.tar.gz"

if [ "$version" = "latest" ]; then
  base="https://github.com/${repo}/releases/latest/download"
else
  base="https://github.com/${repo}/releases/download/${version}"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "Downloading ${base}/${archive}"
curl -fsSL -o "$tmp/$archive" "${base}/${archive}"
curl -fsSL -o "$tmp/$archive.sha256" "${base}/${archive}.sha256"

expected="$(cut -d' ' -f1 "$tmp/$archive.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$archive" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)"
fi
if [ "$expected" != "$actual" ]; then
  echo "checksum mismatch for $archive" >&2
  exit 1
fi

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$dir"
install -m 0755 "$tmp/prrit-${target}/prrit" "$tmp/prrit-${target}/git-remote-prrit" "$dir/"
echo "Installed prrit and git-remote-prrit to $dir"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH (git must find git-remote-prrit there)." ;;
esac
