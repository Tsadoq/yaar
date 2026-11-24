#!/bin/sh
# Shows that publish.sh marks a release as the latest, and says to deploy the site, only when no
# published release has a greater version, and that a draft does not count. A stand-in gh lists the
# releases and records its calls; a stand-in curl answers with the file it is asked for.
# .github/scripts/publish-latest-check.sh (needs bash)
set -eu

NAME=publish-latest-check.sh
HERE=$(cd "$(dirname "$0")" && pwd)
PUBLISH=$HERE/publish.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
[ $# -eq 0 ] || { echo "usage: .github/scripts/publish-latest-check.sh" >&2; exit 2; }
[ -f "$PUBLISH" ] || fail "$PUBLISH is missing"
command -v bash > /dev/null 2>&1 || fail "bash is not installed"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/publish-latest-check.XXXXXX") || fail "no scratch folder could be made"
trap 'rm -rf "$TMP"' EXIT
mkdir "$TMP/bin"
cat > "$TMP/bin/gh" << 'GH'
#!/bin/sh
case "$1 $2" in
  "release list")
    [ -z "${GH_LIST_FAILS:-}" ] || exit 1
    cat "$GH_RELEASES"
    ;;
  *) printf '%s\n' "$*" >> "$GH_LOG" ;;
esac
GH
cat > "$TMP/bin/curl" << 'CURL'
#!/bin/sh
echo curl >> "$GH_LOG"
out=
while [ $# -gt 0 ]; do
  if [ "$1" = -o ]; then out=$2; fi
  shift
done
cp "$BODY" "$out"
printf 200
CURL
chmod +x "$TMP/bin/gh" "$TMP/bin/curl"

# release_dir <version>: the eight files of a release, empty.
release_dir() {
  rm -rf "$TMP/release"
  mkdir "$TMP/release"
  for f in "yaar-review_$1_amd64.deb" "yaar-review_$1_arm64.deb" "yaar-review-$1-linux-x86_64.tar.gz" \
    "yaar-review-$1-linux-aarch64.tar.gz" "yaar-review-$1-macos-universal.zip" "yaar-review-$1-macos-universal.dmg" \
    SHA256SUMS SHA256SUMS.asc; do
    : > "$TMP/release/$f"
  done
}
# run <action> <version> [VAR=value]...: leaves $STATUS and $SAID; $TMP/gh.log holds what gh and curl were asked.
run() {
  action=$1
  version=$2
  shift 2
  : > "$TMP/gh.log"
  : > "$TMP/output"
  release_dir "$version"
  STATUS=0
  env PATH="$TMP/bin:$PATH" GH_TOKEN=x GITHUB_REPOSITORY=example/yaar GH_RELEASES="$TMP/releases" GH_LOG="$TMP/gh.log" \
    BODY="$TMP/release/SHA256SUMS" GITHUB_OUTPUT="$TMP/output" "$@" \
    bash "$PUBLISH" "$action" "$TMP/release" "$version" > "$TMP/out" 2> "$TMP/err" || STATUS=$?
  SAID=$(cat "$TMP/out" "$TMP/err")
}

CASES=0
# latest_is <what it shows> <version> <true|false> <releases: one 'tag isDraft' per line, or empty>
latest_is() {
  CASES=$((CASES + 1))
  printf '%s' "$4" > "$TMP/releases"
  [ -z "$4" ] || printf '\n' >> "$TMP/releases"
  run latest "$2"
  [ "$STATUS" -eq 0 ] || fail "$1: publish.sh latest exited $STATUS: $SAID"
  [ "$(cat "$TMP/output")" = "latest=$3" ] || fail "$1: the output is '$(cat "$TMP/output")' and not latest=$3"
  [ ! -s "$TMP/gh.log" ] || fail "$1: gh was asked to change something: $(cat "$TMP/gh.log")"
}

latest_is "no other release" 0.3.0 true ''
latest_is "a lower release" 0.3.0 true 'v0.2.0 false'
latest_is "a greater release" 0.3.0 false 'v0.4.0 false'
case $SAID in
  *'v0.4.0'*'not as the latest'*'site is not deployed'*) ;;
  *) fail "a greater release: the sentence does not name v0.4.0, nor say the release is not the latest and the site not deployed: $SAID" ;;
esac
latest_is "a greater release that is a draft" 0.3.0 true 'v0.4.0 true'
latest_is "the same release, published already" 0.3.0 true 'v0.3.0 false'
latest_is "a lower and a greater draft" 0.3.0 true 'v0.4.0 true
v0.2.0 false'
latest_is "a lower, a draft and a greater" 0.3.0 false 'v0.2.0 false
v0.4.0 true
v0.5.0 false'
latest_is "10 is more than 9 in the minor number" 0.9.0 false 'v0.10.0 false'
latest_is "9 is less than 10 in the minor number" 0.10.0 true 'v0.9.0 false'
latest_is "a greater major number beats a lesser minor" 1.0.0 true 'v0.99.99 false'
latest_is "a greater patch number" 0.3.0 false 'v0.3.1 false'
latest_is "a tag that is not a version is no release of one" 0.3.0 true 'nightly false
v0.4.0-rc.1 false
v0.4 false'

# The listing fails: no answer is given.
printf 'v0.4.0 false\n' > "$TMP/releases"
run latest 0.3.0 GH_LIST_FAILS=1
[ "$STATUS" -ne 0 ] || fail "a release list that fails passed: $SAID"
[ ! -s "$TMP/output" ] || fail "a release list that fails still wrote an answer: $(cat "$TMP/output")"
case $SAID in *"could not list the releases"*) ;; *) fail "a release list that fails is refused without saying so: $SAID" ;; esac

# The publication follows the answer.
: > "$TMP/releases"
run publish 0.3.0 YAAR_LATEST=true
[ "$STATUS" -eq 0 ] || fail "publish with the answer true exited $STATUS: $SAID"
grep -qxF 'release edit v0.3.0 --repo example/yaar --draft=false --latest' "$TMP/gh.log" \
  || fail "publish with the answer true did not publish the release as the latest: $(cat "$TMP/gh.log")"
grep -qxF curl "$TMP/gh.log" || fail "publish with the answer true did not wait for releases/latest/download"

run publish 0.3.0 YAAR_LATEST=false
[ "$STATUS" -eq 0 ] || fail "publish with the answer false exited $STATUS: $SAID"
[ "$(cat "$TMP/gh.log")" = 'release edit v0.3.0 --repo example/yaar --draft=false --latest=false' ] \
  || fail "publish with the answer false did not publish the release as not the latest: $(cat "$TMP/gh.log")"

for answer in '' maybe TRUE; do
  run publish 0.3.0 YAAR_LATEST="$answer"
  [ "$STATUS" -ne 0 ] || fail "publish with the answer '$answer' passed"
  [ ! -s "$TMP/gh.log" ] || fail "publish with the answer '$answer' called gh: $(cat "$TMP/gh.log")"
  case $SAID in *"YAAR_LATEST must be true or false"*) ;; *) fail "publish with the answer '$answer' is refused without saying why: $SAID" ;; esac
done
run check 0.3.0 YAAR_LATEST=
[ "$STATUS" -ne 0 ] || fail "check with no answer passed"
case $SAID in *"YAAR_LATEST must be true or false"*) ;; *) fail "check with no answer is refused without saying why: $SAID" ;; esac

echo "$NAME: the $CASES cases hold: the latest release and the site go to the greatest published version only, and a draft does not count"
