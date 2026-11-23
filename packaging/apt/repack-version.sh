#!/bin/bash
# The same .deb at another version: unpacks <in.deb>, changes only the Version field of its
# control file, packs it again.
#
#   packaging/apt/repack-version.sh <in.deb> <new-version> <out-dir>
#
# Writes <out-dir>/<Package>_<new-version>_<Architecture>.deb and prints its path. Packs as
# packaging/package-linux.sh does (`dpkg-deb --root-owner-group -Zxz --build`), with
# SOURCE_DATE_EPOCH kept. Refuses a version with a '-' (apt would order it wrongly) or one
# that does not sort after the old one (`dpkg --compare-versions <new> gt <old>`).
#
# Exit codes: 0 written; 1 refused or failed; 2 usage error.
#
# Needs: dpkg-deb and dpkg (package dpkg).
set -euo pipefail
umask 022
# Whatever fails without a sentence of its own ends the script with 1, never with the failing
# tool's own code (gpg's 2 would read as a usage error).
set -E
trap 'echo "$NAME: line $LINENO: a command failed" >&2; exit 1' ERR
NAME=repack-version.sh
if [ $# -ne 3 ]; then
  sed -n '5p' "$0" >&2
  exit 2
fi
IN=$1
NEW=$2
OUT=$3
fail() {
  echo "$NAME: $*" >&2
  exit 1
}

[ -f "$IN" ] || fail "$IN is not a file"
case $NEW in
  *-*) fail "the version $NEW holds a '-', which apt would order wrongly: use '+' to say more" ;;
  '' | [!0-9]* | *[!0-9A-Za-z.+~]*) fail "a version starts with a digit and holds only letters, digits, '.', '+' and '~', not: $NEW" ;;
esac
for tool in dpkg-deb dpkg; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done
PKG=$(dpkg-deb -f "$IN" Package) || fail "$IN is not a .deb that dpkg-deb can read"
OLD=$(dpkg-deb -f "$IN" Version)
ARCH=$(dpkg-deb -f "$IN" Architecture)
if [ -z "$PKG" ] || [ -z "$OLD" ] || [ -z "$ARCH" ]; then fail "$IN has no Package, Version or Architecture field"; fi
dpkg --compare-versions "$NEW" gt "$OLD" || fail "the version $NEW does not sort after $OLD, the version of $IN"

mkdir -p "$OUT"
TARGET=$OUT/${PKG}_${NEW}_${ARCH}.deb
[ ! -e "$TARGET" ] || fail "$TARGET exists already"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/yaar-repack.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
dpkg-deb -R "$IN" "$TMP/tree"
CONTROL=$TMP/tree/DEBIAN/control
[ "$(grep -c '^Version:' "$CONTROL")" -eq 1 ] || fail "$IN has not exactly one Version line in its control file"
awk -v new="$NEW" '/^Version:/ { print "Version: " new; next } { print }' "$CONTROL" > "$TMP/control"
cat "$TMP/control" > "$CONTROL"
dpkg-deb --root-owner-group -Zxz --build "$TMP/tree" "$TARGET" >&2
printf '%s\n' "$TARGET"
