#!/bin/sh
# Shows on scratch repositories every rule of next-version.sh: the version it prints, or nothing,
# and the notes of a tag. packaging/release/next-version-check.sh (needs git and awk)
set -eu

NAME=next-version-check.sh
HERE=$(cd "$(dirname "$0")" && pwd)
NEXT=$HERE/next-version.sh
VERSION_SH=$HERE/../version.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
[ $# -eq 0 ] || { echo "usage: packaging/release/next-version-check.sh" >&2; exit 2; }
[ -x "$NEXT" ] || fail "$NEXT is not an executable file"
[ -f "$VERSION_SH" ] || fail "$VERSION_SH is missing"
command -v git > /dev/null 2>&1 || fail "git is not installed"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/next-version-check.XXXXXX") || fail "no scratch folder could be made"
trap 'rm -rf "$TMP"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME=check GIT_AUTHOR_EMAIL=check@example.invalid
export GIT_COMMITTER_NAME=check GIT_COMMITTER_EMAIL=check@example.invalid

CASES=0

# new_repo <name>: a repository on main that holds a Cargo.toml at 0.4.2 and packaging/version.sh.
new_repo() {
  R=$TMP/repos/$1
  mkdir -p "$R/packaging"
  cd "$R"
  git init -q .
  git symbolic-ref HEAD refs/heads/main
  cp "$VERSION_SH" packaging/version.sh
  printf '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.4.2"\nedition = "2024"\n\n[workspace.dependencies]\nversion = "9.9.9"\n' > Cargo.toml
  git add .
  git commit -q -m 'chore: start the scratch repository'
}
say() { git commit -q --allow-empty -m "$1"; }
sayf() { # <subject> <footer>: a commit with a body that ends in the footer
  git commit -q --allow-empty -m "$1" -m 'A paragraph of the body, which says why.' -m "$2"
}
tag_at() { # <name> <day>: an annotated tag on HEAD, dated that day of January 2030
  GIT_COMMITTER_DATE="2030-01-0${2}T12:00:00Z" git tag -a "$1" -m "$1"
}

# next_is <what it shows> <expected>: expected is empty when nothing is to be released.
next_is() {
  CASES=$((CASES + 1))
  status=0
  got=$("$NEXT" 2> "$TMP/err") || status=$?
  [ "$status" -eq 0 ] || fail "$1: next-version.sh exited $status: $(cat "$TMP/err")"
  [ ! -s "$TMP/err" ] || fail "$1: next-version.sh wrote to standard error: $(cat "$TMP/err")"
  [ "$got" = "$2" ] || fail "$1: it printed '$got' and not '$2'"
}

new_repo no-tag
say 'feat: a feature, which counts for nothing before the first release'
next_is "with no tag it prints the version of Cargo.toml as it is" 0.4.2

new_repo chores
tag_at v0.4.2 1
for subject in 'docs: say it' 'test: show it' 'ci: run it' 'build: make it' 'refactor: shape it' 'chore: tidy it' \
  'chore(deps): move it' 'Update the readme' 'feat add a thing' 'Fix: wrong case' 'wip: not a type'; do
  say "$subject"
done
next_is "only chores, docs, tests, refactors and subjects of no form since the tag release nothing" ''

new_repo at-the-tag
tag_at v0.4.2 1
next_is "nothing since the tag releases nothing" ''

for type in fix perf revert; do
  new_repo "patch-$type"
  tag_at v0.4.2 1
  say 'docs: a chore'
  say "$type: do it"
  next_is "a $type raises the patch number" 0.4.3
done

new_repo scoped
tag_at v0.4.2 1
say 'fix(adapter-pi): do it'
next_is "a fix with a scope raises the patch number" 0.4.3

new_repo feat
tag_at v0.4.2 1
say 'feat: do it'
next_is "a feat raises the minor number and sets the patch number to 0" 0.5.0

new_repo feat-then-fix
tag_at v0.4.2 1
say 'feat(link): do it'
say 'fix: mend it'
next_is "a feat and a fix, the feat first, raise the minor number" 0.5.0

new_repo fix-then-feat
tag_at v0.4.2 1
say 'fix: mend it'
say 'feat: do it'
next_is "a fix and a feat, the fix first, raise the minor number" 0.5.0

new_repo two-fixes
tag_at v0.4.7 1
say 'fix: mend it'
say 'fix: mend it again'
next_is "two fixes raise the patch number once" 0.4.8

new_repo bang-0x
tag_at v0.4.2 1
say 'docs: say it'
say 'feat(link)!: change it'
next_is "a ! after the scope raises the minor number while the major number is 0" 0.5.0

new_repo bang-fix-0x
tag_at v0.4.2 1
say 'fix!: change it'
next_is "a ! on a fix raises the minor number while the major number is 0" 0.5.0

new_repo bang-chore-0x
tag_at v0.4.2 1
say 'chore!: change it'
next_is "a ! on a chore is a breaking change too" 0.5.0

new_repo footer-space-0x
tag_at v0.4.2 1
sayf 'fix: mend it' 'BREAKING CHANGE: the old frame is gone'
next_is "a footer BREAKING CHANGE: raises the minor number while the major number is 0" 0.5.0

new_repo footer-hyphen-0x
tag_at v0.4.2 1
sayf 'fix: mend it' 'BREAKING-CHANGE: the old frame is gone'
next_is "a footer BREAKING-CHANGE: raises the minor number while the major number is 0" 0.5.0

new_repo bang-1x
tag_at v1.2.3 1
say 'feat!: change it'
next_is "a ! raises the major number once it is 1 or more" 2.0.0

new_repo bang-scope-1x
tag_at v1.2.3 1
say 'fix(link)!: change it'
next_is "a ! after a scope raises the major number once it is 1 or more" 2.0.0

new_repo footer-space-1x
tag_at v1.2.3 1
sayf 'docs: say it' 'BREAKING CHANGE: the old frame is gone'
next_is "a footer BREAKING CHANGE: raises the major number once it is 1 or more" 2.0.0

new_repo footer-hyphen-1x
tag_at v1.2.3 1
sayf 'feat: do it' 'BREAKING-CHANGE: the old frame is gone'
next_is "a footer BREAKING-CHANGE: raises the major number once it is 1 or more" 2.0.0

new_repo footer-in-prose
tag_at v1.2.3 1
git commit -q --allow-empty -m 'fix: mend it' -m 'This is not a BREAKING CHANGE: it only mends.'
next_is "the words BREAKING CHANGE: in the middle of a line are not a footer" 1.2.4

new_repo footer-with-more
tag_at v1.2.3 1
git commit -q --allow-empty -m 'fix: mend it' -m 'Why.' -m 'Refs: 12
BREAKING CHANGE: the old frame is gone
Reviewed-by: someone'
next_is "a footer between other footers is found" 2.0.0

new_repo breaking-wins
tag_at v1.2.3 1
say 'feat: do it'
say 'fix: mend it'
say 'refactor!: change it'
next_is "a breaking change beats a feat and a fix" 2.0.0

# A subject that is not of the form is no commit of a release, however near it comes.
for subject in 'feat:x' 'feat:  x' 'feat(UI): x' 'feat(a_b): x'; do
  new_repo "near-form-$(printf '%s' "$subject" | tr -c 'a-zA-Z0-9\n' -)"
  tag_at v0.4.2 1
  say "$subject"
  next_is "the subject '$subject' is not of the form, and releases nothing" ''
done

# A subject of two lines is one subject, as commit-lint.sh reads it: git joins the lines of the first paragraph.
new_repo subject-of-two-lines
tag_at v0.4.2 1
git commit -q --allow-empty -m 'feat:
x'
next_is "a subject whose first line is cut after the colon reads as git joins it, and is of the form" 0.5.0

# What a body holds is the body's, whatever the bytes. A line that begins with the record separator
# of a log is not the start of a commit.
new_repo separator-in-body
tag_at v0.4.2 1
git commit -q --allow-empty -m 'docs: say it' -m "$(printf '\036feat!: injected')"
next_is "a body line that begins with the byte 0x1E is no commit" ''

new_repo separator-in-body-with-fix
tag_at v0.4.2 1
git commit -q --allow-empty -m 'fix: mend it' -m "$(printf 'Why.\n\036feat: injected')"
next_is "a body line that begins with the byte 0x1E adds no feat to a fix" 0.4.3

new_repo merges
tag_at v0.4.2 1
git checkout -q -b side
say 'fix: a fix of the side branch'
git checkout -q main
say 'docs: a commit of main'
git merge -q --no-ff -m 'feat!: this merge commit says feat and a bang, and counts for nothing' side
next_is "a merge commit counts for nothing, its commits count once" 0.4.3

new_repo merges-of-nothing
tag_at v0.4.2 1
git checkout -q -b side
say 'docs: a commit of the side branch'
git checkout -q main
say 'test: a commit of main'
git merge -q --no-ff -m 'feat: this merge commit counts for nothing' side
next_is "a merge of commits that release nothing releases nothing" ''

new_repo release-commit
tag_at v0.4.2 1
sayf 'chore(release): v0.5.0' 'BREAKING CHANGE: not counted'
next_is "the commit chore(release): is never counted, whatever its body says" ''

new_repo release-commit-bang
tag_at v0.4.2 1
say 'chore(release)!: v0.5.0'
say 'fix: mend it'
next_is "the commit chore(release)!: is not counted either" 0.4.3

new_repo two-tags
say 'chore: a commit'
tag_at v1.10.0 1
say 'chore: a commit'
tag_at v1.9.0 2
say 'fix: mend it'
next_is "of two tags the newer by version counts, not the newer by date, and 10 is more than 9" 1.10.1

new_repo two-tags-lightweight
say 'chore: a commit'
git tag v0.9.9
say 'fix: this fix lies before the newer tag by version, and counts for nothing'
git tag v0.10.0
say 'chore: a commit'
next_is "lightweight tags are ordered by version too" ''

new_repo not-versions
say 'fix: a fix, which no tag holds'
for t in v1.2 v1.2.3.4 v1.2.3-rc1 latest vfoo v01.2.3 v1.02.3 1.2.3 v1.2.x v; do git tag "$t"; done
say 'chore: a commit'
next_is "tags that are not versions are ignored: with no other, the version of Cargo.toml stands" 0.4.2

new_repo not-versions-beside
tag_at v0.4.2 1
say 'fix: a fix'
git tag v9.9
git tag v10.0.0-rc.1
say 'chore: a commit'
next_is "tags that are not versions are ignored beside a version" 0.4.3

new_repo tag-elsewhere
tag_at v0.4.2 1
git checkout -q -b side
say 'feat: a feature of a branch that is not merged'
tag_at v7.0.0 2
git checkout -q main
say 'fix: mend it'
next_is "a tag that HEAD does not reach is ignored" 0.4.3

new_repo tag-on-head
say 'fix: a fix before the tag'
tag_at v0.4.2 1
next_is "a fix before the newest tag is not counted again" ''

# The notes of a tag.
notes_are() { # <what it shows> <tag> <expected file>
  CASES=$((CASES + 1))
  status=0
  "$NEXT" --notes "$2" > "$TMP/notes" 2> "$TMP/err" || status=$?
  [ "$status" -eq 0 ] || fail "$1: next-version.sh --notes $2 exited $status: $(cat "$TMP/err")"
  [ ! -s "$TMP/err" ] || fail "$1: next-version.sh --notes $2 wrote to standard error: $(cat "$TMP/err")"
  if ! cmp -s "$TMP/notes" "$3"; then
    echo "$NAME: $1: the notes of $2 are not what they should be. Expected:" >&2
    sed 's/^/  | /' "$3" >&2
    echo "$NAME: got:" >&2
    sed 's/^/  | /' "$TMP/notes" >&2
    exit 1
  fi
}

new_repo notes
say 'feat: a feature before the first tag'
say 'fix(link): a fix before the first tag'
say 'docs: a page before the first tag'
tag_at v0.1.0 1
say 'feat(link): add the first thing'
say 'docs: say it'
say 'fix: mend the second thing'
say 'perf(store): speed up the third thing'
say 'feat: add the fourth thing'
say 'chore: tidy'
say 'refactor: shape the fifth thing'
say 'revert: take the sixth thing back'
say 'fix(adapter-pi): mend the seventh thing'
say 'Update the readme'
say 'revert(link): take the tenth thing back'
say 'perf: speed up the eighth thing'
say 'chore(release): v0.2.0'
say 'feat(frame)!: drop the old frame'
sayf 'fix: mend the ninth thing' 'BREAKING CHANGE: the ninth thing works otherwise'
say 'docs!: rewrite the pages'
tag_at v0.2.0 2
say 'fix: a fix of a later release'
tag_at v0.2.1 3
cat > "$TMP/want" << 'WANT'
## Breaking changes

- **frame**: drop the old frame
- mend the ninth thing
- rewrite the pages

## New

- **link**: add the first thing
- add the fourth thing

## Fixed

- mend the second thing
- **adapter-pi**: mend the seventh thing

## Faster

- **store**: speed up the third thing
- speed up the eighth thing

## Reverted

- take the sixth thing back
- **link**: take the tenth thing back
WANT
notes_are "the notes of a tag hold each heading, the scope in bold, the commits since the tag before, and nothing else" v0.2.0 "$TMP/want"

cat > "$TMP/want" << 'WANT'
## Fixed

- a fix of a later release
WANT
notes_are "the notes of the newest tag hold what lies between it and the tag before" v0.2.1 "$TMP/want"

cat > "$TMP/want" << 'WANT'
## New

- add the first thing
- add the second thing
WANT
new_repo notes-one
say 'feat: add the first thing'
say 'feat: add the second thing'
tag_at v0.1.0 1
say 'feat: not in these notes'
notes_are "a heading with no entry is left out, and a commit after the tag is not in its notes" v0.1.0 "$TMP/want"

: > "$TMP/want"
new_repo notes-none
say 'docs: say it'
say 'chore: tidy'
say 'Update the readme'
tag_at v0.1.0 1
notes_are "a tag whose commits hold nothing to list has empty notes" v0.1.0 "$TMP/want"

cat > "$TMP/want" << 'WANT'
## New

- add it

## Faster

- speed it up
WANT
new_repo notes-first
say 'chore: tidy'
say 'feat: add it'
git checkout -q -b side
say 'perf: speed it up'
git checkout -q main
say 'docs: say it'
git merge -q --no-ff -m 'Merge branch side' side
say 'chore(release): v0.1.0'
tag_at v0.1.0 1
notes_are "the notes of the first tag hold everything before it, the merged branch too, and not the merge commit" v0.1.0 "$TMP/want"

cat > "$TMP/want" << 'WANT'
## Fixed

- mend it
WANT
new_repo notes-order
say 'fix: mend the old thing'
tag_at v1.10.0 1
say 'fix: mend it'
tag_at v1.9.0 2
git tag v1.10.1
notes_are "the tag before is the greatest version it reaches, not the latest one made" v1.10.1 "$TMP/want"

cat > "$TMP/want" << 'WANT'
## Reverted

- take it back
- **link**: take the other back
WANT
new_repo notes-reverts-alone
say 'chore: tidy'
say 'revert: take it back'
say 'revert(link): take the other back'
tag_at v0.1.0 1
notes_are "a release of reverts alone has them listed under Reverted" v0.1.0 "$TMP/want"

cat > "$TMP/want" << 'WANT'
## Fixed

- mend the tab
WANT
new_repo notes-tab
git commit -q --allow-empty -m "$(printf 'fix: mend\tthe tab')"
tag_at v0.1.0 1
notes_are "a tab in a subject is one space in the notes" v0.1.0 "$TMP/want"

# Refusals.
refused() { # <what it shows> <words in the sentence> <argument>...
  CASES=$((CASES + 1))
  what=$1
  words=$2
  shift 2
  status=0
  "$NEXT" "$@" > "$TMP/out" 2> "$TMP/err" || status=$?
  [ "$status" -ne 0 ] || fail "$what: next-version.sh $* passed"
  [ ! -s "$TMP/out" ] || fail "$what: next-version.sh $* printed on standard output: $(cat "$TMP/out")"
  case $(cat "$TMP/err") in
    *"$words"*) ;;
    *) fail "$what: the sentence does not say '$words': $(cat "$TMP/err")" ;;
  esac
}
refused "a tag that does not exist is refused" "no tag v8.8.8" --notes v8.8.8
git tag v1.2
refused "a tag that is not a version is refused" "not a tag of the form v<x.y.z>" --notes v1.2
refused "notes need a tag" "usage" --notes
refused "an unknown argument is refused" "usage" --oops
new_repo outside-cargo
rm Cargo.toml
git add -A
git commit -q -m 'chore: take Cargo.toml away'
refused "with no tag and no version in Cargo.toml it says so" "Cargo.toml" 
cd "$TMP"
mkdir not-a-repository
cd not-a-repository
refused "outside a repository it says so" "not inside a git repository"

echo "$NAME: the $CASES cases hold: the version to release, nothing to release, and the notes of a tag"
