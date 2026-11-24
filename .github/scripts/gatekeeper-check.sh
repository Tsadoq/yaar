#!/bin/bash
# The step "Gatekeeper accepts the notarized disk image and the app" of release-sign-macos
# (row PKG-04, which runs at a release): what a user's Mac will say of the release, checked on
# the files that will be published.
#
#   .github/scripts/gatekeeper-check.sh <dir> <version>
#
# <dir> holds yaar-review-<version>-macos-universal.zip and .dmg, signed with the Developer ID
# and notarised. Unpacks the .zip with `ditto -x -k` into a temporary folder and mounts the
# .dmg read-only (`hdiutil attach -nobrowse -readonly -mountpoint <temp>`, detached at the end
# whatever happens). Fails with a sentence when packaging/macos/team-id holds no line that is
# not a comment, and when any of these does not hold, checked in this order:
#   - `codesign --verify --deep --strict` passes on both apps, and with the requirement
#     `-R='anchor apple generic and certificate leaf[subject.OU] = "<team id>"'`, the team id
#     being the line of packaging/macos/team-id;
#   - `spctl --assess --type execute -vv` on both apps says "accepted" and
#     "source=Notarized Developer ID";
#   - `spctl --assess --type open --context context:primary-signature -vv` on the .dmg says
#     "accepted" and "source=Notarized Developer ID";
#   - `xcrun stapler validate` passes on the app of the .zip, on the app of the .dmg and on
#     the .dmg;
#   - the apps' CFBundleShortVersionString is <version>.
# Then the install line against these files, as a user's Mac runs it, without
# YAAR_ALLOW_UNNOTARIZED: in a temporary folder <srv> it writes SHA256SUMS with the one line
# of the .zip ("<sha256>  <name>", from `shasum -a 256`) beside a copy of the .zip, and
# install.sh: packaging/install.sh with @BASE_URL@ replaced by http://127.0.0.1:<port> and
# @TEAM_ID@ by the line of packaging/macos/team-id (as packaging/release/assemble.sh does);
# serves <srv> with `python3 -m http.server <port> --bind 127.0.0.1 --directory <srv>`,
# detached (standard input from /dev/null, output to a log in <srv>), on a free port (bind
# 127.0.0.1:0 in python3 to find one); waits up to 10 s for GET /install.sh; runs `curl -fsSL
# http://127.0.0.1:<port>/install.sh | bash` with YAAR_BASE_URL and
# YAAR_DOWNLOAD_URL=http://127.0.0.1:<port>, YAAR_INSTALL_DIR a temporary folder, and asserts
# exit 0 and that folder's YAAR.app; removes the folder; stops the server by its pid.
# This script is P3's, built before serve-site.sh exists, and needs nothing of it.
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
set -euo pipefail

SELF=$(basename "$0")
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

usage() { echo "usage: $SELF <dir> <version>" >&2; exit 2; }
[ $# -eq 2 ] || usage
DIR=$1; VER=$2

fail() { echo "$SELF: $*" >&2; exit 1; }
# A command that fails with no sentence of its own still ends the run with one.
trap 'fail "line $LINENO: $BASH_COMMAND failed"' ERR

# The team: the first line of the file that is neither blank nor a comment.
TEAMFILE="$REPO/packaging/macos/team-id"
[ -f "$TEAMFILE" ] || fail "$TEAMFILE is missing"
TEAM=$(sed -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$TEAMFILE" | sed -n '1p')
[ -n "$TEAM" ] || fail "packaging/macos/team-id holds no line that is not a comment: the owner writes the Team ID there once, and without it nobody can say whose signature this is"
TEAM_RE='^[A-Z0-9]{10}$'
[[ $TEAM =~ $TEAM_RE ]] || fail "the line of packaging/macos/team-id, \"$TEAM\", is not ten capital letters and digits"

[ -d "$DIR" ] || fail "$DIR is not a folder"
ZIP="$DIR/yaar-review-$VER-macos-universal.zip"
DMG="$DIR/yaar-review-$VER-macos-universal.dmg"
[ -f "$ZIP" ] || fail "$ZIP is missing"
[ -f "$DMG" ] || fail "$DMG is missing"

WORK=$(mktemp -d)
WORK=$(cd "$WORK" && pwd -P)
MNT=""
SERVER_PID=""
cleanup() {
  if [ -n "$SERVER_PID" ]; then
    kill "$SERVER_PID" > /dev/null 2>&1 || echo "$SELF: the server $SERVER_PID had stopped before this" >&2
    # Reaped here so that the shell does not report it later; a server told to stop ends on a signal.
    wait "$SERVER_PID" > /dev/null 2>&1 || true
  fi
  if [ -n "$MNT" ]; then
    # A volume can be busy for a moment after the last read: ask again until it lets go.
    local deadline=$((SECONDS + 15)) last forced
    until last=$(hdiutil detach -quiet "$MNT" 2>&1); do
      if [ "$SECONDS" -ge "$deadline" ]; then
        forced=$(hdiutil detach -quiet -force "$MNT" 2>&1) ||
          echo "$SELF: could not detach $MNT: hdiutil last said \"$last\", and when forced \"$forced\"" >&2
        break
      fi
      sleep 0.2
    done
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir "$WORK/zip" "$WORK/mnt"
ditto -x -k "$ZIP" "$WORK/zip" || fail "$(basename "$ZIP") cannot be unpacked"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$WORK/mnt" "$DMG" > /dev/null ||
  fail "$(basename "$DMG") cannot be mounted"
MNT="$WORK/mnt"
APPS=("$WORK/zip/YAAR.app" "$WORK/mnt/YAAR.app")
NAMES=("the .zip" "the .dmg")
[ -d "${APPS[0]}" ] || fail "$(basename "$ZIP") holds no YAAR.app"
[ -d "${APPS[1]}" ] || fail "$(basename "$DMG") holds no YAAR.app"

# 1. The signature, and whose it is.
REQ="anchor apple generic and certificate leaf[subject.OU] = \"$TEAM\""
for i in 0 1; do
  app=${APPS[$i]}
  if ! out=$(codesign --verify --deep --strict "$app" 2>&1); then
    fail "the app in ${NAMES[$i]} does not pass codesign --verify --deep --strict: $out"
  fi
  if ! out=$(codesign --verify --deep --strict "-R=$REQ" "$app" 2>&1); then
    fail "the app in ${NAMES[$i]} is not signed by the team $TEAM (requirement: $REQ): $out"
  fi
done

# 2. What Gatekeeper says: accepted, and on the strength of Apple's notarisation.
accepted() { # <what> <spctl output>
  case $2 in *accepted*) ;; *) fail "Gatekeeper does not accept $1: $2" ;; esac
  case $2 in *"source=Notarized Developer ID"*) ;; *) fail "Gatekeeper accepts $1 but not as a notarised Developer ID app: $2" ;; esac
}
for i in 0 1; do
  out=$(spctl --assess --type execute -vv "${APPS[$i]}" 2>&1) || fail "Gatekeeper rejects the app in ${NAMES[$i]}: $out"
  accepted "the app in ${NAMES[$i]}" "$out"
done
out=$(spctl --assess --type open --context context:primary-signature -vv "$DMG" 2>&1) ||
  fail "Gatekeeper rejects $(basename "$DMG"): $out"
accepted "$(basename "$DMG")" "$out"

# 3. The tickets.
for i in 0 1; do
  out=$(xcrun stapler validate "${APPS[$i]}" 2>&1) || fail "the app in ${NAMES[$i]} has no valid ticket stapled: $out"
done
out=$(xcrun stapler validate "$DMG" 2>&1) || fail "$(basename "$DMG") has no valid ticket stapled: $out"

# 4. The version.
for i in 0 1; do
  got=$(plutil -extract CFBundleShortVersionString raw -o - "${APPS[$i]}/Contents/Info.plist" 2> /dev/null) || got="(not set)"
  [ "$got" = "$VER" ] || fail "the app in ${NAMES[$i]} is version \"$got\", not \"$VER\""
done

# 5. The install line against these files, as a user's Mac runs it.
command -v python3 > /dev/null || fail "python3 is not here: the server of this check needs it"
SRV="$WORK/srv"; mkdir "$SRV"
cp "$ZIP" "$SRV/" || fail "$ZIP could not be copied into the folder this check serves"
(cd "$SRV" && shasum -a 256 "$(basename "$ZIP")" > SHA256SUMS) || fail "shasum could not make the SHA256SUMS of $(basename "$ZIP")"
PORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])') ||
  fail "python3 could not find a free port on 127.0.0.1"
URL="http://127.0.0.1:$PORT"
sed -e "s|@BASE_URL@|$URL|g" -e "s|@TEAM_ID@|$TEAM|g" "$REPO/packaging/install.sh" > "$SRV/install.sh" ||
  fail "install.sh could not be made from $REPO/packaging/install.sh"

# Detached from the step: its own session where setsid exists, else nohup.
if command -v setsid > /dev/null; then
  setsid python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$SRV" < /dev/null > "$SRV/server.log" 2>&1 &
else
  nohup python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$SRV" < /dev/null > "$SRV/server.log" 2>&1 &
fi
SERVER_PID=$!

deadline=$((SECONDS + 10))
until seen=$(curl -fsS -o /dev/null "$URL/install.sh" 2>&1); do
  if [ "$SECONDS" -ge "$deadline" ]; then
    fail "the server of this check did not answer at $URL/install.sh within 10 s; the last answer: $seen; its log: $(cat "$SRV/server.log")"
  fi
  sleep 0.2
done

INSTALL="$WORK/applications"; mkdir "$INSTALL"
if ! curl -fsSL "$URL/install.sh" |
  env -u YAAR_ALLOW_UNNOTARIZED YAAR_BASE_URL="$URL" YAAR_DOWNLOAD_URL="$URL" YAAR_INSTALL_DIR="$INSTALL" bash > "$WORK/install.log" 2>&1; then
  cat "$WORK/install.log" >&2
  fail "the install line failed on these files"
fi
cat "$WORK/install.log"
[ -d "$INSTALL/YAAR.app" ] || fail "the install line ended well but left no YAAR.app in $INSTALL"

echo "$SELF: Gatekeeper, the team, the tickets and the install line accept the .zip and the .dmg of $VER"
