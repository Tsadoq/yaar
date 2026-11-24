#!/bin/sh
# What the job release does, at the root of a checkout with its full history and the right to push to
# origin. With RELEASES=on: decides the version (next-version.sh), bumps it and commits it as
# github-actions[bot] (not when it is the version Cargo.toml has), tags it, pushes the commit and the
# tag to main in one push and starts the publishing run with gh. Otherwise it only says what it would do.
# RELEASES=on packaging/release/release.sh
set -eu

NAME=release.sh
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
cd "$ROOT"

say() {
  echo "$NAME: $*"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$*" >> "$GITHUB_STEP_SUMMARY"; fi
}
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
[ $# -eq 0 ] || { echo "usage: RELEASES=on packaging/release/release.sh" >&2; exit 2; }

NEXT=$("$HERE/next-version.sh") || exit 1

if [ "${RELEASES:-}" != on ]; then
  if [ -n "$NEXT" ]; then
    say "automatic releases are off (the repository variable RELEASES is not 'on'): the next version would be $NEXT"
  else
    say "automatic releases are off (the repository variable RELEASES is not 'on'), and there is nothing to release"
  fi
  exit 0
fi
if [ -z "$NEXT" ]; then
  say "nothing to release: no commit since the last release is a fix, a feature or a breaking change"
  exit 0
fi

TAG=v$NEXT
if git rev-parse --verify --quiet "refs/tags/$TAG" > /dev/null; then
  fail "the tag $TAG exists already, on a commit this one does not reach: nothing was released"
fi
ON_ORIGIN=$(git ls-remote --tags origin "refs/tags/$TAG") || fail "git cannot ask origin whether the tag $TAG exists"
[ -z "$ON_ORIGIN" ] || fail "the tag $TAG exists already on origin: nothing was released"

GIT_AUTHOR_NAME='github-actions[bot]'
GIT_AUTHOR_EMAIL='41898282+github-actions[bot]@users.noreply.github.com'
GIT_COMMITTER_NAME=$GIT_AUTHOR_NAME
GIT_COMMITTER_EMAIL=$GIT_AUTHOR_EMAIL
export GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL

if [ "$NEXT" != "$("$ROOT/packaging/version.sh")" ]; then
  "$HERE/bump.sh" "$NEXT" || exit 1
  git add Cargo.toml Cargo.lock
  git commit -q -m "chore(release): $TAG"
fi
git tag -a "$TAG" -m "YAAR $NEXT"

if ! git push --atomic origin HEAD:refs/heads/main "refs/tags/$TAG"; then
  fail "the push of $TAG to main was refused: main moved while this run went, or the token may not push to it. Nothing was released, and the next run on main releases"
fi
if ! gh workflow run ci.yml --ref "$TAG" -f publish=true; then
  fail "$TAG is pushed, but gh could not start its publishing run: start it with gh workflow run ci.yml --ref $TAG -f publish=true"
fi
say "released $TAG: the publishing run is started"
