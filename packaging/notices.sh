#!/bin/sh
# Writes the third-party notices of the programs the packages ship, for the given targets.
#
#   notices.sh <out-file> <target-triple>...
#
# The file opens with the words at the top of notices.hbs, once, and then has one part for
# each group of programs, each opened by a line of 80 "=" and a title line:
#   PART 1 OF 3: THE WINDOW, yaar-review                          (crate yaar-window)
#   PART 2 OF 3: THE KEEPER, yaar-keeper, AND PI'S REPORTING PROGRAM, yaar-pi-report
#                                                     (crates yaar-keeper, yaar-adapter-pi)
#   PART 3 OF 3: CLAUDE CODE'S HOOK PROGRAM AND TOOL SERVER, yaar-claude-hook and
#                yaar-claude-tools                                (crate yaar-companion-claude)
# Each part lists what is linked into its programs, from the lock file, with every licence
# text. packaging/notices-count.sh counts each part against `cargo tree` (row LIC-02).
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

# An empty or near-empty part means that the tool broke or that the template lost its
# rule, not that nothing is linked: the Window links hundreds of crates, and the Keeper's
# part holds at least the text of the licence of its own crates.
small() {
  echo "notices.sh: the part of $1 is suspiciously small" >&2
  exit 1
}
[ "$(wc -c < "$TMP/window.part")" -gt 20000 ] || small yaar-review
[ "$(wc -c < "$TMP/keeper.part")" -gt 10000 ] || small yaar-keeper

DOUBLE="================================================================================"
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
