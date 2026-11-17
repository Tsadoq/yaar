#!/bin/sh
# Fetches the prebuilt check tools from their GitHub releases into $1 (default: $CARGO_HOME/bin or ~/.cargo/bin).
# Versions are pinned here; the sha256 of every download is printed so a later run can be compared.
#   install-tools.sh [dest-dir] [tool ...]      tools: nextest deny machete sccache (default: all four)
set -eu
DEST=${1:-${CARGO_HOME:-$HOME/.cargo}/bin}; [ $# -gt 0 ] && shift
[ $# -gt 0 ] || set -- nextest deny machete sccache
NEXTEST=0.9.152 DENY=0.21.0 MACHETE=0.10.1 SCCACHE=0.19.0
UA="ci-tools-fetch/0.1"
os=$(uname -s); arch=$(uname -m)
case "$arch" in arm64|aarch64) a=aarch64 ;; x86_64|amd64) a=x86_64 ;; *) echo "unsupported arch $arch"; exit 1 ;; esac
case "$os" in
  Darwin) t=$a-apple-darwin; tn=universal-apple-darwin ;;
  Linux)  t=$a-unknown-linux-musl; tn=$a-unknown-linux-gnu ;;
  *) echo "unsupported os $os"; exit 1 ;;
esac
sum() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
mkdir -p "$DEST"
get() { # name url
  curl --proto '=https' --tlsv1.2 -sSfL -A "$UA" -o "$tmp/$1.tar.gz" "$2"
  echo "$1 $(sum "$tmp/$1.tar.gz") $2"
  mkdir -p "$tmp/$1"; tar -xzf "$tmp/$1.tar.gz" -C "$tmp/$1"
  f=$(find "$tmp/$1" -type f -name "$1" | head -1); install -m 755 "$f" "$DEST/$1"
}
for tool in "$@"; do
  case $tool in
    nextest) get cargo-nextest "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$NEXTEST/cargo-nextest-$NEXTEST-$tn.tar.gz" ;;
    deny)    get cargo-deny "https://github.com/EmbarkStudios/cargo-deny/releases/download/$DENY/cargo-deny-$DENY-$t.tar.gz" ;;
    machete) get cargo-machete "https://github.com/bnjbvr/cargo-machete/releases/download/v$MACHETE/cargo-machete-v$MACHETE-$t.tar.gz" ;;
    sccache) get sccache "https://github.com/mozilla/sccache/releases/download/v$SCCACHE/sccache-v$SCCACHE-$t.tar.gz" ;;
    *) echo "unknown tool $tool"; exit 1 ;;
  esac
done
