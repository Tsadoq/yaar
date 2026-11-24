#!/bin/sh
# Fails for each commit of <head> that <base> does not reach, counted from their merge base, merge
# commits apart, whose subject is not <type>[(<scope>)][!]: <description>.
# .github/scripts/commit-lint.sh <base> <head>
# The types are feat, fix, perf, refactor, docs, test, build, ci, chore and revert; next-version.sh
# reads the same form from packaging/release/commit-subject.
set -eu

NAME=commit-lint.sh
FORM='<type>[(<scope>)][!]: <description>'
HERE=$(cd "$(dirname "$0")" && pwd)
PATTERN_FILE=$HERE/../../packaging/release/commit-subject
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
if [ $# -ne 2 ]; then
  echo "usage: .github/scripts/commit-lint.sh <base> <head>" >&2
  exit 2
fi
BASE=$1
HEAD=$2
export LC_ALL=C

[ -f "$PATTERN_FILE" ] || fail "$PATTERN_FILE is missing: it holds the form of a subject"
PATTERN=$(head -n 1 "$PATTERN_FILE") || fail "$PATTERN_FILE cannot be read"
[ -n "$PATTERN" ] || fail "$PATTERN_FILE is empty"

GIT_SAID=$(git rev-parse --git-dir 2>&1) || fail "this folder is not inside a git repository that git will read: $GIT_SAID"
for rev in "$BASE" "$HEAD"; do
  git rev-parse --verify --quiet "$rev^{commit}" > /dev/null \
    || fail "$rev is not a commit of this repository: the history between the base and the head is missing (check out with fetch-depth: 0)"
done

# The merge base, so that a base that moved on, or a stale one, does not put commits of its branch
# on the head.
MERGE_BASE=$(git merge-base "$BASE" "$HEAD") \
  || fail "$BASE and $HEAD have no common ancestor in this repository: the history between them is missing (check out with fetch-depth: 0)"

# A commit with two parents or more is a merge, and it is passed.
LISTING=$(git rev-list --parents "$MERGE_BASE..$HEAD") || fail "git cannot list the commits of $MERGE_BASE..$HEAD"
PROBLEMS=
COUNT=0
while read -r sha parents; do
  [ -n "$sha" ] || continue
  case $parents in *" "*) continue ;; esac
  COUNT=$((COUNT + 1))
  subject=$(git log -1 --format=%s "$sha")
  if ! printf '%s\n' "$subject" | grep -Eq "$PATTERN"; then
    PROBLEMS="$PROBLEMS$(printf '%.7s' "$sha") \"$subject\"
"
  fi
done << LIST
$LISTING
LIST
if [ -n "$PROBLEMS" ]; then
  printf '%s' "$PROBLEMS" | while IFS= read -r problem; do
    echo "$NAME: $problem has no subject of the form $FORM" >&2
  done
  echo "$NAME: the type is one of feat, fix, perf, refactor, docs, test, build, ci, chore, revert; the scope is lower-case letters, digits and hyphens; the description follows a colon and one space" >&2
  exit 1
fi
echo "$NAME: the $COUNT commits of $BASE..$HEAD, merge commits apart, hold the form $FORM"
