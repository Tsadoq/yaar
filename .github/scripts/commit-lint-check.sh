#!/bin/sh
# Shows on scratch repositories that commit-lint.sh passes the commits it should and names
# each one it should not. .github/scripts/commit-lint-check.sh (needs git; reads nothing else)
set -eu

NAME=commit-lint-check.sh
HERE=$(cd "$(dirname "$0")" && pwd)
LINT=$HERE/commit-lint.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
[ $# -eq 0 ] || { echo "usage: .github/scripts/commit-lint-check.sh" >&2; exit 2; }
[ -x "$LINT" ] || fail "$LINT is not an executable file"
command -v git > /dev/null 2>&1 || fail "git is not installed"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/commit-lint-check.XXXXXX") || fail "no scratch folder could be made"
trap 'rm -rf "$TMP"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=check GIT_AUTHOR_EMAIL=check@example.invalid
export GIT_COMMITTER_NAME=check GIT_COMMITTER_EMAIL=check@example.invalid

REPO=$TMP/repo
git init -q "$REPO"
cd "$REPO"
git symbolic-ref HEAD refs/heads/main
say() { git commit -q --allow-empty -m "$1"; }

say 'chore: the first commit, which every range starts after'
START=$(git rev-parse HEAD)

# lint <base> <head>: runs the script, leaves its status in $STATUS and its words in $SAID.
lint() {
  STATUS=0
  "$LINT" "$1" "$2" > "$TMP/out" 2> "$TMP/err" || STATUS=$?
  SAID=$(cat "$TMP/out" "$TMP/err")
}

# Every type, with and without a scope and a '!'.
for subject in 'feat: add a thing' 'fix: mend a thing' 'perf: speed a thing' 'refactor: shape a thing' \
  'docs: say a thing' 'test: show a thing' 'build: make a thing' 'ci: run a thing' 'chore: tidy a thing' \
  'revert: take a thing back' 'feat(link): add a thing to the link' 'fix(adapter-pi): mend pi' \
  'feat(a1-b2)!: break a thing' 'refactor!: break a thing without a scope'; do
  say "$subject"
done
lint "$START" HEAD
[ "$STATUS" -eq 0 ] || fail "the 14 commits of every type were refused (status $STATUS): $SAID"
case $SAID in
  *"14 commits"*) ;;
  *) fail "the commits of every type pass, but the script does not say it looked at 14 commits: $SAID" ;;
esac

# An empty range holds nothing to refuse.
lint HEAD HEAD
[ "$STATUS" -eq 0 ] || fail "an empty range was refused (status $STATUS): $SAID"

# Merge commits pass whatever they say; the commits beneath them are held to the form.
GOOD=$(git rev-parse HEAD)
git checkout -q -b side
say 'feat: a commit of a side branch'
git checkout -q main
say 'fix: a commit of main'
git merge -q --no-ff -m 'Merge branch side into main' side
lint "$GOOD" HEAD
[ "$STATUS" -eq 0 ] || fail "a merge commit with a free subject was refused (status $STATUS): $SAID"

git checkout -q -b side2 "$GOOD"
say 'Fix the side of the thing'
git checkout -q main
git merge -q --no-ff -m 'Merge pull request #2 from someone/side2' side2
lint "$GOOD" HEAD
[ "$STATUS" -eq 1 ] || fail "a commit beneath a merge commit with a free subject passed (status $STATUS): $SAID"
case $SAID in
  *'Fix the side of the thing'*) ;;
  *) fail "the commit beneath the merge commit is refused without being named: $SAID" ;;
esac
case $SAID in
  *'Merge pull request #2'*) fail "the merge commit itself is named, and merge commits are passed: $SAID" ;;
esac

# A subject of two lines is read as git joins it: next-version.sh reads it the same way.
BASE=$(git rev-parse HEAD)
git commit -q --allow-empty -m 'feat:
x'
lint "$BASE" HEAD
[ "$STATUS" -eq 0 ] || fail "a subject cut after the colon, which git joins into 'feat: x', was refused (status $STATUS): $SAID"

# Each of these is refused and named; the good commit beside them is not.
BASE=$(git rev-parse HEAD)
for subject in 'Update the readme' 'feat add a thing' 'Feat: capital type' 'feat(Link): capital scope' \
  'feat(a_b): underscore in the scope' 'feat(): empty scope' 'feat:' 'feat:no space' 'feat:  two spaces' \
  'wip: not a type' 'feat!(link): the bang after the scope' 'fix (link): space before the scope' \
  'revert "feat: a thing"' 'feat:x' 'feat:  x' 'feat(UI): x' 'feat(a_b): x'; do
  say 'fix: this one is fine'
  say "$subject"
  lint "$BASE" HEAD
  [ "$STATUS" -eq 1 ] || fail "the subject '$subject' passed (status $STATUS): $SAID"
  case $SAID in
    *"$subject"*) ;;
    *) fail "the subject '$subject' is refused without being named: $SAID" ;;
  esac
  case $SAID in
    *'this one is fine'*) fail "the good commit beside '$subject' is named as a fault: $SAID" ;;
  esac
  BASE=$(git rev-parse HEAD)
done

# The commits are those of <base>..<head>: a base that moved on holds commits of its own, and the
# head does not answer for them.
BASE=$(git rev-parse HEAD)
git checkout -q -b pr
say 'feat: the work of the pull request'
git checkout -q main
say 'Update the readme on main'
lint main pr
[ "$STATUS" -eq 0 ] || fail "a base that moved on with a commit of its own was held against the head (status $STATUS): $SAID"
case $SAID in
  *"the 1 commits"*) ;;
  *) fail "the head holds one commit past the base, and the script does not say 1: $SAID" ;;
esac
git checkout -q pr
git merge -q --no-ff -m 'Merge branch main into pr' main
say 'fix: more work of the pull request'
lint main pr
[ "$STATUS" -eq 0 ] || fail "the commits of main that a merge brought into the head were held against it (status $STATUS): $SAID"
case $SAID in
  *"the 2 commits"*) ;;
  *) fail "the head holds two commits of its own past the base and a merge, and the script does not say 2: $SAID" ;;
esac
git checkout -q main
git merge -q --no-ff -m 'Merge pull request #3 from someone/pr' pr
git branch -q -D pr

# A criss-cross history: main merged part of the branch, and the branch merged main from before that.
# A bad commit pushed straight to main is not the pull request's: main..pr holds one commit.
CROSS=$TMP/cross
git init -q "$CROSS"
cd "$CROSS"
git symbolic-ref HEAD refs/heads/main
say 'chore: the root of the criss-cross history'
git checkout -q -b pr
say 'feat: the first part of the pull request'
git checkout -q main
say 'Update the readme straight on main'
git merge -q --no-ff -m 'Merge pull request #4 from someone/pr' pr
git checkout -q pr
git merge -q --no-ff -m 'Merge branch main into pr' main~1
say 'fix: the own work of the pull request'
[ "$(git merge-base --all main pr | wc -l | tr -d ' ')" -eq 2 ] || fail "the scratch history is not a criss-cross: main and pr have not two merge bases"
lint main pr
[ "$STATUS" -eq 0 ] || fail "in a criss-cross history, a bad commit already on main was held against the head (status $STATUS): $SAID"
case $SAID in
  *"the 1 commits"*) ;;
  *) fail "in a criss-cross history, main..pr holds one commit that is not a merge, and the script does not say 1: $SAID" ;;
esac
cd "$REPO"

# A base and a head with no history in common: a sentence, not a pass.
git checkout -q --orphan alone
say 'feat: a commit that no other reaches'
lint main alone
[ "$STATUS" -eq 1 ] || fail "a head with no common ancestor passed (status $STATUS): $SAID"
case $SAID in
  *'no common ancestor'*'fetch-depth: 0'*) ;;
  *) fail "a head with no common ancestor is refused without saying how to get the history: $SAID" ;;
esac
git checkout -q main
git branch -q -D alone

# Every fault is named, not only the first.
BASE=$(git rev-parse HEAD)
say 'first bad one'
say 'feat: good'
say 'second bad one'
lint "$BASE" HEAD
[ "$STATUS" -eq 1 ] || fail "two bad commits passed (status $STATUS): $SAID"
for subject in 'first bad one' 'second bad one'; do
  case $SAID in
    *"$subject"*) ;;
    *) fail "of two bad commits, '$subject' is not named: $SAID" ;;
  esac
done
case $SAID in
  *'<type>[(<scope>)][!]: <description>'*) ;;
  *) fail "the refusal does not say the form a subject has: $SAID" ;;
esac

# The history between the two is not there: a sentence, not a pass.
lint 0000000000000000000000000000000000000001 HEAD
[ "$STATUS" -eq 1 ] || fail "a base that is no commit of the repository passed (status $STATUS): $SAID"
case $SAID in
  *'fetch-depth: 0'*) ;;
  *) fail "a base that is no commit does not say how to get the history: $SAID" ;;
esac

STATUS=0
"$LINT" "$START" > /dev/null 2>&1 || STATUS=$?
[ "$STATUS" -eq 2 ] || fail "a call with one argument exited $STATUS and not 2"

echo "$NAME: every type passes, merge commits pass, each bad subject is named, and a missing history is refused"
