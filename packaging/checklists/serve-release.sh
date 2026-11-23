#!/bin/bash
# Serves the files of a release on this machine, as GitHub Pages and the release's download
# address would, so that the install and the uninstall lines of the checklists run on the
# files the publishing run made, before anything is public.
#
#   packaging/checklists/serve-release.sh <folder of release-final> [--port N]
#
# <folder of release-final> is the artifact `release-final` of the publishing run, downloaded
# (packaging/README.md, "The day of a release"): a folder that holds `site/` (index.html,
# install.sh, apt/, .nojekyll) and `release/` (the six packages, SHA256SUMS, SHA256SUMS.asc).
#
# What it does: checks that the folder is laid out so, and that every file SHA256SUMS lists
# is there with the checksum listed (what you walk is what the run made); then serves, from
# a folder of links in its own scratch, with python3's standard library only:
#   http://127.0.0.1:<port>/            the site (install.sh, index.html, apt/)
#   http://127.0.0.1:<port>/download/   the release files, as releases/latest/download serves
#                                       them
# on 127.0.0.1 only, never on another address. --port N asks for a port; without it the
# system picks a free one (port 0). It prints the port, its own pid and the server's, the
# install line and the uninstall line, with YAAR_BASE_URL and YAAR_DOWNLOAD_URL set to this
# server and no test variable: the app of a publishing run is notarised, so the install
# checks its Gatekeeper verdict, its ticket and its team as a user's install does. A rehearsal's
# folder (ad hoc signed, its install.sh naming no team) is refused by the macOS route; the
# script says so when it sees the marker.
#
# It serves in the foreground until Ctrl-C or SIGTERM to the pid it printed. Both stop the
# server it started, by its pid (never by name), and remove its scratch folder. The install
# line is run in another terminal. On a Linux machine, run this script there, on a copy of
# the same folder.
#
# A .dmg or .zip downloaded in a browser from the run's page carries the quarantine mark; a
# file this server hands to curl carries none. The first open of the .dmg from the browser
# is item 13 of the macOS list in conventions.sh.
#
# What it needs of the owner: the folder downloaded, python3, curl, and a tool for SHA-256
# (sha256sum, or shasum on macOS). Nothing leaves the machine.
#
# Where it writes: only its scratch folder (`mktemp -d`, removed at the end); it writes no
# results file.
#
# Exit codes: 0 never by itself (it serves until stopped, and a stop by Ctrl-C is 130 and by
# SIGTERM 143); 1 refused or failed (a sentence says why); 2 usage error.
set -euo pipefail

CK_NAME=serve-release.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

FOLDER=""
PORT=0
while [ "$#" -gt 0 ]; do
  case $1 in
    -h | --help)
      ck_help "$0"
      exit 0
      ;;
    --port)
      [ "$#" -ge 2 ] || ck_usage_error "--port needs a number"
      PORT=$2
      shift 2
      case $PORT in
        '' | *[!0-9]*) ck_usage_error "--port needs a number, not '$PORT'" ;;
      esac
      ;;
    -*) ck_usage_error "unknown option '$1'" ;;
    *)
      [ -z "$FOLDER" ] || ck_usage_error "two folders named, $FOLDER and $1"
      FOLDER=$1
      shift
      ;;
  esac
done
[ -n "$FOLDER" ] || ck_usage_error "name the folder of release-final"

[ -d "$FOLDER" ] || ck_die "$FOLDER is not a folder"
FOLDER=$(cd "$FOLDER" && pwd -P)
SITE="$FOLDER/site"
RELEASE="$FOLDER/release"
[ -d "$SITE" ] || ck_die "$FOLDER holds no site/ folder: it is not laid out as release-final"
[ -d "$RELEASE" ] || ck_die "$FOLDER holds no release/ folder: it is not laid out as release-final"
[ -f "$SITE/install.sh" ] || ck_die "$SITE/install.sh is missing"
[ -f "$RELEASE/SHA256SUMS" ] || ck_die "$RELEASE/SHA256SUMS is missing"

for tool in python3 curl; do
  command -v "$tool" > /dev/null || ck_die "the tool $tool is missing, which serving and the install line need"
done

sha256_of() {
  if command -v sha256sum > /dev/null; then
    sha256sum "$1" | awk '{ print $1 }'
  elif command -v shasum > /dev/null; then
    shasum -a 256 "$1" | awk '{ print $1 }'
  else
    ck_die "neither sha256sum nor shasum is there to check $RELEASE/SHA256SUMS"
  fi
}

COUNT=0
while read -r sum name; do
  [ -n "$name" ] || continue
  [ -f "$RELEASE/$name" ] || ck_die "$RELEASE/SHA256SUMS lists $name, which is not in $RELEASE"
  got=$(sha256_of "$RELEASE/$name")
  [ "$got" = "$sum" ] || ck_die "$RELEASE/$name does not match its line in SHA256SUMS (listed $sum, found $got)"
  COUNT=$((COUNT + 1))
done < "$RELEASE/SHA256SUMS"
[ "$COUNT" -gt 0 ] || ck_die "$RELEASE/SHA256SUMS lists no file"

W=$(mktemp -d "${TMPDIR:-/tmp}/yaar-serve-release.XXXXXX") || ck_die "cannot make a scratch folder under ${TMPDIR:-/tmp}"
SERVER_PID=""

cleanup() {
  if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2> /dev/null; then
    if kill "$SERVER_PID" 2> /dev/null; then
      local deadline=$((SECONDS + 5))
      while kill -0 "$SERVER_PID" 2> /dev/null && [ "$SECONDS" -lt "$deadline" ]; do
        sleep 0.2
      done
    fi
    if kill -0 "$SERVER_PID" 2> /dev/null; then
      kill -KILL "$SERVER_PID" 2> /dev/null
    fi
  fi
  rm -rf "$W"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# The served folder: the site's entries as links at the top, the release's folder as
# `download`.
mkdir "$W/root"
for entry in "$SITE"/* "$SITE"/.[!.]*; do
  [ -e "$entry" ] || continue
  name=${entry##*/}
  [ "$name" != download ] || ck_die "$SITE holds an entry named download, which this server uses for the release files"
  ln -s "$entry" "$W/root/$name"
done
ln -s "$RELEASE" "$W/root/download"

LOG="$W/server.log"
if command -v setsid > /dev/null; then
  setsid python3 -u -m http.server "$PORT" --bind 127.0.0.1 --directory "$W/root" > "$LOG" 2>&1 < /dev/null &
else
  nohup python3 -u -m http.server "$PORT" --bind 127.0.0.1 --directory "$W/root" > "$LOG" 2>&1 < /dev/null &
fi
SERVER_PID=$!

serving_line_or_gone() {
  grep -q '^Serving HTTP on ' "$LOG" || ! kill -0 "$SERVER_PID" 2> /dev/null
}
if ! ck_wait_until 10 serving_line_or_gone; then
  ck_die "the server said nothing in 10 s: $(tail -n 3 "$LOG" | tr '\n' ' ')"
fi
if ! grep -q '^Serving HTTP on ' "$LOG"; then
  ck_die "the server did not start: $(tail -n 3 "$LOG" | tr '\n' ' ')"
fi
PORT=$(sed -n 's/^Serving HTTP on [^ ]* port \([0-9][0-9]*\) .*$/\1/p' "$LOG" | head -n 1)
[ -n "$PORT" ] || ck_die "the server's log does not say its port: $(head -n 3 "$LOG" | tr '\n' ' ')"
BASE="http://127.0.0.1:$PORT"

installs() {
  curl -fsS --max-time 5 -o /dev/null "$BASE/install.sh" 2> /dev/null
}
if ! ck_wait_until 10 installs; then
  ck_die "GET $BASE/install.sh did not answer 200 in 10 s: $(tail -n 3 "$LOG" | tr '\n' ' ')"
fi

printf '%s: serving on %s/ (the site) and %s/download/ (the release files), 127.0.0.1 only\n' "$CK_NAME" "$BASE" "$BASE"
printf '%s: pid %s (stop it with Ctrl-C, or kill %s)\n' "$CK_NAME" "$$" "$$"
printf '%s: server pid %s\n' "$CK_NAME" "$SERVER_PID"
if grep -q '@TEAM_ID@' "$SITE/install.sh"; then
  printf '%s: note: this install.sh names no team (it still holds @TEAM_ID@): the macOS route refuses it, as it refuses an app that is not notarised\n' "$CK_NAME"
fi
printf '\nInstall, in another terminal:\n'
printf '  curl -fsSL %s/install.sh | YAAR_BASE_URL=%s YAAR_DOWNLOAD_URL=%s/download bash\n' "$BASE" "$BASE" "$BASE"
printf '\nUninstall:\n'
printf '  curl -fsSL %s/install.sh | YAAR_BASE_URL=%s YAAR_DOWNLOAD_URL=%s/download bash -s -- --uninstall\n' "$BASE" "$BASE" "$BASE"
printf '\n'

# The server ends only when it is stopped: if it ends by itself, say so.
if wait "$SERVER_PID"; then
  status=0
else
  status=$?
fi
SERVER_PID=""
ck_die "the server ended by itself (status $status): $(tail -n 3 "$LOG" | tr '\n' ' ')"
