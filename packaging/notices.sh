#!/bin/sh
# Writes the third-party notices of the two programs for the given targets.
#
#   notices.sh <out-file> <target-triple>...
#
# Needs cargo-about (.github/scripts/install-tools.sh <dir> about). Fails when a linked
# dependency is under a licence that about.toml does not accept.
set -eu
OUT=${1:?usage: notices.sh <out-file> <target-triple>...}; shift
[ $# -gt 0 ] || { echo "usage: notices.sh <out-file> <target-triple>..." >&2; exit 2; }
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGETS=""
for t in "$@"; do TARGETS="$TARGETS --target $t"; done
mkdir -p "$(dirname "$OUT")"
# The Window's dependencies include every dependency of the Keeper.
# shellcheck disable=SC2086
cargo about generate --locked --config "$ROOT/about.toml" \
  --manifest-path "$ROOT/crates/window/window/Cargo.toml" $TARGETS \
  --output-file "$OUT" "$ROOT/packaging/notices.hbs"
# An empty or near-empty file means the template or the tool broke, not that nothing is linked.
[ "$(wc -c < "$OUT")" -gt 20000 ] || { echo "notices.sh: $OUT is suspiciously small" >&2; exit 1; }
