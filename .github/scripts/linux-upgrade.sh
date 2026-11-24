#!/bin/bash
# Row PKG-06: a later version arrives with apt-get upgrade.
#
#   .github/scripts/linux-upgrade.sh <user> <site-later-dir> <later-version>
#
# Run after the install line installed <version> from the site served on port 8000.
#   1. Before: `dpkg-query -W -f='${Version}' yaar-review` is not <later-version>, and
#      `apt-cache madison yaar-review` lists one version.
#   2. serve-site.sh switch <site-later-dir> 8000: the same address now serves the
#      repository with both versions, signed by the same key (a new deployment of the site).
#   3. As <user>, with sudo: `apt-get update`, then `apt-get upgrade -y` (not install).
#   4. After: the installed version is <later-version>; `apt-cache madison yaar-review` lists
#      both versions from 127.0.0.1:8000 (the first was not unsigned by adding the second);
#      `apt-get update` printed no "W:" and no "E:" line; /usr/bin/yaar-review is still the
#      program of the package (`dpkg -S /usr/bin/yaar-review`).
# The server stays on <site-later-dir>: the steps after this one run on the newest site, as a
# user's system does after a release (the second run of the install line then meets the
# version it has installed as the newest, which is the state a first run leaves).
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# What the header leaves: `apt-get update` is the plain one (every source), so that a
# warning about the repository under test cannot hide behind a narrowed source list; its
# output and exit status are both read; a command that stops the script without a sentence
# of its own is named in one, and the exit code is 1.
set -euo pipefail

NAME=linux-upgrade.sh
SAID=0
fail() {
  SAID=1
  echo "$NAME: $*" >&2
  exit 1
}

if [ $# -ne 3 ]; then
  echo "usage: $NAME <user> <site-later-dir> <later-version>" >&2
  exit 2
fi
USER_NAME=$1
SITE_LATER=$2
LATER=$3
id -u "$USER_NAME" > /dev/null 2>&1 || {
  echo "$NAME: no user $USER_NAME on this system" >&2
  exit 2
}
[ -d "$SITE_LATER" ] || {
  echo "$NAME: $SITE_LATER is not a folder" >&2
  exit 2
}
HERE=$(cd "$(dirname "$0")" && pwd)

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-upgrade.XXXXXX")
# A command that set -e stopped on says so in a sentence, with the code this script's header
# names, instead of leaving the failing tool's own status.
cleanup() {
  local status=$1 cmd=$2
  if [ "$status" -ne 0 ] && [ "$SAID" -eq 0 ]; then
    echo "$NAME: '$cmd' failed with status $status" >&2
    status=1
  fi
  rm -rf "$SCRATCH"
  exit "$status"
}
trap 'cleanup "$?" "$BASH_COMMAND"' EXIT

# 1. Before.
FIRST=$(dpkg-query -W -f='${Version}' yaar-review) || fail "yaar-review is not installed: the install line has not run"
echo "$NAME: installed before: $FIRST"
[ "$FIRST" != "$LATER" ] || fail "the installed version is already $LATER, the later one: the step proves nothing"
apt-cache madison yaar-review > "$SCRATCH/madison-before"
cat "$SCRATCH/madison-before"
LINES=$(wc -l < "$SCRATCH/madison-before" | tr -d ' ')
[ "$LINES" -eq 1 ] || fail "apt-cache madison lists $LINES versions of yaar-review before the later site is served, not one"

# 2. The same address serves the later site.
"$HERE/serve-site.sh" switch "$SITE_LATER" 8000

# 3. update, then upgrade (not install), as the user with sudo.
STATUS=0
# shellcheck disable=SC2024 # the redirect is this script's own, for its scratch folder
sudo -n -u "$USER_NAME" sudo -n env DEBIAN_FRONTEND=noninteractive apt-get update > "$SCRATCH/update" 2>&1 || STATUS=$?
cat "$SCRATCH/update"
[ "$STATUS" -eq 0 ] || fail "apt-get update exited with $STATUS"
if grep -E '^(W|E):' "$SCRATCH/update" > "$SCRATCH/update-bad"; then
  fail "apt-get update printed a warning or an error, so the later site is not trusted as the first was: $(tr '\n' ' ' < "$SCRATCH/update-bad")"
fi
STATUS=0
# shellcheck disable=SC2024 # the redirect is this script's own, for its scratch folder
sudo -n -u "$USER_NAME" sudo -n env DEBIAN_FRONTEND=noninteractive apt-get upgrade -y > "$SCRATCH/upgrade" 2>&1 || STATUS=$?
cat "$SCRATCH/upgrade"
[ "$STATUS" -eq 0 ] || fail "apt-get upgrade -y exited with $STATUS"

# 4. After.
NOW=$(dpkg-query -W -f='${Version}' yaar-review) || fail "yaar-review is not installed after the upgrade"
echo "$NAME: installed after: $NOW"
[ "$NOW" = "$LATER" ] || fail "the installed version is $NOW after apt-get upgrade, not $LATER"
apt-cache madison yaar-review > "$SCRATCH/madison-after"
cat "$SCRATCH/madison-after"
for v in "$FIRST" "$LATER"; do
  line=$(awk -F'|' -v v="$v" '{ gsub(/ /, "", $2); if ($2 == v) print }' "$SCRATCH/madison-after")
  [ -n "$line" ] || fail "apt-cache madison does not list yaar-review $v after the upgrade"
  case $line in
    *127.0.0.1:8000*) ;;
    *) fail "apt-cache madison lists yaar-review $v, but not from 127.0.0.1:8000: $line" ;;
  esac
done
OWNER=$(dpkg -S /usr/bin/yaar-review) || fail "no package owns /usr/bin/yaar-review after the upgrade"
case $OWNER in
  "yaar-review: /usr/bin/yaar-review") ;;
  *) fail "/usr/bin/yaar-review is not the program of the package yaar-review: $OWNER" ;;
esac
echo "$NAME: yaar-review $FIRST became $LATER by apt-get upgrade, both versions are listed from 127.0.0.1:8000"
