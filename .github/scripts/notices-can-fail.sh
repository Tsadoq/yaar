#!/bin/sh
# The control half of row LIC-02: shows that packaging/notices-count.sh fails when it should,
# so that a count that has never refused anything is not taken for proof.
#
#   .github/scripts/notices-can-fail.sh <packages-dir>
#
# Works on copies under a temporary folder; never changes <packages-dir>. From the archive of
# <packages-dir> that ends in -linux-x86_64.tar.gz it makes, one at a time, a folder that
# holds only a broken copy of that archive, and runs notices-count.sh on it:
#   1. the "Used by:" line of one crate removed (the first crate of PART 1): must fail, and
#      its standard error must name that crate;
#   2. the licence text of one block emptied (the block that names that crate): must fail,
#      naming "no licence text";
#   3. a "Used by:" line added for a crate the lock file does not hold ("planted-crate 9.9.9"):
#      must fail, naming planted-crate;
#   4. the copy unchanged: must pass (the control: the archive as built counts).
# Then from the .zip of <packages-dir> (yaar-review-<V>-macos-universal.zip), in the same way,
# a folder holding only a copy whose YAAR.app/Contents/Resources/THIRD-PARTY-NOTICES.txt lost
# every "Used by:" line that names gpui-pre-apple (the toolkit's Apple crate, which the
# workspace patches in from vendor/ and which is third-party code all the same), unpacked
# with `unzip` and packed again with `python3 -m zipfile -c` (the job's container has no
# zip; the count reads only the notices file):
#   5. must fail, naming gpui-pre-apple;
#   6. the .zip unchanged: must pass.
# Each check prints what it did and what the count said.
#
# Exit codes: 0 all six as above; 1 one of them not (a sentence says which); 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu

NAME=notices-can-fail.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}

[ $# -eq 1 ] || { echo "usage: .github/scripts/notices-can-fail.sh <packages-dir>" >&2; exit 2; }
DIR=$1
[ -d "$DIR" ] || { echo "$NAME: $DIR is not a folder" >&2; exit 2; }

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
COUNT=$ROOT/packaging/notices-count.sh
[ -x "$COUNT" ] || fail "$COUNT is not an executable file"
for tool in cargo jq gzip tar unzip awk python3 cmp; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is needed and is not on the path"
done

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

ARCHIVE_MEMBER=yaar-review/share/doc/yaar-review/THIRD-PARTY-NOTICES.txt
ZIP_MEMBER=YAAR.app/Contents/Resources/THIRD-PARTY-NOTICES.txt

# The one archive and the one .zip the checks break (the folder may hold more).
ARCHIVE=
ZIP=
for f in "$DIR"/yaar-review-*-linux-x86_64.tar.gz; do [ -f "$f" ] && ARCHIVE=$f; done
for f in "$DIR"/yaar-review-*-macos-universal.zip; do [ -f "$f" ] && ZIP=$f; done
[ -n "$ARCHIVE" ] || fail "$DIR holds no yaar-review-<V>-linux-x86_64.tar.gz to break"
[ -n "$ZIP" ] || fail "$DIR holds no yaar-review-<V>-macos-universal.zip to break"

# The workspace's own crates, which the count leaves out: breaking a line of one of them
# would prove nothing.
(cd "$ROOT" && cargo metadata --locked --offline --no-deps --format-version 1) \
  > "$TMP/metadata.json" 2> "$TMP/metadata.err" \
  || { cat "$TMP/metadata.err" >&2; fail "cargo metadata of $ROOT failed"; }
jq -r '.packages[].name' "$TMP/metadata.json" > "$TMP/workspace"

# Unpacks the notices file of the archive $1 into the folder $2 (the whole archive).
unpack_archive() {
  mkdir -p "$2"
  tar -xzf "$1" -C "$2" || fail "$(basename "$1") cannot be unpacked"
  [ -f "$2/$ARCHIVE_MEMBER" ] || fail "$(basename "$1") holds no $ARCHIVE_MEMBER"
}
unpack_zip() {
  mkdir -p "$2"
  unzip -q "$1" -d "$2" || fail "$(basename "$1") cannot be unpacked"
  [ -f "$2/$ZIP_MEMBER" ] || fail "$(basename "$1") holds no $ZIP_MEMBER"
}
# Packs the folder $2 again as the archive $1 and as the .zip $1.
pack_archive() {
  tar -czf "$1" -C "$2" yaar-review || fail "the broken copy of the archive cannot be packed"
}
pack_zip() {
  (cd "$2" && python3 -m zipfile -c "$1" YAAR.app) || fail "the broken copy of the .zip cannot be packed"
}

# The crate this script breaks: the first of PART 1 that is not the workspace's own and
# whose "name version" stands in that part once, so that taking it out leaves no entry.
# Prints "name version".
first_crate() {
  awk '
    /^PART [0-9]+ OF 3: / { part = $2; next }
    part == 1 && prev == "Used by:" { reading = 1 }
    part == 1 && reading { if ($0 ~ /^  [^ ]/) { print $1, $2; next } else reading = 0 }
    { prev = $0 }' "$1" > "$TMP/part1.crates"
  awk '
    FNR == 1 { file++ }
    file == 1 { ws[$1] = 1; next }
    file == 2 { seen[$0]++; next }
    { split($0, w, " "); if (!(w[1] in ws) && seen[$0] == 1) { print; exit } }' \
    "$TMP/workspace" "$TMP/part1.crates" "$TMP/part1.crates"
}

# The notices file with the one line "  <crate> <version>" of PART 1 taken out.
without_line() { # file crate-version
  awk -v want="$2" '
    /^PART [0-9]+ OF 3: / { part = $2 }
    part == 1 && !done && $0 ~ /^  [^ ]/ && ($1 " " $2) == want { done = 1; next }
    { print }' "$1"
}
# The notices file with the text of the licence block that names <crate> <version> taken
# out, down to the rule that opens the next block.
without_text() { # file crate-version
  awk -v want="$2" '
    function rule() { return length($0) == 80 && ($0 ~ /^-+$/ || $0 ~ /^=+$/) }
    /^PART [0-9]+ OF 3: / { part = $2 }
    rule() { dropping = 0; reading = 0; print; next }
    dropping { next }
    part == 1 && $0 == "Used by:" { reading = 1; hit = 0; print; next }
    reading {
      if ($0 ~ /^  [^ ]/) { if (($1 " " $2) == want) hit = 1 }
      else { reading = 0; if (hit) dropping = 1 }
      print; next
    }
    { print }' "$1"
}
# The notices file with a "Used by:" line for a crate the lock file does not hold added to
# the first block of PART 1.
with_planted() { # file
  awk '
    /^PART [0-9]+ OF 3: / { part = $2 }
    { print }
    part == 1 && $0 == "Used by:" && !done { done = 1; print "  planted-crate 9.9.9" }' "$1"
}
# The notices file without any "Used by:" line that names gpui-pre-apple.
without_toolkit_apple() { # file
  awk '!/^  gpui-pre-apple /' "$1"
}

# Runs the count on the folder $1 and keeps what it said in $TMP/said and its exit code in
# $RC.
count() {
  RC=0
  "$COUNT" "$1" > "$TMP/said" 2>&1 || RC=$?
}
show() { # what was done
  echo "$NAME: $1"
  if [ -s "$TMP/said" ]; then
    awk 'NR <= 4 { print "    the count said: " $0 }' "$TMP/said"
  fi
}
# Check that the count failed (exit 1) and said every word given.
must_fail() { # description word...
  desc=$1; shift
  show "$desc"
  [ "$RC" = 1 ] || fail "the count exited $RC where it must exit 1: $desc"
  for word in "$@"; do
    grep -q -F -e "$word" "$TMP/said" || fail "the count failed but does not name '$word': $desc"
  done
}
must_pass() { # description
  show "$1"
  [ "$RC" = 0 ] || fail "the count exited $RC on a copy that was not broken: $1"
}

# One broken copy of the archive: its notices file made by the function $2 from the
# original, the archive packed again under its own name in the folder $1, and counted.
break_archive() { # folder transform
  folder=$TMP/$1 transform=$2
  mkdir -p "$folder.x"
  unpack_archive "$ARCHIVE" "$folder.x"
  mkdir -p "$folder"
  "$transform" "$folder.x/$ARCHIVE_MEMBER" "$CRATE" > "$folder.x/notices.broken"
  cmp -s "$folder.x/notices.broken" "$folder.x/$ARCHIVE_MEMBER" \
    && fail "the break of $transform changed nothing in the notices of $(basename "$ARCHIVE")"
  cp "$folder.x/notices.broken" "$folder.x/$ARCHIVE_MEMBER"
  rm "$folder.x/notices.broken"
  pack_archive "$folder/$(basename "$ARCHIVE")" "$folder.x"
  count "$folder"
}

unpack_archive "$ARCHIVE" "$TMP/orig"
CRATE=$(first_crate "$TMP/orig/$ARCHIVE_MEMBER")
[ -n "$CRATE" ] || fail "$(basename "$ARCHIVE"): PART 1 of its notices names no crate to break"
NAME_OF_CRATE=${CRATE%% *}

break_archive one without_line
must_fail "1. the line of $CRATE taken out of PART 1 of $(basename "$ARCHIVE")" "$NAME_OF_CRATE"

break_archive two without_text
must_fail "2. the licence text of the block that names $CRATE emptied in $(basename "$ARCHIVE")" "no licence text"

break_archive three with_planted
must_fail "3. a line for planted-crate 9.9.9 added to PART 1 of $(basename "$ARCHIVE")" "planted-crate"

mkdir "$TMP/four"
cp "$ARCHIVE" "$TMP/four/"
count "$TMP/four"
must_pass "4. $(basename "$ARCHIVE") unchanged"

# The .zip: every line that names gpui-pre-apple taken out.
unpack_zip "$ZIP" "$TMP/zip.x"
without_toolkit_apple "$TMP/zip.x/$ZIP_MEMBER" > "$TMP/zip.broken"
cmp -s "$TMP/zip.broken" "$TMP/zip.x/$ZIP_MEMBER" \
  && fail "$(basename "$ZIP"): its notices have no \"Used by:\" line that names gpui-pre-apple, so nothing was broken"
cp "$TMP/zip.broken" "$TMP/zip.x/$ZIP_MEMBER"
mkdir "$TMP/five"
pack_zip "$TMP/five/$(basename "$ZIP")" "$TMP/zip.x"
count "$TMP/five"
must_fail "5. every line that names gpui-pre-apple taken out of $(basename "$ZIP")" "gpui-pre-apple"

mkdir "$TMP/six"
cp "$ZIP" "$TMP/six/"
count "$TMP/six"
must_pass "6. $(basename "$ZIP") unchanged"

echo "$NAME: the count fails when a crate has no entry, when a block has no text, when an entry is for a crate that is not linked and when the toolkit's Apple crate is missing, and passes on both packages as built"
