#!/bin/bash
# The project's key of the apt repository, in the job assemble-release only, on a run that
# publishes. Reviewed by reading: no test runs it.
#
#   .github/scripts/project-key.sh import
#   .github/scripts/project-key.sh remove
#
# import, with APT_SIGNING_KEY (the secret: the armoured OpenPGP secret key the owner made
# once, ed25519, for signing, with no passphrase on this copy): refuses when it is empty;
# makes $RUNNER_TEMP/yaar-apt-key (mode 700) as GNUPGHOME and imports the key there from
# standard input (never from a command line); reads its fingerprint; refuses, changing
# nothing more, unless it equals the one line of packaging/apt/project-key.fingerprint
# that is not a comment (a key
# that is not the project's would cut every installed copy off from updates); writes
# GNUPGHOME and YAAR_APT_KEY=<fingerprint> into $GITHUB_ENV. Prints the fingerprint only.
# remove: kills the gpg agent of that folder and deletes the folder. Succeeds when there is
# nothing to remove.
#
# Exit codes: 0 done; 1 refused or failed; 2 usage error.
#
# Beyond the header's list: a fingerprint line that is not forty hexadecimal digits (either
# case) is refused; so is a variable holding anything but exactly one secret key, a
# GNUPGHOME folder left from an earlier import, and a run with RUNNER_TEMP or GITHUB_ENV
# unset. A refusal after the key was read removes the folder again: a key that is not the
# project's has no business staying on the runner, and nothing is written to GITHUB_ENV. gpg's
# own messages go to a file inside the folder and are deleted with it; a failure says its
# first line.
set -euo pipefail
umask 077

NAME=project-key.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
if [ $# -ne 1 ]; then
  echo "usage: $NAME import | remove" >&2
  exit 2
fi
ACTION=$1
case $ACTION in
  import | remove) ;;
  *)
    echo "usage: $NAME import | remove" >&2
    exit 2
    ;;
esac

[ -n "${RUNNER_TEMP:-}" ] || fail "RUNNER_TEMP is not set: the folder of the key lies under it"
KEYDIR=$RUNNER_TEMP/yaar-apt-key

if [ "$ACTION" = remove ]; then
  if [ -e "$KEYDIR" ] || [ -L "$KEYDIR" ]; then
    # The agent holds the folder's sockets and the key in memory: it goes first. A failure of
    # the kill is said, and the folder goes all the same.
    if command -v gpgconf > /dev/null 2>&1; then
      gpgconf --homedir "$KEYDIR" --kill gpg-agent > /dev/null 2>&1 \
        || echo "$NAME: warning: gpgconf could not stop the gpg agent of $KEYDIR" >&2
    else
      echo "$NAME: warning: gpgconf is not installed, so the gpg agent of $KEYDIR is not stopped" >&2
    fi
    rm -rf "$KEYDIR"
    if [ -e "$KEYDIR" ] || [ -L "$KEYDIR" ]; then fail "$KEYDIR could not be removed"; fi
    echo "$NAME: removed $KEYDIR"
  else
    echo "$NAME: nothing to remove: $KEYDIR is not there"
  fi
  exit 0
fi

# import
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
FPR_FILE=$ROOT/packaging/apt/project-key.fingerprint

case ${APT_SIGNING_KEY:-} in
  *[![:space:]]*) ;;
  *) fail "APT_SIGNING_KEY is empty: the secret of the environment release holding the project's key is not set" ;;
esac
[ -n "${GITHUB_ENV:-}" ] || fail "GITHUB_ENV is not set: this step hands the key's folder to the later steps through it"
[ -f "$FPR_FILE" ] || fail "$FPR_FILE is missing: the project's key has no fingerprint on record"
command -v gpg > /dev/null 2>&1 || fail "gpg is not installed"
# The one line of the file that is not a comment.
WANT=$(awk '/^[ \t]*(#|$)/ { next } { print; exit }' "$FPR_FILE")
[ -n "$WANT" ] || fail "$FPR_FILE holds no fingerprint (only comments): the owner writes the key's fingerprint there once, and the project's key cannot sign before"
case $WANT in
  *[!0-9A-Fa-f]*) fail "$FPR_FILE: the fingerprint line is not forty hexadecimal digits: $WANT" ;;
esac
[ "${#WANT}" -eq 40 ] || fail "$FPR_FILE: the fingerprint line is not forty hexadecimal digits: $WANT"
WANT=$(printf '%s' "$WANT" | tr 'a-f' 'A-F')
if [ -e "$KEYDIR" ] || [ -L "$KEYDIR" ]; then
  fail "$KEYDIR is there already, left from an earlier import: run $NAME remove first"
fi

mkdir -m 700 "$KEYDIR"
GNUPGHOME=$KEYDIR
export GNUPGHOME
refuse() {
  # Nothing of the key stays on the runner, and the later steps are told nothing.
  if ! gpgconf --homedir "$KEYDIR" --kill gpg-agent > /dev/null 2>&1; then
    echo "$NAME: warning: gpgconf could not stop the gpg agent of $KEYDIR" >&2
  fi
  rm -rf "$KEYDIR"
  fail "$*"
}
LOG=$KEYDIR/gpg.log
# From standard input: the secret never reaches a command line.
if ! printf '%s\n' "$APT_SIGNING_KEY" | gpg --batch --no-tty --import > "$LOG" 2>&1; then
  refuse "APT_SIGNING_KEY is not an OpenPGP key gpg can import (gpg says: $(head -n 1 "$LOG"))"
fi
LISTING=$(gpg --batch --no-tty --with-colons --list-secret-keys 2> /dev/null) || refuse "gpg cannot list the key it imported"
COUNT=$(printf '%s\n' "$LISTING" | awk -F: '$1 == "sec" { n++ } END { print n + 0 }')
[ "$COUNT" -eq 1 ] || refuse "APT_SIGNING_KEY holds $COUNT secret keys, not exactly one"
# The primary key's fingerprint is the first fpr line after the sec line; a subkey's follows
# its own ssb line.
HAVE=$(printf '%s\n' "$LISTING" | awk -F: '$1 == "sec" { s = 1; next } s && $1 == "fpr" { print $10; exit }')
[ -n "$HAVE" ] || refuse "no fingerprint could be read from the key in APT_SIGNING_KEY"
HAVE=$(printf '%s' "$HAVE" | tr 'a-f' 'A-F')
if [ "$HAVE" != "$WANT" ]; then
  refuse "the key in APT_SIGNING_KEY has the fingerprint $HAVE, not the $WANT of $FPR_FILE: a key that is not the project's would cut every installed copy off from updates"
fi
rm -f "$LOG"
case $KEYDIR in
  *[![:print:]]*) refuse "$KEYDIR holds a character GITHUB_ENV cannot carry" ;;
esac
{
  printf 'GNUPGHOME=%s\n' "$KEYDIR"
  printf 'YAAR_APT_KEY=%s\n' "$HAVE"
} >> "$GITHUB_ENV" || refuse "GITHUB_ENV, $GITHUB_ENV, cannot be written"
printf '%s\n' "$HAVE"
