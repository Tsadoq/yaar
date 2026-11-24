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
#   7. from the same archive, the licence text of the last block of the file (the last of
#      PART 3) emptied: must fail, naming PART 3 and "no licence text" (a count that
#      forgets the block that no rule closes passes this one).
# Last, on the archive unchanged, the count reads a broken copy of .github/scripts/release-build.sh
# (through RELEASE_BUILD_SH), which builds the programs the way the notices must not be made
# for:
#   8. the three cargo invocations made one that builds all four packages, so that Cargo
#      merges their features: must fail, naming "one cargo invocation";
#   9. the invocation of yaar-companion-claude taken out: must fail, naming
#      "no cargo invocation of its own builds yaar-companion-claude";
#  10. the invocation of yaar-window made "cargo build --release --locked --workspace": must
#      fail, naming --workspace, which the count cannot read;
#  11. the three cargo invocations kept and a fourth line added after them, a merged build of
#      the four packages behind an env assignment (CARGO_TERM_COLOR=never cargo build ...):
#      must fail, naming that line, which the count cannot read (a reader that looks only for
#      lines that begin with `cargo build` passes it);
#  12. the same fourth line with cargo called by a path, "$HOME/.cargo/bin/cargo" build ...:
#      must fail, naming that line (a reader that takes a word for cargo only when it is
#      exactly `cargo` passes it);
#  13. the three cargo invocations kept and the line set -- "$@" --workspace added after
#      `cd "$ROOT"`, so that each "$@" on them is no longer the caller's arguments and each
#      invocation builds the whole workspace: must fail, naming that line (a reader that
#      reads "$@" on the cargo lines and does not look at a line that changes it passes it).
# And on every package of <packages-dir> that carries notices, unchanged (the .debs, the
# archives of both Linux machines and the .zip; the archive that ends in
# -linux-aarch64.tar.gz must be there):
#  14. the count made plainly, and then twice more with cargo shown another machine (RUSTC a
#      stand-in that reports x86_64-unknown-linux-gnu, then aarch64-unknown-linux-gnu, as
#      the host and answers only cargo's questions): each must pass and print the same
#      lines as the plain count (cargo works out a proc-macro's dependencies for the machine
#      it runs on; a count that lets the machine through wants, on an x86-64 machine, a
#      crate that no aarch64 package links, and fails there on notices that are right).
# Each check prints what it did and what the count said.
#
# Exit codes: 0 all fourteen as above; 1 one of them not (a sentence says which); 2 usage error.
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
ARM_ARCHIVE=
for f in "$DIR"/yaar-review-*-linux-x86_64.tar.gz; do [ -f "$f" ] && ARCHIVE=$f; done
for f in "$DIR"/yaar-review-*-macos-universal.zip; do [ -f "$f" ] && ZIP=$f; done
for f in "$DIR"/yaar-review-*-linux-aarch64.tar.gz; do [ -f "$f" ] && ARM_ARCHIVE=$f; done
[ -n "$ARCHIVE" ] || fail "$DIR holds no yaar-review-<V>-linux-x86_64.tar.gz to break"
[ -n "$ZIP" ] || fail "$DIR holds no yaar-review-<V>-macos-universal.zip to break"
[ -n "$ARM_ARCHIVE" ] || fail "$DIR holds no yaar-review-<V>-linux-aarch64.tar.gz, which check 14 counts as if on another machine"

# The workspace's own crates, which the count leaves out: breaking a line of one of them
# would prove nothing.
(cd "$ROOT" && cargo metadata --locked --offline --no-deps --format-version 1) \
  > "$TMP/metadata.json" 2> "$TMP/metadata.err" \
  || { cat "$TMP/metadata.err" >&2; fail "cargo metadata of $ROOT failed"; }
jq -r '.packages[].name' "$TMP/metadata.json" > "$TMP/workspace" \
  || fail "jq cannot read the packages of cargo metadata"

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
    { prev = $0 }' "$1" > "$TMP/part1.crates" || return 1
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
# The notices file with the text of its last licence block taken out: everything after the
# "Used by:" lines of the last "Used by:" in the file, which no rule closes.
without_last_text() { # file crate-version (unused)
  awk '
    NR == FNR {
      if ($0 == "Used by:") { end = FNR; reading = 1; next }
      if (reading) { if ($0 ~ /^  [^ ]/) end = FNR; else reading = 0 }
      next
    }
    FNR <= end { print }' "$1" "$1"
}
# release-build.sh with its three cargo invocations made one, which builds the four packages.
merged_build() { # file
  awk '
    /^[ \t]*cargo build / { if (!done) { done = 1; print "cargo build --release --locked -p yaar-window -p yaar-keeper -p yaar-adapter-pi -p yaar-companion-claude \"$@\"" } ; next }
    { print }' "$1"
}
# release-build.sh without the invocation that names yaar-companion-claude.
build_without_companion() { # file
  awk '/^[ \t]*cargo build / && /yaar-companion-claude/ { next } { print }' "$1"
}
# release-build.sh with the invocation of yaar-window made one that names no package.
build_of_workspace() { # file
  awk '/^[ \t]*cargo build / && /yaar-window/ { print "cargo build --release --locked --workspace \"$@\""; next } { print }' "$1"
}
# release-build.sh with a fourth cargo line after the three: a merged build that an env
# assignment in front of `cargo` hides from a reader that looks for lines beginning with it.
build_with_env_merged() { # file
  cat "$1"
  echo 'CARGO_TERM_COLOR=never cargo build --release --locked -p yaar-window -p yaar-keeper -p yaar-adapter-pi -p yaar-companion-claude "$@"'
}
# release-build.sh with a fourth cargo line after the three: a merged build whose cargo is
# called by a path, which a reader that takes only the word `cargo` for cargo does not see.
build_with_path_merged() { # file
  cat "$1"
  # The line is written as it stands, $HOME unexpanded: it is a line of a script, not of this one.
  # shellcheck disable=SC2016
  echo '"$HOME/.cargo/bin/cargo" build --release --locked -p yaar-window -p yaar-keeper -p yaar-adapter-pi -p yaar-companion-claude "$@"'
}
# release-build.sh with a line after `cd "$ROOT"` that puts --workspace after the arguments of
# the caller, so that "$@" on each cargo line carries it too.
build_with_set_args() { # file
  # The line is written as it stands, "$@" unexpanded: it is a line of a script, not of this one.
  awk '{ print } /^cd "\$ROOT"$/ { print "set -- \"$@\" --workspace" }' "$1"
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

# The build script the count reads: the repository's, or the broken copy a check sets.
BUILD=$ROOT/.github/scripts/release-build.sh
[ -f "$BUILD" ] || fail "$BUILD is not a file"

# Runs the count on the folder $1 and keeps what it said in $TMP/said and its exit code in
# $RC.
count() {
  RC=0
  env "RELEASE_BUILD_SH=$BUILD" "$COUNT" "$1" > "$TMP/said" 2>&1 || RC=$?
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

# Fails with the sentence $3 when the files $1 and $2 are the same; cmp says 0 for the same,
# 1 for different and anything else when it could not read one of them.
changed() { # file file sentence-when-the-same
  same=0
  cmp -s "$1" "$2" || same=$?
  case "$same" in
    0) fail "$3" ;;
    1) return 0 ;;
    *) fail "cmp cannot compare $1 with $2" ;;
  esac
}

# One broken copy of the archive: its notices file made by the function $2 from the
# original, the archive packed again under its own name in the folder $1, and counted.
break_archive() { # folder transform
  folder=$TMP/$1 transform=$2
  mkdir -p "$folder.x"
  unpack_archive "$ARCHIVE" "$folder.x"
  mkdir -p "$folder"
  "$transform" "$folder.x/$ARCHIVE_MEMBER" "$CRATE" > "$folder.x/notices.broken" \
    || fail "$transform cannot break the notices of $(basename "$ARCHIVE")"
  changed "$folder.x/notices.broken" "$folder.x/$ARCHIVE_MEMBER" \
    "the break of $transform changed nothing in the notices of $(basename "$ARCHIVE")"
  cp "$folder.x/notices.broken" "$folder.x/$ARCHIVE_MEMBER"
  rm "$folder.x/notices.broken"
  pack_archive "$folder/$(basename "$ARCHIVE")" "$folder.x"
  count "$folder"
}

unpack_archive "$ARCHIVE" "$TMP/orig"
CRATE=$(first_crate "$TMP/orig/$ARCHIVE_MEMBER") \
  || fail "$(basename "$ARCHIVE"): PART 1 of its notices cannot be read"
[ -n "$CRATE" ] || fail "$(basename "$ARCHIVE"): PART 1 of its notices names no crate to break"

break_archive one without_line
must_fail "1. the line of $CRATE taken out of PART 1 of $(basename "$ARCHIVE")" "no entry for $CRATE"

break_archive two without_text
must_fail "2. the licence text of the block that names $CRATE emptied in $(basename "$ARCHIVE")" "no licence text"

break_archive three with_planted
must_fail "3. a line for planted-crate 9.9.9 added to PART 1 of $(basename "$ARCHIVE")" "an entry for planted-crate 9.9.9, which is not linked"

mkdir "$TMP/four"
cp "$ARCHIVE" "$TMP/four/"
count "$TMP/four"
must_pass "4. $(basename "$ARCHIVE") unchanged"

# The .zip: every line that names gpui-pre-apple taken out.
unpack_zip "$ZIP" "$TMP/zip.x"
without_toolkit_apple "$TMP/zip.x/$ZIP_MEMBER" > "$TMP/zip.broken" \
  || fail "$(basename "$ZIP"): its notices cannot be read"
changed "$TMP/zip.broken" "$TMP/zip.x/$ZIP_MEMBER" \
  "$(basename "$ZIP"): its notices have no \"Used by:\" line that names gpui-pre-apple, so nothing was broken"
cp "$TMP/zip.broken" "$TMP/zip.x/$ZIP_MEMBER"
mkdir "$TMP/five"
pack_zip "$TMP/five/$(basename "$ZIP")" "$TMP/zip.x"
count "$TMP/five"
must_fail "5. every line that names gpui-pre-apple taken out of $(basename "$ZIP")" "no entry for gpui-pre-apple"

mkdir "$TMP/six"
cp "$ZIP" "$TMP/six/"
count "$TMP/six"
must_pass "6. $(basename "$ZIP") unchanged"

break_archive seven without_last_text
must_fail "7. the licence text of the last block of the file emptied in $(basename "$ARCHIVE")" "PART 3" "no licence text"

# The count of the unchanged archive, with a build script that is not as it must be.
break_build() { # number description transform words...
  num=$1 desc=$2 transform=$3; shift 3
  "$transform" "$ROOT/.github/scripts/release-build.sh" > "$TMP/build.$num" \
    || fail "$transform cannot break .github/scripts/release-build.sh"
  changed "$TMP/build.$num" "$ROOT/.github/scripts/release-build.sh" \
    "the break of $transform changed nothing in .github/scripts/release-build.sh"
  BUILD=$TMP/build.$num
  count "$TMP/four"
  BUILD=$ROOT/.github/scripts/release-build.sh
  must_fail "$num. $desc" "$@"
}

break_build 8 "release-build.sh builds the four packages in one cargo invocation" merged_build "one cargo invocation"
break_build 9 "release-build.sh does not build yaar-companion-claude" build_without_companion "no cargo invocation of its own builds yaar-companion-claude"
break_build 10 "release-build.sh builds the workspace for the Window" build_of_workspace "--workspace"
break_build 11 "release-build.sh has a merged build of the four packages behind an env assignment" build_with_env_merged "CARGO_TERM_COLOR=never cargo build --release --locked -p yaar-window -p yaar-keeper -p yaar-adapter-pi -p yaar-companion-claude"
break_build 12 "release-build.sh has a merged build of the four packages that calls cargo by a path" build_with_path_merged "\"\$HOME/.cargo/bin/cargo\" build --release --locked -p yaar-window -p yaar-keeper -p yaar-adapter-pi -p yaar-companion-claude"
break_build 13 "release-build.sh changes \"\$@\" with set -- before its cargo lines" build_with_set_args "set -- \"\$@\" --workspace"

# Check 14: the count of the packages as built does not depend on the machine that runs it.
# The folder $TMP/hosts holds a link to each package that carries notices; the count of it
# is made plainly, and then with RUSTC a stand-in that reports FAKE_HOST as the machine's
# host and answers only the questions cargo asks (its version, its cfg, its file names), and
# with a target folder of its own, so that the repository's is not touched.
REAL_RUSTC=${RUSTC:-rustc}
HOST_RUSTC=$TMP/host-rustc.sh
cat > "$HOST_RUSTC" << 'STAND_IN'
#!/bin/sh
case " $* " in
  *" -vV "*) "$FAKE_REAL_RUSTC" "$@" | sed "s/^host: .*/host: $FAKE_HOST/"; exit ;;
  *" --target "* | *" --target="*) exec "$FAKE_REAL_RUSTC" "$@" ;;
  *--print*) exec "$FAKE_REAL_RUSTC" "$@" --target "$FAKE_HOST" ;;
esac
echo "host-rustc.sh: only cargo's questions are answered: $*" >&2
exit 3
STAND_IN
chmod +x "$HOST_RUSTC"
mkdir "$TMP/hosts"
DIR_ABS=$(cd "$DIR" && pwd)
for f in "$DIR_ABS"/yaar-review_*_amd64.deb "$DIR_ABS"/yaar-review_*_arm64.deb \
         "$DIR_ABS"/yaar-review-*-linux-x86_64.tar.gz "$DIR_ABS"/yaar-review-*-linux-aarch64.tar.gz \
         "$DIR_ABS"/yaar-review-*-macos-universal.zip; do
  [ -f "$f" ] && ln -s "$f" "$TMP/hosts/"
done
count "$TMP/hosts"
must_pass "14. every package of $(basename "$DIR_ABS") counted plainly, on this machine"
cp "$TMP/said" "$TMP/said.plain"
for host in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  RC=0
  env "RELEASE_BUILD_SH=$BUILD" "RUSTC=$HOST_RUSTC" "FAKE_REAL_RUSTC=$REAL_RUSTC" "FAKE_HOST=$host" "CARGO_TARGET_DIR=$TMP/host-target-$host" \
    "$COUNT" "$TMP/hosts" > "$TMP/said" 2>&1 || RC=$?
  show "14. the same packages counted as on a machine whose host is $host"
  [ "$RC" = 0 ] || fail "the count exited $RC when cargo took this machine for $host, and 0 on this machine as it is: the count depends on the machine that runs it (a proc-macro's dependencies are worked out for the machine, not for the target)"
  same=0
  cmp -s "$TMP/said.plain" "$TMP/said" || same=$?
  case "$same" in
    0) ;;
    1) fail "the count passed when cargo took this machine for $host but printed other lines than on this machine as it is" ;;
    *) fail "cmp cannot compare the lines of the count" ;;
  esac
done

echo "$NAME: the count fails when a crate has no entry, when a block has no text, when the last block of the file has no text, when an entry is for a crate that is not linked, when the toolkit's Apple crate is missing and when the programs are not built part by part, a cargo line cannot be read or a line changes the arguments of the cargo lines, and passes on both packages as built, whichever machine counts them"
