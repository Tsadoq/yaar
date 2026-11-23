#!/bin/bash
# Proves the two scripts of this folder that open nothing: conventions.sh (with a file of
# answers, and with --dry-run) and serve-release.sh (on a hand-made folder laid out as the
# artifact release-final, on a free port, stopped by its pid). The other scripts open the
# app's window or post input and are not run by anything but the owner's word.
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
#                    and the app, and writes its own file; nothing is written to the real
#                    home (HOME, TMPDIR and PATH of the runs are in this script's scratch).
#   serve-release.sh the install and uninstall lines are printed with the server's own
#                    address and no test variable; the site is served at / and the release
#                    files at /download/, byte for byte; the server listens on 127.0.0.1
#                    only; a second server on the same port, a folder that is not laid out
#                    as release-final and a file that disagrees with SHA256SUMS are
#                    refused with a sentence; stopped by the pid it printed, the script and
#                    its server are gone, the port refuses, and no scratch is left.
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

real_state_before=none
if [ -e "$REAL_HOME/yaar-checklists" ]; then
  real_state_before=$(ls -ld "$REAL_HOME/yaar-checklists" | awk '{ print $6, $7, $8 }')
fi

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
  CONV_STATUS=0
  PATH="$S/bin:$PATH" YAAR_REVIEW="$S/bin/yaar-review" \
    "$HERE/conventions.sh" "$list" "$@" < "$answers" > "$out" 2> "$out.err" || CONV_STATUS=$?
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

grep -q 'Light' "$S/out-mixed.txt" && grep -q 'as the system' "$S/out-mixed.txt" \
  || fail "conventions.sh macos does not print the light and dark steps of item 11"
awk '/^Item 16 of 17/ { on = 1 } on && /ground/ { found = 1 } END { exit !found }' "$S/out-mixed.txt" \
  || fail "conventions.sh macos does not print the light theme steps of item 16"
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

# Linux: 11 items.
{
  lines 11 yes
  echo yes
} > "$S/answers-linux.txt"
conv linux "$S/answers-linux.txt" "$S/out-linux.txt" --dry-run
[ "$CONV_STATUS" -eq 0 ] || fail "conventions.sh linux --dry-run did not exit 0 (it said: $(cat "$S/out-linux.txt.err"))"
[ "$(grep -c '^Item [0-9]* of 11: ' "$S/out-linux.txt")" -eq 11 ] || fail "conventions.sh linux did not print its 11 items"
[ "$(grep -c '^| [0-9]* |' "$RESULTS/conventions-linux-dry-run.md")" -eq 11 ] \
  || fail "conventions-linux-dry-run.md does not hold 11 rows"
pass "conventions.sh linux prints and records its 11 items"

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
checksum() {
  if command -v sha256sum > /dev/null; then
    sha256sum "$1" | awk '{ print $1 }'
  else
    shasum -a 256 "$1" | awk '{ print $1 }'
  fi
}
{
  for f in yaar-review-0.1.0-macos-universal.zip yaar-review_0.1.0_amd64.deb; do
    printf '%s  %s\n' "$(checksum "$F/release/$f")" "$f"
  done
} > "$F/release/SHA256SUMS"
printf 'a detached signature\n' > "$F/release/SHA256SUMS.asc"

serve_status=0
"$HERE/serve-release.sh" > /dev/null 2> "$S/serve-usage.err" < /dev/null || serve_status=$?
[ "$serve_status" -eq 2 ] || fail "serve-release.sh with no folder gave exit $serve_status, not 2"
pass "serve-release.sh with no folder is exit 2"

mkdir -p "$S/not-final/release"
serve_status=0
"$HERE/serve-release.sh" "$S/not-final" > /dev/null 2> "$S/serve-nosite.err" < /dev/null || serve_status=$?
[ "$serve_status" -eq 1 ] || fail "serve-release.sh on a folder with no site/ gave exit $serve_status, not 1"
grep -q '^serve-release.sh: .*site' "$S/serve-nosite.err" || fail "the refusal of a folder with no site/ does not name serve-release.sh and site"
pass "a folder that is not laid out as release-final is refused"

cp -R "$F" "$S/release-changed"
printf 'changed after the sums were made\n' >> "$S/release-changed/release/yaar-review_0.1.0_amd64.deb"
serve_status=0
"$HERE/serve-release.sh" "$S/release-changed" > /dev/null 2> "$S/serve-sums.err" < /dev/null || serve_status=$?
[ "$serve_status" -eq 1 ] || fail "serve-release.sh on a changed file gave exit $serve_status, not 1"
grep -q 'yaar-review_0.1.0_amd64.deb' "$S/serve-sums.err" || fail "the refusal of a changed file does not name the file"
pass "a file that disagrees with SHA256SUMS is refused, by name"

# Start it, detached, on a free port.
: > "$S/serve.out"
nohup "$HERE/serve-release.sh" "$F" --port 0 > "$S/serve.out" 2> "$S/serve.err" < /dev/null &
SERVE_PID=$!
SERVERS="$SERVERS $SERVE_PID"
have_install_line() { grep -q '^  curl ' "$S/serve.out"; }
wait_for() {
  local deadline=$((SECONDS + 20))
  until "$@"; do
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 0.2
  done
}
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
kill -0 "$SERVER_PID" 2> /dev/null || fail "the server pid $SERVER_PID that serve-release.sh printed is not running"

# A second server on the busy port is refused.
serve_status=0
"$HERE/serve-release.sh" "$F" --port "$PORT" > "$S/serve2.out" 2> "$S/serve2.err" < /dev/null || serve_status=$?
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

real_state_after=none
if [ -e "$REAL_HOME/yaar-checklists" ]; then
  real_state_after=$(ls -ld "$REAL_HOME/yaar-checklists" | awk '{ print $6, $7, $8 }')
fi
[ "$real_state_before" = "$real_state_after" ] || fail "the real $REAL_HOME/yaar-checklists changed during the run"
pass "nothing was written to the real home"
echo "$CK_NAME: all checks hold"
