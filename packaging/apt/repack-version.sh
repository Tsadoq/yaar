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
# What it writes is checked against its input before the path is printed, and the file is
# removed and the script ends with 1 unless the output holds exactly what the input holds but
# for the one field: the same list of files (`dpkg-deb -c`: names, modes, owners, sizes and
# times), the same bytes in each file, the same control members with the same modes and bytes,
# and a control file that differs only in its Version line, which is the new version. The
# sentence names the path or the field that differs. The output is removed too when any
# command after it is written fails (the script then ends with 1 and the line that failed).
#
# Exit codes: 0 written; 1 refused or failed; 2 usage error.
#
# Needs: dpkg-deb and dpkg (package dpkg), diff and find (GNU).
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
# Whatever ends the script before the path is printed, a refusal or a failed command, takes the
# output with it: a caller never finds a repack that was not checked.
DONE=0
cleanup() {
  rm -rf "$TMP"
  if [ "$DONE" -ne 1 ]; then rm -f "$TARGET"; fi
}
trap cleanup EXIT
dpkg-deb -R "$IN" "$TMP/tree"
CONTROL=$TMP/tree/DEBIAN/control
[ "$(grep -c '^Version:' "$CONTROL")" -eq 1 ] || fail "$IN has not exactly one Version line in its control file"
awk -v new="$NEW" '/^Version:/ { print "Version: " new; next } { print }' "$CONTROL" > "$TMP/control"
# The time the package was built at is kept: SOURCE_DATE_EPOCH from the environment when it is
# set, else the time of the input's control file (package-linux.sh gave it SOURCE_DATE_EPOCH),
# so that two repacks of one .deb are the same bytes.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  SOURCE_DATE_EPOCH=$(stat -c %Y "$CONTROL") || fail "cannot read the time of the control file of $IN"
fi
export SOURCE_DATE_EPOCH
cat "$TMP/control" > "$CONTROL"
touch -d "@$SOURCE_DATE_EPOCH" "$CONTROL"
dpkg-deb --root-owner-group -Zxz --build "$TMP/tree" "$TARGET" >&2

# The repack is the input with its Version changed and nothing else: a repack that lost the
# program, a dependency or a maintainer script would still be a package apt-get upgrade installs.
# same <file> <file> <what is compared>: ends the script, the output removed, when they differ.
same() {
  rc=0
  diff -r "$1" "$2" > "$TMP/diff.out" 2>&1 || rc=$?
  case $rc in
    0) return 0 ;;
    1)
      rm -f "$TARGET"
      fail "$TARGET is not $IN with only its Version changed: $3 differ: $(head -n 6 "$TMP/diff.out" | tr '\n' ' ')"
      ;;
    *) fail "diff cannot compare $3 of $IN and $TARGET: $(head -n 3 "$TMP/diff.out" | tr '\n' ' ')" ;;
  esac
}
dpkg-deb -c "$IN" > "$TMP/in.list"
dpkg-deb -c "$TARGET" > "$TMP/out.list"
same "$TMP/in.list" "$TMP/out.list" "the lists of files (dpkg-deb -c)"
mkdir "$TMP/in-data" "$TMP/out-data"
dpkg-deb -x "$IN" "$TMP/in-data"
dpkg-deb -x "$TARGET" "$TMP/out-data"
same "$TMP/in-data" "$TMP/out-data" "the files"
dpkg-deb -e "$IN" "$TMP/in-ctl"
dpkg-deb -e "$TARGET" "$TMP/out-ctl"
find "$TMP/in-ctl" -printf '%P %m\n' | LC_ALL=C sort > "$TMP/in-ctl.list"
find "$TMP/out-ctl" -printf '%P %m\n' | LC_ALL=C sort > "$TMP/out-ctl.list"
same "$TMP/in-ctl.list" "$TMP/out-ctl.list" "the control members and their modes"
GOT=$(dpkg-deb -f "$TARGET" Version)
if [ "$GOT" != "$NEW" ]; then rm -f "$TARGET"; fail "$TARGET holds version $GOT, not $NEW"; fi
for side in in out; do
  awk '/^Version:/ { next } { print }' "$TMP/$side-ctl/control" > "$TMP/$side-ctl/control.rest"
  rm "$TMP/$side-ctl/control"
done
same "$TMP/in-ctl" "$TMP/out-ctl" "the control members (the Version line of control left out)"
DONE=1
printf '%s\n' "$TARGET"
