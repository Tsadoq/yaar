#!/bin/sh
# Shows that publish.sh draft puts the changes of the tag, then the text of notes.md.in, into the notes
# of the release, and that a tag with no change to list gives the text alone. A stand-in gh records
# the notes it is given. .github/scripts/publish-notes-check.sh (needs bash and git)
set -eu

NAME=publish-notes-check.sh
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
[ $# -eq 0 ] || { echo "usage: .github/scripts/publish-notes-check.sh" >&2; exit 2; }
for tool in bash git; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done

TMP=$(mktemp -d "${TMPDIR:-/tmp}/publish-notes-check.XXXXXX") || fail "no scratch folder could be made"
trap 'rm -rf "$TMP"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=check GIT_AUTHOR_EMAIL=check@example.invalid
export GIT_COMMITTER_NAME=check GIT_COMMITTER_EMAIL=check@example.invalid

# A repository that holds the scripts and the two texts publish.sh reads, with the tags to release.
R=$TMP/repo
mkdir -p "$R/.github/scripts" "$R/packaging/release" "$R/packaging/site" "$TMP/bin"
cp "$HERE/publish.sh" "$R/.github/scripts/publish.sh"
cp "$ROOT/packaging/release/next-version.sh" "$ROOT/packaging/release/commit-subject" "$ROOT/packaging/release/notes.md.in" "$R/packaging/release/"
cp "$ROOT/packaging/site/base-url" "$R/packaging/site/base-url"
cp "$ROOT/packaging/version.sh" "$R/packaging/version.sh"
cp "$ROOT/LICENSE" "$R/LICENSE"
cd "$R"
git init -q .
git symbolic-ref HEAD refs/heads/main
git add .
git commit -q -m 'chore: start'
git commit -q --allow-empty -m 'feat(link): add the first thing'
git tag v0.1.0
git commit -q --allow-empty -m 'fix: mend the second thing'
git commit -q --allow-empty -m 'docs: say it'
git tag v0.1.1
git commit -q --allow-empty -m 'chore: tidy'
git tag v0.1.2

cat > "$TMP/bin/gh" << 'GH'
#!/bin/sh
# release list answers nothing; release create keeps the notes file it is given.
case $1 in
  release)
    case $2 in
      create)
        while [ $# -gt 0 ]; do
          if [ "$1" = --notes-file ]; then cp "$2" "$GH_NOTES"; fi
          shift
        done
        ;;
    esac
    ;;
esac
GH
chmod +x "$TMP/bin/gh"

# draft <version>: leaves the notes in $TMP/notes.
draft() {
  rm -rf "$TMP/release" "$TMP/notes"
  mkdir "$TMP/release"
  for f in "yaar-review_$1_amd64.deb" "yaar-review_$1_arm64.deb" "yaar-review-$1-linux-x86_64.tar.gz" \
    "yaar-review-$1-linux-aarch64.tar.gz" "yaar-review-$1-macos-universal.zip" "yaar-review-$1-macos-universal.dmg" \
    SHA256SUMS SHA256SUMS.asc; do
    : > "$TMP/release/$f"
  done
  STATUS=0
  env PATH="$TMP/bin:$PATH" GH_TOKEN=x GITHUB_REPOSITORY=example/yaar GH_NOTES="$TMP/notes" \
    bash .github/scripts/publish.sh draft "$TMP/release" "$1" > "$TMP/out" 2> "$TMP/err" || STATUS=$?
  [ "$STATUS" -eq 0 ] || fail "publish.sh draft for $1 exited $STATUS: $(cat "$TMP/err")"
  [ -f "$TMP/notes" ] || fail "publish.sh draft for $1 gave gh no notes file"
}
TEXT=$TMP/text
sed -e 's|@VERSION@|0.1.1|g' -e "s|@BASE_URL@|$(head -n 1 packaging/site/base-url)|g" packaging/release/notes.md.in > "$TEXT"

draft 0.1.0
{
  printf '## New\n\n- **link**: add the first thing\n\n'
  sed -e 's|@VERSION@|0.1.0|g' -e "s|@BASE_URL@|$(head -n 1 packaging/site/base-url)|g" packaging/release/notes.md.in
} > "$TMP/want"
cmp -s "$TMP/notes" "$TMP/want" || fail "the notes of the first release are not its changes, a blank line and the text of notes.md.in: $(head -n 6 "$TMP/notes" | tr '\n' '|')"

draft 0.1.1
{
  printf '## Fixed\n\n- mend the second thing\n\n'
  cat "$TEXT"
} > "$TMP/want"
cmp -s "$TMP/notes" "$TMP/want" || fail "the notes of a later release are not its changes, a blank line and the text of notes.md.in: $(head -n 6 "$TMP/notes" | tr '\n' '|')"

draft 0.1.2
sed -e 's|@VERSION@|0.1.2|g' -e "s|@BASE_URL@|$(head -n 1 packaging/site/base-url)|g" packaging/release/notes.md.in > "$TMP/want"
cmp -s "$TMP/notes" "$TMP/want" || fail "the notes of a release with nothing to list are not the text of notes.md.in alone: $(head -n 3 "$TMP/notes" | tr '\n' '|')"

# A commit subject that looks like a placeholder is no fault of the text.
git commit -q --allow-empty -m 'fix: keep @NAME@ out of the way'
git tag v0.1.3
draft 0.1.3
grep -qF 'keep @NAME@ out of the way' "$TMP/notes" || fail "a subject with @NAME@ in it did not reach the notes"

echo "$NAME: the notes of a release are the changes of its tag, then the text of notes.md.in, and the text alone when there is no change"
