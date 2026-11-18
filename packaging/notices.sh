#!/bin/sh
# Writes the third-party notices of the two programs for the given targets.
#
#   notices.sh <out-file> <target-triple>...
#
# The file opens with the words at the top of notices.hbs, once, and then has one part
# for each program: what is linked into yaar-review, then what is linked into yaar-keeper.
# Neither program's dependencies hold the other's. The Keeper links SQLite and its
# binding, which the Window does not; the Window links the interface toolkit.
#
# Needs cargo-about (.github/scripts/install-tools.sh <dir> about). Fails when a linked
# dependency is under a licence that about.toml does not accept.
set -eu
OUT=${1:?usage: notices.sh <out-file> <target-triple>...}; shift
[ $# -gt 0 ] || { echo "usage: notices.sh <out-file> <target-triple>..." >&2; exit 2; }
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGETS=""
for t in "$@"; do TARGETS="$TARGETS --target $t"; done
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# The template rendered for one program: generate <manifest> <file>.
generate() {
  # shellcheck disable=SC2086
  cargo about generate --locked --config "$ROOT/about.toml" \
    --manifest-path "$ROOT/$1" $TARGETS \
    --output-file "$2" "$ROOT/packaging/notices.hbs"
}
generate crates/window/window/Cargo.toml "$TMP/window"
generate crates/keeper/keeper/Cargo.toml "$TMP/keeper"

# A rendering is the opening words, then everything from the first rule of 80 dashes on:
# the overview and the licence texts of that program.
RULE='^-\{80\}$'
sed "/$RULE/,\$d" "$TMP/window" > "$TMP/opening"
sed -n "/$RULE/,\$p" "$TMP/window" > "$TMP/window.part"
sed -n "/$RULE/,\$p" "$TMP/keeper" > "$TMP/keeper.part"

# An empty or near-empty part means the template or the tool broke, not that nothing is
# linked: the Window links hundreds of crates, and the Keeper's part holds at least the
# text of the licence of its own crates.
[ -s "$TMP/opening" ] || { echo "notices.sh: notices.hbs no longer opens with words and a rule of 80 dashes" >&2; exit 1; }
[ "$(wc -c < "$TMP/window.part")" -gt 20000 ] || { echo "notices.sh: the part of yaar-review is suspiciously small" >&2; exit 1; }
[ "$(wc -c < "$TMP/keeper.part")" -gt 10000 ] || { echo "notices.sh: the part of yaar-keeper is suspiciously small" >&2; exit 1; }

DOUBLE=================================================================================
mkdir -p "$(dirname "$OUT")"
{
  cat "$TMP/opening"
  echo "$DOUBLE"
  echo "PART 1 OF 2: THE WINDOW, yaar-review"
  echo "$DOUBLE"
  echo
  cat "$TMP/window.part"
  echo "$DOUBLE"
  echo "PART 2 OF 2: THE KEEPER, yaar-keeper"
  echo "$DOUBLE"
  echo
  cat "$TMP/keeper.part"
} > "$OUT"
