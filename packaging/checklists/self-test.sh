#!/bin/bash
# Proves the scripts of this folder that open nothing: conventions.sh (with a file of
# answers, and with --dry-run) and serve-release.sh (on a hand-made folder laid out as the
# artifact release-final, on a free port, stopped by its pid); the dry runs of the three
# other walkers; and what the measuring scripts only read (the logs of an earlier run, made
# up here). The scripts that open the app's window or post input are not run by anything but
# the owner's word.
#
#   packaging/checklists/self-test.sh
#
# What it checks, one sentence each when it fails:
#   conventions.sh   the 17 items of macOS and the 11 of Linux are each printed once, with
#                    their position; the answers of a file land in the Markdown file with the
#                    note of a no and the reason of a skip; a "no" or a skip leaves the
#                    sign-off unsigned and names the items; all yes asks for the signature;
#                    answers that end early stop it with a sentence and write no file; a
#                    wrong command line is exit 2; --dry-run calls no program of the system
#                    and the app (stand-ins for them record every call), and writes its
#                    own file; items 11 and 16 print their light theme steps; a pipe in a
#                    note stays in its cell; a walker that does not end in 20 s is stopped
#                    by its pid and fails the test; nothing named yaar-checklists* is made
#                    or changed in the real home (HOME, TMPDIR and PATH of the runs are in
#                    this script's scratch).
#   serve-release.sh the install and uninstall lines are printed with the server's own
#                    address and no test variable; the site is served at / and the release
#                    files at /download/, byte for byte; the server listens on 127.0.0.1
#                    only; a second server on the same port, a folder that is not laid out
#                    as release-final and a file that disagrees with SHA256SUMS are
#                    refused with a sentence, as are a listed file that is missing and a
#                    SHA256SUMS that lists nothing; stopped by the pid it printed, the
#                    script and its server are gone, the port refuses, and no scratch is
#                    left.
#   the walkers      screen-walk.sh, real-clis.sh and linux-hardware.sh print their items
#                    once each in a dry run and call no program of the system.
#   the measures     typing.sh, scroll.sh and start-time.sh read made-up logs: key to
#                    screen times (early, 13 ms, 399 ms, 401 ms, the median of an even and
#                    of an odd count, the 95th percentile below the largest), frames a
#                    second and long gaps, the marks of a start; none leaves anything in
#                    TMPDIR; start-time.sh --linux gives 1 for any status but 0 and 1 of
#                    the machine's run (a stand-in for the machine's script, on the Mac).
#   programs         a program is found by its full path, not its name; a new copy among
#                    known ones is found; a process of another user is not this user's
#                    (launchd's on the Mac; a copy run as nobody, by root, on Linux).
#
# Needs of the machine: curl, lsof, awk, cmp. Touches nothing outside its scratch folder
# (`mktemp -d`, removed at the end); the only network is 127.0.0.1.
#
# Exit codes: 0 every check holds; 1 one does not (the sentence names it); 2 usage error.
set -euo pipefail

CK_NAME=self-test.sh
HERE=$(cd "$(dirname "$0")" && pwd)

if [ "$#" -ne 0 ]; then
  printf '%s: it takes no argument\n' "$CK_NAME" >&2
  exit 2
fi
for tool in curl lsof awk cmp; do
  command -v "$tool" > /dev/null || {
    printf '%s: the tool %s is missing, which these checks need\n' "$CK_NAME" "$tool" >&2
    exit 1
  }
done

REAL_HOME=$HOME
S=$(mktemp -d "${TMPDIR:-/tmp}/yaar-checklists-self-test.XXXXXX")
SERVERS=""
cleanup() {
  local pid
  for pid in $SERVERS; do
    if kill -0 "$pid" 2> /dev/null; then
      kill "$pid" 2> /dev/null || true
    fi
  done
  rm -rf "$S"
}
trap cleanup EXIT

fail() {
  printf '%s: FAIL: %s\n' "$CK_NAME" "$1" >&2
  exit 1
}

pass() {
  printf '%s: ok: %s\n' "$CK_NAME" "$1"
}

wait_for() {
  local deadline=$((SECONDS + 20))
  until "$@"; do
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 0.2
  done
}

# refuse <what> <out file> <err file> <serve-release.sh arguments>: runs it detached and
# waits for it to end by itself; one that is still serving after 20 s did not refuse. The
# status is left in REFUSAL_STATUS.
refuse() {
  local what=$1 out=$2 err=$3
  shift 3
  "$HERE/serve-release.sh" "$@" > "$out" 2> "$err" < /dev/null &
  local pid=$!
  SERVERS="$SERVERS $pid"
  has_ended() { ! kill -0 "$pid" 2> /dev/null; }
  if ! wait_for has_ended; then
    kill "$pid"
    fail "$what: serve-release.sh did not refuse, it was still running after 20 s"
  fi
  if wait "$pid"; then
    REFUSAL_STATUS=0
  else
    REFUSAL_STATUS=$?
  fi
}

# Everything in the real home whose name starts with yaar-checklists (the results folder is
# the one place these scripts write to, and a script that took the real home by another name
# would show here), with the checksum of each file: a file made, changed or removed shows. The
# rest of the real home is other work's and is not looked at.
real_state() {
  local found
  found=$(find "$REAL_HOME" -maxdepth 1 -name 'yaar-checklists*' 2> /dev/null | sort)
  if [ -z "$found" ]; then
    echo none
    return 0
  fi
  printf '%s\n' "$found" | while IFS= read -r top; do
    find "$top"
    find "$top" -type f -exec cksum {} +
  done | sort | cksum
}
real_state_before=$(real_state)

mkdir -p "$S/home" "$S/tmp" "$S/bin"
export HOME="$S/home" TMPDIR="$S/tmp"
TODAY=$(date +%Y-%m-%d)
RESULTS="$HOME/yaar-checklists/$TODAY"

# Stand-ins for what a run that is not a dry run asks of the system. Each records that it
# was called; a dry run must call none of them.
cat > "$S/bin/sw_vers" << EOF
#!/bin/sh
echo called >> "$S/calls"
echo "26.0 (TESTBUILD)"
EOF
cat > "$S/bin/yaar-review" << EOF
#!/bin/sh
echo called >> "$S/calls"
echo "yaar-review 9.9.9"
EOF
chmod 755 "$S/bin/sw_vers" "$S/bin/yaar-review"
# The other programs a dry run could reach for. Each only records that it was called, with
# its arguments, in the file `calls`, and does nothing else.
for spy in uname lspci cargo curl sudo apt apt-get dpkg foot open gio ssh; do
  printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\n' "$spy" "$S" > "$S/bin/$spy"
  chmod 755 "$S/bin/$spy"
done

# no_calls <what>: no stand-in was called by the run just made.
no_calls() {
  [ ! -e "$S/calls" ] || fail "$1 called a program of the system or of the app (calls: $(tr '\n' ';' < "$S/calls"))"
}

# tmp_clean <what>: the run just made left nothing in TMPDIR, which is this test's own.
tmp_clean() {
  local left
  left=$(find "$TMPDIR" -mindepth 1 -maxdepth 1 | head -n 1)
  [ -z "$left" ] || fail "$1 left $left in its TMPDIR: its scratch is not removed when it ends"
}

# run_walker <what> <out file> <err file> <answers file> <command...>: runs a script with
# its answers from a file and waits for it to end by itself; one that is still running after
# 20 s is stopped by its pid and the test fails with the end of what it said. A walker that
# loops on a line it cannot take would otherwise hang the test and fill the disk, so it may
# not write a file of more than 40 MB. The status is left in WALK_STATUS.
run_walker() {
  local what=$1 out=$2 err=$3 answers=$4
  shift 4
  ( ulimit -f 40960; exec "$@" ) < "$answers" > "$out" 2> "$err" &
  local pid=$!
  SERVERS="$SERVERS $pid"
  walker_ended() { ! kill -0 "$pid" 2> /dev/null; }
  if ! wait_for walker_ended; then
    kill "$pid" 2> /dev/null || true
    wait "$pid" 2> /dev/null || true
    fail "$what did not end in 20 s, it waits or loops (its last words: $(tail -c 300 "$err" | tr '\n' ' '))"
  fi
  if wait "$pid"; then
    WALK_STATUS=0
  else
    WALK_STATUS=$?
  fi
}

lines() {
  local n=$1 word=$2 i=0
  while [ "$i" -lt "$n" ]; do
    printf '%s\n' "$word"
    i=$((i + 1))
  done
}

# ---------------------------------------------------------------------------------------
# conventions.sh
# ---------------------------------------------------------------------------------------

conv() {
  # conv <list> <answers file> <output file> [more arguments]: the status in $CONV_STATUS.
  local list=$1 answers=$2 out=$3
  shift 3
  run_walker "conventions.sh $list" "$out" "$out.err" "$answers" \
    env PATH="$S/bin:$PATH" YAAR_REVIEW="$S/bin/yaar-review" "$HERE/conventions.sh" "$list" "$@"
  CONV_STATUS=$WALK_STATUS
}

# macOS, 17 items: 1 yes, 2 note then yes, 3 no with a reason, 4 to 8 yes, 9 skip with a
# reason, 10 to 17 yes.
{
  echo yes
  echo note
  echo "the quit leaves the window a moment"
  echo yes
  echo no
  echo "Cmd-M does nothing in a full screen window"
  lines 5 yes
  echo skip
  echo "no Dock on this screen"
  lines 8 yes
} > "$S/answers-mixed.txt"

conv macos "$S/answers-mixed.txt" "$S/out-mixed.txt" --dry-run
[ "$CONV_STATUS" -eq 0 ] || fail "conventions.sh macos --dry-run with 17 answers did not exit 0 (it said: $(cat "$S/out-mixed.txt.err"))"

for n in $(seq 1 17); do
  [ "$(grep -c "^Item $n of 17: " "$S/out-mixed.txt")" -eq 1 ] \
    || fail "conventions.sh macos did not print 'Item $n of 17' exactly once"
done
[ "$(grep -c '^Item ' "$S/out-mixed.txt")" -eq 17 ] || fail "conventions.sh macos printed other than 17 items"
pass "conventions.sh macos prints its 17 items once each, with their place"

# item_block <output file> <n> <list size>: the lines of item <n>, up to the next item.
item_block() {
  awk -v from="^Item $2 of $3: " '$0 ~ from { on = 1; next } on && /^Item / { exit } on { print }' "$1"
}
item_block "$S/out-mixed.txt" 11 17 > "$S/item11.txt"
item_block "$S/out-mixed.txt" 16 17 > "$S/item16.txt"
for step in 'choose Light while the system is light' 'choose "as the system"' 'switch the system to dark' \
  'Dark chosen while the system is dark' 'a theme name that no theme has' 'set the system to dark, start the app' \
  'first frame is already dark'; do
  grep -qF "$step" "$S/item11.txt" || fail "item 11 of conventions.sh macos does not print the step '$step'"
done
for step in 'With the light theme on' 'added row, a removed row and a changed word' 'the sixteen colours' \
  'empty cells and at the space right of its last column' 'its empty cells and the space right of the last column show the terminal'; do
  grep -qF "$step" "$S/item16.txt" || fail "item 16 of conventions.sh macos does not print the step '$step'"
done
pass "items 11 and 16 carry the light theme steps"

OUT="$RESULTS/conventions-macos-dry-run.md"
[ -f "$OUT" ] || fail "conventions.sh macos --dry-run wrote no $OUT"
[ "$(grep -c '^| [0-9]* |' "$OUT")" -eq 17 ] || fail "$OUT does not hold 17 rows of answers"
grep -q "^- Date: $TODAY\$" "$OUT" || fail "$OUT does not name the date $TODAY"
grep -qi 'dry run' "$OUT" || fail "$OUT does not say that it is a dry run"
grep -q '^| 3 | .* | no | Cmd-M does nothing in a full screen window |$' "$OUT" \
  || fail "$OUT does not hold item 3 as no, with its reason"
grep -q '^| 2 | .* | yes | the quit leaves the window a moment |$' "$OUT" \
  || fail "$OUT does not hold the note of item 2 on a yes"
grep -q '^| 9 | .* | skipped | no Dock on this screen |$' "$OUT" \
  || fail "$OUT does not hold item 9 as skipped, with its reason"
grep -q '^Not signed. Items answered no or not walked: 3, 9\.$' "$OUT" \
  || fail "$OUT does not name items 3 and 9 as the ones not signed"
pass "the answers land in conventions-macos-dry-run.md, with notes, and the sign-off names 3 and 9"

[ ! -e "$S/calls" ] || fail "conventions.sh --dry-run called a program of the system or of the app"
[ ! -e "$RESULTS/conventions-macos.md" ] || fail "a dry run wrote the real sign-off file conventions-macos.md"
pass "a dry run asks nothing of the system and leaves the real file name alone"

# All yes: the signature is asked and given.
{
  lines 17 yes
  echo yes
} > "$S/answers-yes.txt"
conv macos "$S/answers-yes.txt" "$S/out-yes.txt" --dry-run
[ "$CONV_STATUS" -eq 0 ] || fail "conventions.sh macos with all yes did not exit 0 (it said: $(cat "$S/out-yes.txt.err"))"
grep -q '^Signed: I accept these as following the platform (CONV-01, macOS)\.$' "$OUT" \
  || fail "all yes and a yes to the signature did not write the signed line"
pass "all yes, then yes, signs"

# The same, but the signature is refused with a reason.
{
  lines 17 yes
  echo no
  echo "I want to look at item 16 again"
} > "$S/answers-nosign.txt"
conv macos "$S/answers-nosign.txt" "$S/out-nosign.txt" --dry-run
[ "$CONV_STATUS" -eq 0 ] || fail "a refused signature did not end the walk with exit 0"
grep -q '^Not signed: I want to look at item 16 again$' "$OUT" || fail "a refused signature did not write its reason"
pass "a refused signature is written with its reason"

# Answers that end early stop it, and no file is written.
rm -f "$OUT"
lines 5 yes > "$S/answers-short.txt"
conv macos "$S/answers-short.txt" "$S/out-short.txt" --dry-run
[ "$CONV_STATUS" -eq 1 ] || fail "answers that end after item 5 gave exit $CONV_STATUS, not 1"
grep -q '^conventions.sh: .*ended before' "$S/out-short.txt.err" \
  || fail "answers that end early did not give a sentence that names conventions.sh and says they ended (got: $(cat "$S/out-short.txt.err"))"
[ ! -e "$OUT" ] || fail "answers that end early left a half-written file"
pass "answers that end early stop it with a sentence and no file"

# A no needs its reason: an empty line is asked again, not taken.
{
  echo no
  echo ""
  echo "the reason, after an empty line"
  lines 16 yes
  echo yes
} > "$S/answers-empty.txt"
conv macos "$S/answers-empty.txt" "$S/out-empty.txt" --dry-run
[ "$CONV_STATUS" -eq 0 ] || fail "an empty reason after a no stopped the walk (it said: $(cat "$S/out-empty.txt.err"))"
grep -q '^| 1 | .* | no | the reason, after an empty line |$' "$OUT" || fail "an empty line was taken as the reason of a no"
pass "an empty reason is asked again"

# Linux: 11 items, a note with a pipe in it on the first: the pipe must not split its cell.
{
  echo note
  echo "left | right"
  lines 11 yes
  echo yes
} > "$S/answers-linux.txt"
conv linux "$S/answers-linux.txt" "$S/out-linux.txt" --dry-run
[ "$CONV_STATUS" -eq 0 ] || fail "conventions.sh linux --dry-run did not exit 0 (it said: $(cat "$S/out-linux.txt.err"))"
[ "$(grep -c '^Item [0-9]* of 11: ' "$S/out-linux.txt")" -eq 11 ] || fail "conventions.sh linux did not print its 11 items"
[ "$(grep -c '^| [0-9]* |' "$RESULTS/conventions-linux-dry-run.md")" -eq 11 ] \
  || fail "conventions-linux-dry-run.md does not hold 11 rows"
grep -q '^| 1 | .* | yes | left \\| right |$' "$RESULTS/conventions-linux-dry-run.md" \
  || fail "a pipe in a note was not escaped in its table cell (the row: $(grep '^| 1 |' "$RESULTS/conventions-linux-dry-run.md"))"
no_calls "conventions.sh linux --dry-run"
pass "conventions.sh linux prints and records its 11 items, a pipe in a note staying in its cell"

# Not a dry run: the version and the system are asked, through the stand-ins, and the real
# file name is used.
rm -f "$S/calls"
conv macos "$S/answers-yes.txt" "$S/out-real.txt"
[ "$CONV_STATUS" -eq 0 ] || fail "conventions.sh macos with stand-in programs did not exit 0 (it said: $(cat "$S/out-real.txt.err"))"
REAL_OUT="$RESULTS/conventions-macos.md"
[ -f "$REAL_OUT" ] || fail "conventions.sh macos wrote no $REAL_OUT"
grep -q 'yaar-review 9.9.9' "$REAL_OUT" || fail "$REAL_OUT does not hold the version the app printed"
grep -q '26.0 (TESTBUILD)' "$REAL_OUT" || fail "$REAL_OUT does not hold the system's version"
pass "without --dry-run the file is conventions-macos.md with the version and the system"

# Usage errors.
CONV_STATUS=0
"$HERE/conventions.sh" > /dev/null 2> "$S/usage.err" < /dev/null || CONV_STATUS=$?
[ "$CONV_STATUS" -eq 2 ] || fail "conventions.sh with no list gave exit $CONV_STATUS, not 2"
CONV_STATUS=0
"$HERE/conventions.sh" windows > /dev/null 2> "$S/usage.err" < /dev/null || CONV_STATUS=$?
[ "$CONV_STATUS" -eq 2 ] || fail "conventions.sh windows gave exit $CONV_STATUS, not 2"
grep -q '^conventions.sh: ' "$S/usage.err" || fail "the usage error does not name conventions.sh"
pass "a missing or unknown list is exit 2"

# ---------------------------------------------------------------------------------------
# serve-release.sh
# ---------------------------------------------------------------------------------------

F="$S/release-final"
mkdir -p "$F/site/apt" "$F/release"
printf '#!/bin/bash\necho "the install script of the test"\n' > "$F/site/install.sh"
printf '<html><body>YAAR 0.1.0</body></html>\n' > "$F/site/index.html"
printf 'the public key of the test\n' > "$F/site/apt/yaar-review.asc"
: > "$F/site/.nojekyll"
printf 'the zip of the test\n' > "$F/release/yaar-review-0.1.0-macos-universal.zip"
printf 'the deb of the test\n' > "$F/release/yaar-review_0.1.0_amd64.deb"
printf 'the arm deb of the test\n' > "$F/release/yaar-review_0.1.0_arm64.deb"
printf 'the x86 archive of the test\n' > "$F/release/yaar-review-0.1.0-linux-x86_64.tar.gz"
printf 'the arm archive of the test\n' > "$F/release/yaar-review-0.1.0-linux-aarch64.tar.gz"
printf 'the disk image of the test\n' > "$F/release/yaar-review-0.1.0-macos-universal.dmg"
checksum() {
  if command -v sha256sum > /dev/null; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}
{
  for f in yaar-review-0.1.0-macos-universal.zip yaar-review_0.1.0_amd64.deb yaar-review_0.1.0_arm64.deb \
    yaar-review-0.1.0-linux-x86_64.tar.gz yaar-review-0.1.0-linux-aarch64.tar.gz yaar-review-0.1.0-macos-universal.dmg; do
    printf '%s  %s\n' "$(checksum "$F/release/$f")" "$f"
  done
} > "$F/release/SHA256SUMS"
printf 'a detached signature\n' > "$F/release/SHA256SUMS.asc"

serve_status=0
"$HERE/serve-release.sh" > /dev/null 2> "$S/serve-usage.err" < /dev/null || serve_status=$?
[ "$serve_status" -eq 2 ] || fail "serve-release.sh with no folder gave exit $serve_status, not 2"
pass "serve-release.sh with no folder is exit 2"

mkdir -p "$S/not-final/release"
refuse "a folder with no site/" "$S/serve-nosite.out" "$S/serve-nosite.err" "$S/not-final"
serve_status=$REFUSAL_STATUS
[ "$serve_status" -eq 1 ] || fail "serve-release.sh on a folder with no site/ gave exit $serve_status, not 1"
grep -q '^serve-release.sh: .*site' "$S/serve-nosite.err" || fail "the refusal of a folder with no site/ does not name serve-release.sh and site"
pass "a folder that is not laid out as release-final is refused"

cp -R "$F" "$S/release-changed"
printf 'changed after the sums were made\n' >> "$S/release-changed/release/yaar-review_0.1.0_amd64.deb"
refuse "a file that disagrees with SHA256SUMS" "$S/serve-sums.out" "$S/serve-sums.err" "$S/release-changed"
serve_status=$REFUSAL_STATUS
[ "$serve_status" -eq 1 ] || fail "serve-release.sh on a changed file gave exit $serve_status, not 1"
grep -q 'yaar-review_0.1.0_amd64.deb' "$S/serve-sums.err" || fail "the refusal of a changed file does not name the file"
pass "a file that disagrees with SHA256SUMS is refused, by name"

# A listed file that is not in the folder is refused by name, and so is a SHA256SUMS that
# lists nothing.
cp -R "$F" "$S/release-missing"
rm "$S/release-missing/release/yaar-review_0.1.0_amd64.deb"
refuse "a listed file that is missing" "$S/serve-missing.out" "$S/serve-missing.err" "$S/release-missing"
[ "$REFUSAL_STATUS" -eq 1 ] || fail "serve-release.sh on a folder missing a listed file gave exit $REFUSAL_STATUS, not 1"
grep -q 'yaar-review_0.1.0_amd64.deb, which is not in' "$S/serve-missing.err" || fail "the refusal of a missing file does not name it (it said: $(cat "$S/serve-missing.err"))"
cp -R "$F" "$S/release-empty-sums"
: > "$S/release-empty-sums/release/SHA256SUMS"
refuse "a SHA256SUMS that lists nothing" "$S/serve-empty.out" "$S/serve-empty.err" "$S/release-empty-sums"
[ "$REFUSAL_STATUS" -eq 1 ] || fail "serve-release.sh on a SHA256SUMS that lists nothing gave exit $REFUSAL_STATUS, not 1"
grep -q '^serve-release.sh: .*SHA256SUMS' "$S/serve-empty.err" || fail "the refusal of an empty SHA256SUMS does not name serve-release.sh and the file"
pass "a listed file that is missing, and a SHA256SUMS that lists nothing, are refused by name"

# A SHA256SUMS whose last line has no newline after it is read to its end: the last file
# listed, changed after the sums were made, is refused by name.
# (A seventh file, listed last, so that the six packages are all listed whatever happens to
# the last line.)
cp -R "$F" "$S/release-nonl"
last_listed=extra-notes.txt
printf 'the notes of the test\n' > "$S/release-nonl/release/$last_listed"
printf '%s  %s\n' "$(checksum "$S/release-nonl/release/$last_listed")" "$last_listed" >> "$S/release-nonl/release/SHA256SUMS"
printf '%s' "$(cat "$S/release-nonl/release/SHA256SUMS")" > "$S/release-nonl/release/SHA256SUMS"
[ "$(tail -n 1 "$S/release-nonl/release/SHA256SUMS" | awk '{ print $2 }')" = "$last_listed" ] || fail "the test could not make its SHA256SUMS end with $last_listed"
printf 'changed after the sums were made\n' >> "$S/release-nonl/release/$last_listed"
refuse "a SHA256SUMS with no final newline, its last file changed" "$S/serve-nonl.out" "$S/serve-nonl.err" "$S/release-nonl"
[ "$REFUSAL_STATUS" -eq 1 ] || fail "serve-release.sh on a changed last listed file, no newline after its line, gave exit $REFUSAL_STATUS, not 1"
grep -q "$last_listed" "$S/serve-nonl.err" || fail "the refusal of a changed last file with no final newline does not name $last_listed (it said: $(cat "$S/serve-nonl.err"))"

# What release-final holds, each missing in turn: the folder is refused, naming it.
for gone in site/index.html site/apt site/.nojekyll release/SHA256SUMS.asc; do
  cp -R "$F" "$S/release-gone"
  rm -rf "${S:?}/release-gone/$gone"
  refuse "a folder with no $gone" "$S/serve-gone.out" "$S/serve-gone.err" "$S/release-gone"
  [ "$REFUSAL_STATUS" -eq 1 ] || fail "serve-release.sh on a folder with no $gone gave exit $REFUSAL_STATUS, not 1"
  grep -q "^serve-release.sh: .*$gone" "$S/serve-gone.err" || fail "the refusal of a folder with no $gone does not name it (it said: $(cat "$S/serve-gone.err"))"
  rm -rf "$S/release-gone"
done

# Each of the six packages must be listed, each by its own name: a SHA256SUMS that lacks one
# of them, whichever it is, is refused and says which kind is missing. (The other five lines
# stay, so that a pattern that is wider than its package, one that another package satisfies,
# lets the run through and is seen.)
for gone in yaar-review_0.1.0_amd64.deb yaar-review_0.1.0_arm64.deb \
  yaar-review-0.1.0-linux-x86_64.tar.gz yaar-review-0.1.0-linux-aarch64.tar.gz \
  yaar-review-0.1.0-macos-universal.zip yaar-review-0.1.0-macos-universal.dmg; do
  cp -R "$F" "$S/release-five"
  grep -v " $gone\$" "$F/release/SHA256SUMS" > "$S/release-five/release/SHA256SUMS"
  [ "$(wc -l < "$S/release-five/release/SHA256SUMS")" -eq "$(($(wc -l < "$F/release/SHA256SUMS") - 1))" ] \
    || fail "the test could not drop the line of $gone from its SHA256SUMS"
  refuse "a SHA256SUMS that does not list $gone" "$S/serve-five.out" "$S/serve-five.err" "$S/release-five"
  [ "$REFUSAL_STATUS" -eq 1 ] || fail "serve-release.sh on a SHA256SUMS without $gone gave exit $REFUSAL_STATUS, not 1"
  grep -q '^serve-release.sh: .*six packages' "$S/serve-five.err" || fail "the refusal of a SHA256SUMS that lists five packages ($gone is missing) does not say so (it said: $(cat "$S/serve-five.err"))"
  grep -qF -- "${gone#*0.1.0}" "$S/serve-five.err" || fail "the refusal of a SHA256SUMS without $gone does not name the kind of package that is missing, ${gone#*0.1.0} (it said: $(cat "$S/serve-five.err"))"
  rm -rf "$S/release-five"
done
cp -R "$F" "$S/release-star"
sed 's/  \(yaar-review_0.1.0_amd64.deb\)$/ *\1/' "$F/release/SHA256SUMS" > "$S/release-star/release/SHA256SUMS"
refuse "a SHA256SUMS line in binary mode" "$S/serve-star.out" "$S/serve-star.err" "$S/release-star"
[ "$REFUSAL_STATUS" -eq 1 ] || fail "serve-release.sh on a binary-mode line gave exit $REFUSAL_STATUS, not 1"
grep -q "two spaces" "$S/serve-star.err" || fail "the refusal of a binary-mode line does not name the format (it said: $(cat "$S/serve-star.err"))"
pass "the last line of SHA256SUMS counts without a newline, the layout of release-final and the six packages are required, and a binary-mode line is refused with the format"

# Start it, detached, on a free port.
: > "$S/serve.out"
nohup "$HERE/serve-release.sh" "$F" --port 0 > "$S/serve.out" 2> "$S/serve.err" < /dev/null &
SERVE_PID=$!
SERVERS="$SERVERS $SERVE_PID"
have_install_line() { grep -q '^  curl ' "$S/serve.out"; }
line_or_dead() { have_install_line || ! kill -0 "$SERVE_PID" 2> /dev/null; }
wait_for line_or_dead
have_install_line || fail "serve-release.sh printed no install line in 20 s (stdout: $(cat "$S/serve.out"); stderr: $(cat "$S/serve.err"))"

PORT=$(sed -n 's/^serve-release.sh: serving on http:\/\/127\.0\.0\.1:\([0-9][0-9]*\)\/.*$/\1/p' "$S/serve.out" | head -n 1)
[ -n "$PORT" ] || fail "serve-release.sh did not say which port it serves on (stdout: $(cat "$S/serve.out"))"
BASE="http://127.0.0.1:$PORT"

grep -qxF "  curl -fsSL $BASE/install.sh | YAAR_BASE_URL=$BASE YAAR_DOWNLOAD_URL=$BASE/download bash" "$S/serve.out" \
  || fail "the install line is not 'curl -fsSL $BASE/install.sh | YAAR_BASE_URL=$BASE YAAR_DOWNLOAD_URL=$BASE/download bash' (stdout: $(cat "$S/serve.out"))"
grep -qxF "  curl -fsSL $BASE/install.sh | YAAR_BASE_URL=$BASE YAAR_DOWNLOAD_URL=$BASE/download bash -s -- --uninstall" "$S/serve.out" \
  || fail "the uninstall line is not printed with the server's address"
if grep -q 'YAAR_ALLOW_UNNOTARIZED' "$S/serve.out"; then
  fail "the install line carries the test variable YAAR_ALLOW_UNNOTARIZED, which the notarised app does not need"
fi
pass "the install and uninstall lines name the server's own address and no test variable"

grep -q "^serve-release.sh: pid $SERVE_PID " "$S/serve.out" \
  || fail "serve-release.sh did not print its own pid $SERVE_PID (stdout: $(cat "$S/serve.out"))"

fetch() {
  # fetch <path> <file>: the body of GET <path> into <file>, failing on any status but 200.
  curl -fsS --max-time 10 -o "$2" "$BASE$1"
}
fetch /install.sh "$S/got-install.sh" || fail "GET /install.sh did not answer 200"
cmp -s "$S/got-install.sh" "$F/site/install.sh" || fail "GET /install.sh is not the site's install.sh"
fetch / "$S/got-index.html" || fail "GET / did not answer 200"
cmp -s "$S/got-index.html" "$F/site/index.html" || fail "GET / is not the site's index.html"
fetch /apt/yaar-review.asc "$S/got-key.asc" || fail "GET /apt/yaar-review.asc did not answer 200"
cmp -s "$S/got-key.asc" "$F/site/apt/yaar-review.asc" || fail "GET /apt/yaar-review.asc is not the site's key"
fetch /.nojekyll "$S/got-nojekyll" || fail "GET /.nojekyll did not answer 200"
fetch /download/SHA256SUMS "$S/got-sums" || fail "GET /download/SHA256SUMS did not answer 200"
cmp -s "$S/got-sums" "$F/release/SHA256SUMS" || fail "GET /download/SHA256SUMS is not the release's"
fetch /download/yaar-review-0.1.0-macos-universal.zip "$S/got-zip" || fail "GET /download/<the .zip> did not answer 200"
cmp -s "$S/got-zip" "$F/release/yaar-review-0.1.0-macos-universal.zip" || fail "GET /download/<the .zip> is not the release's .zip"
pass "the site is served at / and the release files at /download/, byte for byte"

code=$(curl -s --max-time 10 -o /dev/null -w '%{http_code}' "$BASE/download/no-such-file")
[ "$code" = 404 ] || fail "GET of a file the release does not hold answered $code, not 404"
code=$(curl -s --max-time 10 -o /dev/null -w '%{http_code}' "$BASE/release/SHA256SUMS")
[ "$code" = 404 ] || fail "GET /release/SHA256SUMS answered $code: the release folder is served at / too, not at /download/ only"
pass "a missing file is 404, and the release is only under /download/"

listeners=$(lsof -nP -iTCP:"$PORT" -sTCP:LISTEN -Fn 2> /dev/null | sed -n 's/^n//p' | sort -u)
[ "$listeners" = "127.0.0.1:$PORT" ] || fail "the server listens on '$listeners', not on 127.0.0.1:$PORT alone"
pass "the server listens on 127.0.0.1 only"

SERVER_PID=$(sed -n 's/^serve-release.sh: server pid \([0-9][0-9]*\)$/\1/p' "$S/serve.out" | head -n 1)
[ -n "$SERVER_PID" ] || fail "serve-release.sh did not print the pid of its server (stdout: $(cat "$S/serve.out"))"
SERVERS="$SERVERS $SERVER_PID"
kill -0 "$SERVER_PID" 2> /dev/null || fail "the server pid $SERVER_PID that serve-release.sh printed is not running"

# A second server on the busy port is refused.
refuse "a busy port" "$S/serve2.out" "$S/serve2.err" "$F" --port "$PORT"
serve_status=$REFUSAL_STATUS
[ "$serve_status" -eq 1 ] || fail "a second serve-release.sh on the busy port $PORT gave exit $serve_status, not 1"
grep -q '^serve-release.sh: ' "$S/serve2.err" || fail "the refusal of a busy port does not name serve-release.sh"
kill -0 "$SERVE_PID" 2> /dev/null || fail "the second start stopped the first server"
pass "a second server on the busy port is refused and the first goes on"

# Stop it by the pid it printed.
kill "$SERVE_PID"
gone() { ! kill -0 "$SERVE_PID" 2> /dev/null; }
wait_for gone || fail "serve-release.sh (pid $SERVE_PID) was still there 20 s after SIGTERM"
server_gone() { ! kill -0 "$SERVER_PID" 2> /dev/null; }
wait_for server_gone || fail "the server (pid $SERVER_PID) was still there after serve-release.sh ended"
if curl -s --max-time 5 -o /dev/null "$BASE/install.sh"; then
  fail "the port $PORT still answers after the stop"
fi
leftover=$(find "$TMPDIR" -maxdepth 1 -name 'yaar-serve-release*' | head -n 1)
[ -z "$leftover" ] || fail "serve-release.sh left its scratch $leftover"
pass "stopped by its pid, the script and its server are gone, the port refuses, no scratch is left"

# ---------------------------------------------------------------------------------------
# The other walkers, in a dry run: their items, once each
# ---------------------------------------------------------------------------------------

walk_count() {
  # walk_count <script> <items> <dry-run file name>: all yes, then the signature.
  local script=$1 items=$2 file=$3
  { lines "$items" yes; echo yes; } > "$S/answers-count.txt"
  rm -f "$S/calls"
  run_walker "$script --dry-run" "$S/out-count.txt" "$S/out-count.txt.err" "$S/answers-count.txt" \
    env PATH="$S/bin:$PATH" YAAR_REVIEW="$S/bin/yaar-review" "$HERE/$script" --dry-run
  CONV_STATUS=$WALK_STATUS
  [ "$CONV_STATUS" -eq 0 ] || fail "$script --dry-run with $items answers did not exit 0 (it said: $(cat "$S/out-count.txt.err"))"
  [ "$(grep -c "^Item [0-9]* of $items: " "$S/out-count.txt")" -eq "$items" ] \
    || fail "$script did not print its $items items, each once"
  [ "$(( $(grep -c '^| ' "$RESULTS/$file") - 2 ))" -eq "$items" ] || fail "$RESULTS/$file does not hold $items rows"
  grep -q '^Signed: ' "$RESULTS/$file" || fail "$RESULTS/$file is not signed after all yes"
  no_calls "$script --dry-run"
}
walk_count screen-walk.sh 5 screen-walk-dry-run.md
[ -s "$RESULTS/term-06.txt" ] || fail "screen-walk.sh wrote no test file of text for TERM-06"
grep -q "$(printf '\033\\[1mbold')" "$RESULTS/term-06.txt" || fail "the TERM-06 file holds no bold text"
grep -q "$(printf '\033\\[3mitalic')" "$RESULTS/term-06.txt" || fail "the TERM-06 file holds no italic text"
grep -q "$(printf '\033\\[1;3mbold italic')" "$RESULTS/term-06.txt" || fail "the TERM-06 file holds no combined text"
grep -q "$(printf '\342\224\214')" "$RESULTS/term-06.txt" || fail "the TERM-06 file holds no box-drawing text"
walk_count real-clis.sh 15 real-clis-dry-run.md
walk_count linux-hardware.sh 8 linux-hardware-dry-run.md
pass "screen-walk.sh, real-clis.sh and linux-hardware.sh print their 5, 15 and 8 checks once each and sign"

# The graphics line of linux-hardware.sh, with a stand-in lspci of the test's own: a machine
# whose lspci lists no display controller gets a sentence in its place (the script ended
# silently with status 1 when grep found none), the first of several lines is the one taken,
# and a failing lspci ends the script with a sentence.
mkdir -p "$S/lspci-bin"
graphics_with() {
  # graphics_with <what lspci prints> <its status>: the line and the status of ck_graphics_line.
  # shellcheck disable=SC2016
  printf '#!/bin/sh\nprintf "%%s\\n" "$LSPCI_SAYS"\nexit "$LSPCI_STATUS"\n' > "$S/lspci-bin/lspci"
  chmod 755 "$S/lspci-bin/lspci"
  GRAPHICS_STATUS=0
  # shellcheck disable=SC2016
  GRAPHICS_LINE=$(env PATH="$S/lspci-bin:$PATH" LSPCI_SAYS="$1" LSPCI_STATUS="$2" bash -c '
    CK_NAME=graphics-test
    . "$1"
    set -euo pipefail
    ck_graphics_line' _ "$HERE/lib.sh" 2> "$S/graphics.err") || GRAPHICS_STATUS=$?
}
graphics_with "00:00.0 Host bridge: Some Corporation" 0
[ "$GRAPHICS_STATUS" -eq 0 ] || fail "ck_graphics_line ended with status $GRAPHICS_STATUS when lspci lists no display controller (it said: $(cat "$S/graphics.err"))"
[ "$GRAPHICS_LINE" = "lspci shows no display controller" ] || fail "ck_graphics_line gave '$GRAPHICS_LINE' when lspci lists no display controller"
graphics_with "00:00.0 Host bridge: Some Corporation
00:02.0 VGA compatible controller: Intel Corporation
01:00.0 3D controller: Another Corporation" 0
[ "$GRAPHICS_LINE" = "00:02.0 VGA compatible controller: Intel Corporation" ] || fail "ck_graphics_line gave '$GRAPHICS_LINE', not the first display line of lspci"
graphics_with "00:00.0 Host bridge: Some Corporation" 3
[ "$GRAPHICS_STATUS" -eq 1 ] || fail "ck_graphics_line ended with status $GRAPHICS_STATUS, not 1, when lspci fails"
grep -q '^graphics-test: lspci failed' "$S/graphics.err" || fail "ck_graphics_line said nothing when lspci fails (it said: $(cat "$S/graphics.err"))"
pass "the graphics line is the first display line of lspci, a sentence when there is none, and a failing lspci ends it with a sentence"

# Not a dry run, where Linux is what runs this: linux-hardware.sh on a copy of this folder
# whose memory.sh is a stand-in, with an lspci that lists no display controller, writes its
# file and says so on standard output.
if [ "$(uname -s)" = Linux ]; then
  mkdir -p "$S/lh/bin"
  cp "$HERE"/*.sh "$S/lh/"
  printf '#!/bin/sh\necho "memory stand-in: 123 MB"\n' > "$S/lh/memory.sh"
  chmod 755 "$S/lh/memory.sh"
  printf '#!/bin/sh\nprintf "00:00.0 Host bridge: Some Corporation\\n"\n' > "$S/lh/bin/lspci"
  chmod 755 "$S/lh/bin/lspci"
  { echo ""; lines 8 yes; echo yes; } > "$S/answers-lh.txt"
  rm -f "$S/calls"
  run_walker "linux-hardware.sh" "$S/out-lh.txt" "$S/out-lh.txt.err" "$S/answers-lh.txt" \
    env PATH="$S/lh/bin:$PATH" YAAR_REVIEW="$S/bin/yaar-review" "$S/lh/linux-hardware.sh"
  [ "$WALK_STATUS" -eq 0 ] || fail "linux-hardware.sh, with an lspci that lists no display controller, exited $WALK_STATUS (it said: $(cat "$S/out-lh.txt.err"))"
  grep -q 'lspci shows no display controller' "$RESULTS/linux-hardware.md" || fail "linux-hardware.md does not say that lspci shows no display controller"
  pass "linux-hardware.sh without --dry-run writes its file when lspci lists no display controller"
else
  printf '%s: not shown here: linux-hardware.sh without --dry-run runs on Linux only\n' "$CK_NAME"
fi

# ---------------------------------------------------------------------------------------
# What the measuring scripts read, on files made here: nothing is started or posted
# ---------------------------------------------------------------------------------------

BASE_NS=1000000000000
{
  i=0
  while [ "$i" -lt 30 ]; do
    echo "post $((BASE_NS + i * 200000000)) $i a"
    i=$((i + 1))
  done
  echo "done $((BASE_NS + 6000000000))"
} > "$S/keys.input"
{
  echo "start"
  i=0
  while [ "$i" -lt 29 ]; do
    t=$((BASE_NS + i * 200000000))
    # Every third key is first met by a frame 5 ms after it that is not its own.
    if [ $((i % 3)) -eq 0 ]; then echo "cap $((t + 5000000)) $((t + 9000000)) rows=0-9"; fi
    case $i in
      1) echo "cap 0 $((t + 40000000)) rows=0-9" ;;          # no display time: the time it was seen
      2) echo "cap $((t + 50000000)) $((t + 51000000)) rows=0-9" ;;
      28) echo "cap $((t + 280000000)) $((t + 281000000)) rows=0-9" ;;   # later than the next key plus 20 ms: not its own, but the last key's, 80 ms after it
      *) echo "cap $((t + 30000000)) $((t + 31000000)) rows=0-9" ;;
    esac
    i=$((i + 1))
  done
  echo "end"
} > "$S/keys.cap"
got=$("$HERE/typing.sh" --analyse "$S/keys.input" "$S/keys.cap" 2> "$S/keys.err")
# The times are 16 of 30 ms, then 40, 50 and 80: the 95th percentile is the value at rank 17
# (50), which is not the largest (80).
[ "$got" = "n=19 median=30.00 p95=50.00" ] || fail "typing.sh --analyse gave '$got', not 'n=19 median=30.00 p95=50.00' for 19 matched keys (stderr: $(cat "$S/keys.err"))"
grep -qx 'missed=1 early=10' "$S/keys.err" || fail "typing.sh --analyse did not count 1 missed and 10 early keys (stderr: $(cat "$S/keys.err"))"
tmp_clean "typing.sh --analyse"
pass "typing.sh reads key to screen times: the first changed frame after each post, early and missed keys counted, a missing display time falling back to the time seen, the 95th percentile below the largest"

# The edges, one key a second so that the next key never bounds a key: a frame 11 ms after a
# post is too soon to be the key's (early), one 13 ms after is its own; one 399 ms after is
# its own and one 401 ms after is not (missed); the last key is bounded by the 400 ms too.
# Six keys are matched, so the median is the mean of the two in the middle.
{
  echo "post $((BASE_NS)) 0 a"
  echo "post $((BASE_NS + 1000000000)) 1 a"
  echo "post $((BASE_NS + 2000000000)) 2 a"
  echo "post $((BASE_NS + 3000000000)) 3 a"
  echo "post $((BASE_NS + 4000000000)) 4 a"
  echo "post $((BASE_NS + 5000000000)) 5 a"
  echo "post $((BASE_NS + 6000000000)) 6 a"
  echo "post $((BASE_NS + 7000000000)) 7 a"
} > "$S/edges.input"
{
  echo "start"
  echo "cap $((BASE_NS + 11000000)) $((BASE_NS + 11000000)) rows=0-9"
  echo "cap $((BASE_NS + 1013000000)) $((BASE_NS + 1013000000)) rows=0-9"
  echo "cap $((BASE_NS + 2020000000)) $((BASE_NS + 2020000000)) rows=0-9"
  echo "cap $((BASE_NS + 3030000000)) $((BASE_NS + 3030000000)) rows=0-9"
  echo "cap $((BASE_NS + 4040000000)) $((BASE_NS + 4040000000)) rows=0-9"
  echo "cap $((BASE_NS + 5399000000)) $((BASE_NS + 5399000000)) rows=0-9"
  echo "cap $((BASE_NS + 6401000000)) $((BASE_NS + 6401000000)) rows=0-9"
  echo "cap $((BASE_NS + 7025000000)) $((BASE_NS + 7025000000)) rows=0-9"
  echo "end"
} > "$S/edges.cap"
got=$("$HERE/typing.sh" --analyse "$S/edges.input" "$S/edges.cap" 2> "$S/edges.err")
[ "$got" = "n=6 median=27.50 p95=399.00" ] || fail "typing.sh --analyse gave '$got', not 'n=6 median=27.50 p95=399.00' for the times 13, 20, 25, 30, 40 and 399 ms (stderr: $(cat "$S/edges.err"))"
grep -qx 'missed=1 early=1' "$S/edges.err" || fail "typing.sh --analyse did not count 1 missed (401 ms) and 1 early (11 ms) key (stderr: $(cat "$S/edges.err"))"
tmp_clean "typing.sh --analyse"
pass "typing.sh counts a frame 11 ms after a key as early and 13 ms as its own, 399 ms as its own and 401 ms as missed, and takes the mean of the two in the middle for an even count"

# The grace after the next key: a key whose first changed frame comes 10 ms after the next
# key's post is still matched (110 ms, inside the 20 ms), and the next key meets that same
# frame 10 ms after its own post (early); a first frame 25 ms after the next key's post is
# past the grace, so that key is missed, and the next key matches it (25 ms).
{
  echo "post $((BASE_NS)) 0 a"
  echo "post $((BASE_NS + 100000000)) 1 a"
  echo "post $((BASE_NS + 1000000000)) 2 a"
  echo "post $((BASE_NS + 1100000000)) 3 a"
} > "$S/grace.input"
{
  echo "start"
  echo "cap $((BASE_NS + 110000000)) $((BASE_NS + 110000000)) rows=0-9"
  echo "cap $((BASE_NS + 1125000000)) $((BASE_NS + 1125000000)) rows=0-9"
  echo "end"
} > "$S/grace.cap"
got=$("$HERE/typing.sh" --analyse "$S/grace.input" "$S/grace.cap" 2> "$S/grace.err")
[ "$got" = "n=2 median=67.50 p95=110.00" ] || fail "typing.sh --analyse gave '$got', not 'n=2 median=67.50 p95=110.00' for the times 110 and 25 ms (stderr: $(cat "$S/grace.err"))"
grep -qx 'missed=1 early=1' "$S/grace.err" || fail "typing.sh --analyse did not count 1 missed (first frame 25 ms after the next key) and 1 early key (stderr: $(cat "$S/grace.err"))"
tmp_clean "typing.sh --analyse"
pass "typing.sh gives a key the 20 ms after the next key's post (a frame 10 ms after it is the key's own, one 25 ms after is not)"

T1=2000000000000
{
  echo "sstart $BASE_NS dy=30 hz=100 n=100"
  echo "spost $((BASE_NS + 10000000)) 0"
  echo "send $((BASE_NS + 1000000000))"
  echo "sstart $T1 dy=-30 hz=100 n=50"
  echo "send $((T1 + 500000000))"
} > "$S/scroll.input"
{
  echo "start"
  k=1
  while [ "$k" -le 100 ]; do
    if [ "$k" -ne 50 ] && [ "$k" -ne 51 ]; then echo "cap $((BASE_NS + k * 10000000)) $((BASE_NS + k * 10000000 + 1000000)) rows=0-9"; fi
    k=$((k + 1))
  done
  k=1
  while [ "$k" -le 50 ]; do
    echo "cap $((T1 + k * 10000000)) $((T1 + k * 10000000 + 1000000)) rows=0-9"
    k=$((k + 1))
  done
  echo "end"
} > "$S/scroll.cap"
"$HERE/scroll.sh" --analyse "$S/scroll.input" "$S/scroll.cap" 100 > "$S/scroll.got"
grep -qx 'phase 1: frames=98 seconds=1.00 fps=98.0 long=1 long_pct=1.03' "$S/scroll.got" \
  || fail "scroll.sh --analyse did not read phase 1 as 98 frames and 1 long gap (got: $(cat "$S/scroll.got"))"
grep -qx 'phase 2: frames=50 seconds=0.50 fps=100.0 long=0 long_pct=0.00' "$S/scroll.got" \
  || fail "scroll.sh --analyse did not read phase 2 as 50 frames and no long gap (got: $(cat "$S/scroll.got"))"
grep -qx 'total: frames=148 seconds=1.50 fps=98.7 long=1 long_pct=0.68' "$S/scroll.got" \
  || fail "scroll.sh --analyse did not total 148 frames and 1 long gap (got: $(cat "$S/scroll.got"))"
tmp_clean "scroll.sh --analyse"
pass "scroll.sh reads frames a second and gaps longer than two refresh intervals, phase by phase"

# The edge of "longer than two refresh intervals" at 100 Hz (20 ms): a gap of exactly 20 ms
# is not long, one of 21 ms is, so is one of 30 ms, and one of 19 ms is not. One phase of
# five frames whose four gaps are 20, 21, 19 and 30 ms has two long gaps.
{
  echo "sstart $BASE_NS dy=30 hz=100 n=5"
  echo "send $((BASE_NS + 200000000))"
} > "$S/edge-scroll.input"
{
  echo "start"
  echo "cap $((BASE_NS + 10000000)) $((BASE_NS + 10000000)) rows=0-9"
  echo "cap $((BASE_NS + 30000000)) $((BASE_NS + 30000000)) rows=0-9"
  echo "cap $((BASE_NS + 51000000)) $((BASE_NS + 51000000)) rows=0-9"
  echo "cap $((BASE_NS + 70000000)) $((BASE_NS + 70000000)) rows=0-9"
  echo "cap $((BASE_NS + 100000000)) $((BASE_NS + 100000000)) rows=0-9"
  echo "end"
} > "$S/edge-scroll.cap"
"$HERE/scroll.sh" --analyse "$S/edge-scroll.input" "$S/edge-scroll.cap" 100 > "$S/edge-scroll.got"
grep -qx 'phase 1: frames=5 seconds=0.20 fps=25.0 long=2 long_pct=50.00' "$S/edge-scroll.got" \
  || fail "scroll.sh --analyse did not read the gaps of 20, 21, 19 and 30 ms at 100 Hz as 2 long ones: 20 ms is not longer than two intervals, 21 ms is (got: $(cat "$S/edge-scroll.got"))"
tmp_clean "scroll.sh --analyse"
pass "scroll.sh counts a gap of 21 ms as long at 100 Hz and a gap of 20 ms as not"

{
  echo "2026-11-24 some line of the app's own"
  echo "yaar-trace main epoch_ms=1000100"
  echo "yaar-trace first_frame epoch_ms=1000180"
  echo "a line that merely mentions yaar-trace first_diff epoch_ms=5"
  echo "yaar-trace first_diff epoch_ms=1000250"
  echo "yaar-trace first_diff epoch_ms=1000900"
} > "$S/marks.err"
got=$("$HERE/start-time.sh" --read "$S/marks.err" 1000000 | tail -n 1)
[ "$got" = "100 180 250" ] || fail "start-time.sh --read gave '$got', not '100 180 250' for the marks main, first_frame and the first first_diff"
grep -v 'first_diff' "$S/marks.err" > "$S/marks-nodiff.err"
got=$("$HERE/start-time.sh" --read "$S/marks-nodiff.err" 1000000 | tail -n 1)
[ "$got" = "100 180 none" ] || fail "start-time.sh --read gave '$got', not '100 180 none' with no first_diff mark"
tmp_clean "start-time.sh --read"
pass "start-time.sh reads the marks main, first_frame and first_diff, the first of each, and says none for one that is not there"

# --linux on the Mac runs in the virtual machine through its script. Here that script is a
# stand-in that answers with a status of the test's choosing, and nothing is started.
if [ "$(uname -s)" = Darwin ]; then
  cat > "$S/fakevm.sh" << 'FAKE'
#!/bin/sh
case "$1" in
  status) echo running ;;
  ssh)
    case "$2" in
      *start-time.sh*)
        echo "start-time.sh: results in /remote/results/start-time-linux-000000"
        exit "$FAKE_VM_STATUS"
        ;;
    esac
    ;;
  scp-to | scp-from) ;;
  *)
    echo "unexpected call: $*" >&2
    exit 9
    ;;
esac
FAKE
  chmod 755 "$S/fakevm.sh"
  for pair in 0:0 1:1 2:1 255:1; do
    remote=${pair%:*}
    want=${pair#*:}
    run_walker "start-time.sh --linux" "$S/vm.out" "$S/vm.err" /dev/null \
      env YAAR_VM="$S/fakevm.sh" FAKE_VM_STATUS="$remote" "$HERE/start-time.sh" --linux
    [ "$WALK_STATUS" -eq "$want" ] || fail "start-time.sh --linux, with the machine's run ending $remote, exited $WALK_STATUS, not $want (it said: $(cat "$S/vm.err"))"
    if [ "$remote" -gt 1 ]; then
      grep -q "^start-time.sh: .*status $remote" "$S/vm.err" || fail "start-time.sh --linux said nothing of the machine's status $remote (it said: $(cat "$S/vm.err"))"
    fi
  done
  tmp_clean "start-time.sh --linux"
  pass "start-time.sh --linux passes the machine's 0 and 1 on, and any other status becomes 1 with a sentence that names it"
fi

# The programs of the app are found by their full path: two copies of one program at two
# paths are two programs.
mkdir -p "$S/a" "$S/b"
cp /bin/sleep "$S/a/yaar-review"
cp /bin/sleep "$S/b/yaar-review"
if [ "$(uname -s)" = Darwin ]; then
  # A copy of a system program is killed by the kernel until it is signed again, ad hoc and
  # without a keychain, as a copy of ours.
  codesign -s - --force "$S/a/yaar-review" 2> /dev/null
  codesign -s - --force "$S/b/yaar-review" 2> /dev/null
fi
"$S/a/yaar-review" 60 < /dev/null > /dev/null 2>&1 &
PID_A=$!
"$S/b/yaar-review" 60 < /dev/null > /dev/null 2>&1 &
PID_B=$!
SERVERS="$SERVERS $PID_A $PID_B"
pids_of() {
  bash -c '. "$1"; ck_pids_of "$2"' _ "$HERE/lib.sh" "$1"
}
found_a() {
  [ "$(pids_of "$S/a/yaar-review")" = "$PID_A" ]
}
wait_for found_a || fail "a process started from $S/a/yaar-review was not found by that path alone (found: $(pids_of "$S/a/yaar-review"))"
[ "$(pids_of "$S/b/yaar-review")" = "$PID_B" ] \
  || fail "a process started from $S/b/yaar-review was not found by that path alone"
[ -z "$(pids_of "$S/c/yaar-review")" ] || fail "a path nothing runs from found a process"
pass "a program is found by its full path, not by its name"

# A copy started after the first one is the one new process of its path (what typing.sh
# ends as its reference, and nothing else).
"$S/a/yaar-review" 60 < /dev/null > /dev/null 2>&1 &
PID_A2=$!
SERVERS="$SERVERS $PID_A2"
new_pids_of() {
  bash -c '. "$1"; ck_new_pids_of "$2" "$3"' _ "$HERE/lib.sh" "$1" "$2"
}
found_new() {
  [ "$(new_pids_of "$S/a/yaar-review" "$PID_A")" = "$PID_A2" ]
}
wait_for found_new || fail "the new copy (pid $PID_A2) of $S/a/yaar-review was not the one new process of that path (found: $(new_pids_of "$S/a/yaar-review" "$PID_A"))"
[ -z "$(new_pids_of "$S/a/yaar-review" "$PID_A $PID_A2")" ] || fail "a process that was known was found as new"
[ -z "$(new_pids_of "$S/b/yaar-review" "$PID_B")" ] || fail "the program at another path was found as new"
kill "$PID_A" "$PID_A2" "$PID_B"
if wait "$PID_A" "$PID_A2" "$PID_B" 2> /dev/null; then :; fi
pass "the one new process of a program is found by its full path, whichever application is in front"

# A process of another user is not this user's: the summed memory and the signals of the
# scripts reach this user's copies alone.
if [ "$(uname -s)" = Darwin ] && [ "$(id -u)" -ne 0 ]; then
  # launchd, pid 1, is root's: the one program of another user every Mac runs.
  LAUNCHD=$(ps -o comm= -p 1)
  [ -n "$LAUNCHD" ] || fail "ps shows no program for pid 1"
  [ -z "$(pids_of "$LAUNCHD")" ] || fail "ck_pids_of found $LAUNCHD, which root runs, as a process of $(id -un)"
  pass "a program another user runs is not found (launchd, root's, on the Mac)"
elif [ "$(uname -s)" = Linux ] && [ "$(id -u)" -eq 0 ] && command -v setpriv > /dev/null; then
  chmod 755 "$S"
  mkdir -p "$S/o"
  cp /bin/sleep "$S/o/yaar-review"
  chmod 755 "$S/o" "$S/o/yaar-review"
  setpriv --reuid=65534 --regid=65534 --clear-groups "$S/o/yaar-review" 60 < /dev/null > /dev/null 2>&1 &
  PID_O=$!
  SERVERS="$SERVERS $PID_O"
  runs_as_nobody() {
    local d
    for d in /proc/[0-9]*; do
      if [ "$(tr '\0' ' ' < "$d/cmdline" 2> /dev/null)" = "$S/o/yaar-review 60 " ] && [ "$(stat -c %u "$d")" -eq 65534 ]; then
        return 0
      fi
    done
    return 1
  }
  wait_for runs_as_nobody || fail "the copy of $S/o/yaar-review did not start as the user nobody"
  # Root may read another user's /proc/<pid>/exe only with the capability to trace it (a
  # container does not give it by default: podman run --cap-add=SYS_PTRACE). Without it the
  # link cannot be read and this check would pass whatever ck_pids_of did.
  reads_exe() {
    local d
    for d in /proc/[0-9]*; do
      if [ "$(readlink "$d/exe" 2> /dev/null)" = "$S/o/yaar-review" ]; then
        return 0
      fi
    done
    return 1
  }
  if reads_exe; then
    [ -z "$(pids_of "$S/o/yaar-review")" ] || fail "ck_pids_of found a process of the user nobody as a process of root"
    pass "a program another user runs is not found (a copy run as nobody, by root)"
  else
    printf '%s: not shown here: root cannot read the program of a process of another user (needs CAP_SYS_PTRACE)\n' "$CK_NAME"
  fi
  kill "$PID_O"
  if wait "$PID_O" 2> /dev/null; then :; fi
else
  printf '%s: not shown here: a process of another user needs the Mac as a user, or Linux as root with setpriv\n' "$CK_NAME"
fi

# ---------------------------------------------------------------------------------------

real_state_after=$(real_state)
[ "$real_state_before" = "$real_state_after" ] || fail "something named yaar-checklists* in the real $REAL_HOME changed during the run"
pass "nothing named yaar-checklists* was made or changed in the real home"
echo "$CK_NAME: all checks hold"
