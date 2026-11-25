#!/bin/sh
# Writes the third-party notices of the programs the packages ship, in three parts and a section
# for the two fonts the Window embeds.
# notices.sh <out-file> <target-triple>...; needs cargo-about (install-tools.sh <dir> about).
set -eu
OUT=${1:?usage: notices.sh <out-file> <target-triple>...}; shift
[ $# -gt 0 ] || { echo "usage: notices.sh <out-file> <target-triple>..." >&2; exit 2; }
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGETS=""
for t in "$@"; do TARGETS="$TARGETS --target $t"; done
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

generate() {
  # shellcheck disable=SC2086
  cargo about generate --locked --config "$ROOT/about.toml" \
    --manifest-path "$ROOT/$1" $TARGETS \
    --output-file "$2" "$ROOT/packaging/notices.hbs" \
    || { echo "notices.sh: cargo about could not write the notices of $1: a dependency may be under a licence that about.toml does not accept" >&2; exit 1; }
}
# pi's reporting program (crate yaar-adapter-pi) gets no rendering of its own: the Keeper links
# that crate, so the Keeper's rendering already holds what the program links.
generate crates/window/window/Cargo.toml "$TMP/window"
generate crates/keeper/keeper/Cargo.toml "$TMP/keeper"
generate companions/claude/Cargo.toml "$TMP/companion"

RULE='^-\{80\}$'
sed "/$RULE/,\$d" "$TMP/window" > "$TMP/opening"
for part in window keeper companion; do
  sed -n "/$RULE/,\$p" "$TMP/$part" > "$TMP/$part.part"
done

# An empty or near-empty part means the tool broke or the template lost its rule.
small() {
  echo "notices.sh: the part of $1 is suspiciously small" >&2
  exit 1
}
[ "$(wc -c < "$TMP/window.part")" -gt 20000 ] || small yaar-review
[ "$(wc -c < "$TMP/keeper.part")" -gt 10000 ] || small yaar-keeper
[ "$(wc -c < "$TMP/companion.part")" -gt 10000 ] || small "Claude Code's two programs"

FONTS=$ROOT/crates/window/ui/fonts
# Each line of the README opens with a font and its release, up to the colon.
INTER=$(sed -n '1s/:.*//p' "$FONTS/README")
MONO=$(sed -n '2s/:.*//p' "$FONTS/README")
[ -n "$INTER" ] && [ -n "$MONO" ] && [ -s "$FONTS/OFL.txt" ] \
  || { echo "notices.sh: the fonts folder holds no README of two lines and no OFL.txt" >&2; exit 1; }

DOUBLE="================================================================================"
mkdir -p "$(dirname "$OUT")"
{
  cat "$TMP/opening"
  echo "$DOUBLE"
  echo "PART 1 OF 3: THE WINDOW, yaar-review"
  echo "$DOUBLE"
  echo
  cat "$TMP/window.part"
  echo "$DOUBLE"
  echo "PART 2 OF 3: THE KEEPER, yaar-keeper, AND PI'S REPORTING PROGRAM, yaar-pi-report"
  echo "$DOUBLE"
  echo
  cat "$TMP/keeper.part"
  echo "$DOUBLE"
  echo "PART 3 OF 3: CLAUDE CODE'S HOOK PROGRAM AND TOOL SERVER, yaar-claude-hook and yaar-claude-tools"
  echo "$DOUBLE"
  echo
  cat "$TMP/companion.part"
  echo "$DOUBLE"
  echo "Fonts: $INTER and $MONO, SIL Open Font License 1.1"
  echo "$DOUBLE"
  echo
  cat "$FONTS/OFL.txt"
} > "$OUT"
# Text only OFL.txt holds: the heading above says "SIL Open Font License" whatever the file is.
grep -q "The Inter Project Authors" "$OUT" && grep -q "The JetBrains Mono Project Authors" "$OUT" \
  || { echo "notices.sh: the notices hold no font licence" >&2; exit 1; }
