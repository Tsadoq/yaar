#!/bin/bash
# Serves a downloaded release-final folder on 127.0.0.1 as GitHub Pages and the download address
# would, so the install lines run on the publishing run's files before anything is public.
#
# packaging/checklists/serve-release.sh <folder of release-final> [--port N]
#
# Checks the layout (site/, release/) and every checksum, then prints the install and uninstall
# lines to run in another terminal (on Linux, run this there on a copy of the folder).
# Exit: never 0 by itself (130 Ctrl-C, 143 SIGTERM), 1 refused or failed, 2 usage.
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
[ -f "$SITE/index.html" ] || ck_die "$SITE/index.html is missing: $FOLDER is not laid out as release-final"
[ -d "$SITE/apt" ] || ck_die "$SITE/apt is missing: $FOLDER is not laid out as release-final"
[ -e "$SITE/.nojekyll" ] || ck_die "$SITE/.nojekyll is missing: $FOLDER is not laid out as release-final"
[ -f "$RELEASE/SHA256SUMS" ] || ck_die "$RELEASE/SHA256SUMS is missing"
[ -f "$RELEASE/SHA256SUMS.asc" ] || ck_die "$RELEASE/SHA256SUMS.asc is missing: $FOLDER is not laid out as release-final"

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
LISTED=""
while read -r sum name || [ -n "${sum:-}" ]; do
  [ -n "${name:-}" ] || continue
  case $name in
    \**) ck_die "$RELEASE/SHA256SUMS lists $name: a line is '<sha256>  <name>' with two spaces and no *" ;;
  esac
  [ -f "$RELEASE/$name" ] || ck_die "$RELEASE/SHA256SUMS lists $name, which is not in $RELEASE"
  got=$(sha256_of "$RELEASE/$name")
  [ "$got" = "$sum" ] || ck_die "$RELEASE/$name does not match its line in SHA256SUMS (listed $sum, found $got)"
  LISTED="$LISTED$name
"
  COUNT=$((COUNT + 1))
done < "$RELEASE/SHA256SUMS"
[ "$COUNT" -gt 0 ] || ck_die "$RELEASE/SHA256SUMS lists no file"

lists() {
  local pattern=$1 listed
  while IFS= read -r listed; do
    # shellcheck disable=SC2254
    case $listed in
      $pattern) return 0 ;;
    esac
  done << LIST
$LISTED
LIST
  return 1
}
for pattern in 'yaar-review_*_amd64.deb' 'yaar-review_*_arm64.deb' 'yaar-review-*-linux-x86_64.tar.gz' \
  'yaar-review-*-linux-aarch64.tar.gz' 'yaar-review-*-macos-universal.zip' 'yaar-review-*-macos-universal.dmg'; do
  lists "$pattern" || ck_die "$RELEASE/SHA256SUMS lists no $pattern: it does not list the six packages of a release"
done

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
CK_EXTRA_CLEANUP=cleanup
trap 'exit 130' INT
trap 'exit 143' TERM

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

if wait "$SERVER_PID"; then
  status=0
else
  status=$?
fi
SERVER_PID=""
ck_die "the server ended by itself (status $status): $(tail -n 3 "$LOG" | tr '\n' ' ')"
