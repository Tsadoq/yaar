#!/bin/bash
# The keychain of the job release-sign-macos: made from the repository's secrets, used to
# sign, removed at the end. Runs only in that job, on a run that publishes; no test runs it
# and no agent runs it on a Mac of the owner's: it is reviewed by reading.
#
#   .github/scripts/macos-keychain.sh import
#   .github/scripts/macos-keychain.sh remove
#
# import, with these variables (the secrets of the environment `release`):
#   MACOS_CERTIFICATE_P12        the Developer ID Application certificate with its private
#                                key, and Apple's Developer ID Certification Authority (G2)
#                                certificate beside it, exported together as a .p12 file,
#                                in base64
#   MACOS_CERTIFICATE_PASSWORD   the password the .p12 was exported with
#   APPLE_NOTARY_KEY_P8          the text of the App Store Connect API key file
#                                AuthKey_<key id>.p8
#   APPLE_NOTARY_KEY_ID          that key's id
#   APPLE_NOTARY_ISSUER_ID       the issuer id of the team's keys (a UUID)
#   1. Refuses, naming the variable and never its value, when any is empty; refuses when
#      packaging/macos/team-id holds no line that is not a comment.
#   2. Makes a random password (`openssl rand -hex 24`), never printed; creates
#      $RUNNER_TEMP/yaar-signing.keychain-db with it; `security set-keychain-settings -lut
#      21600`; unlocks it; decodes the .p12 to $RUNNER_TEMP/yaar-signing/cert.p12 (folder of
#      mode 700); `security import ... -k <keychain> -P "$MACOS_CERTIFICATE_PASSWORD" -T
#      /usr/bin/codesign -f pkcs12`; `security set-key-partition-list -S apple-tool:,apple:
#      -s -k <password> <keychain>`; puts the keychain first in the user's search list,
#      keeping the others (`security list-keychains -d user -s <keychain> <the old ones>`,
#      the old list kept in $RUNNER_TEMP/yaar-signing/search-list); deletes cert.p12.
#   3. Finds the one valid identity "Developer ID Application: <name> (<team id>)" in that
#      keychain (`security find-identity -v -p codesigning <keychain>`). Refuses when there
#      is none, with the sentence: "no valid Developer ID Application identity in the .p12:
#      export the certificate together with its key and Apple's Developer ID Certification
#      Authority (G2) certificate" (without the intermediate the identity is not valid on a
#      Mac that has not got it); refuses when there are several; refuses when <team id> is
#      not the line of packaging/macos/team-id. Writes its SHA-1 hash as SIGN_IDENTITY and
#      the keychain's path as SIGN_KEYCHAIN into $GITHUB_ENV.
#   4. Writes APPLE_NOTARY_KEY_P8 to $RUNNER_TEMP/yaar-signing/AuthKey.p8 (mode 600), and
#      NOTARY_KEY=<that path>, NOTARY_KEY_ID, NOTARY_ISSUER into $GITHUB_ENV.
#   5. Checks the notary key before anything is signed: `xcrun notarytool history --key
#      <path> --key-id <id> --issuer <issuer>` (it signs in to Apple and uploads nothing)
#      must exit 0; otherwise refuses with "the notary key was refused by Apple: check its
#      id, its issuer and its access" and notarytool's own message.
#   Prints only: "keychain made; signing identity: <the identity's name>; notary key
#   accepted".
# remove: deletes the keychain (`security delete-keychain`), puts the search list back as it
# was before import, and deletes $RUNNER_TEMP/yaar-signing. Succeeds when there is nothing to
# remove.
#
# No value of a secret is ever printed, put on a command line other tools log, or written
# outside $RUNNER_TEMP/yaar-signing.
#
# Exit codes: 0 done; 1 a refusal or a failed command (a sentence says which); 2 usage error.
set -euo pipefail
umask 077

SELF=$(basename "$0")
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)

usage() { echo "usage: $SELF import|remove" >&2; exit 2; }
fail() { echo "$SELF: $*" >&2; exit 1; }
[ $# -eq 1 ] || usage
MODE=$1
case $MODE in import | remove) ;; *) usage ;; esac

# Never printed, never put in a command: the names of the places only.
[ -n "${RUNNER_TEMP:-}" ] || [ "$MODE" = import ] || fail "RUNNER_TEMP is empty: the keychain of this job cannot be found"
FOLDER="${RUNNER_TEMP:-}/yaar-signing"
KEYCHAIN="${RUNNER_TEMP:-}/yaar-signing.keychain-db"
OLD_LIST="$FOLDER/search-list"

# What a step of this script says when a tool fails: its own words, kept in a file of the
# folder (mode 600) and shown only then. None of them holds a secret.
LOG=""
run() { # <what> <command>...: runs it with its output kept; on failure says what it was
  local what=$1; shift
  if ! "$@" > "$LOG" 2>&1; then
    cat "$LOG" >&2
    fail "$what failed"
  fi
}

if [ "$MODE" = remove ]; then
  # Everything that can be removed is tried, then the first failure is reported: a keychain
  # that stays behind must not keep the key file with it.
  status=0
  if [ -f "$OLD_LIST" ]; then
    old=()
    while IFS= read -r line; do
      if [ -n "$line" ]; then old+=("$line"); fi
    done < "$OLD_LIST"
    if ! security list-keychains -d user -s ${old[@]+"${old[@]}"} > /dev/null 2>&1; then
      echo "$SELF: the keychain search list could not be put back as it was" >&2
      status=1
    fi
  fi
  if [ -e "$KEYCHAIN" ]; then
    if ! security delete-keychain "$KEYCHAIN" > /dev/null 2>&1; then
      echo "$SELF: the keychain $KEYCHAIN could not be deleted" >&2
      status=1
    fi
  fi
  rm -rf "$FOLDER"
  exit "$status"
fi

# ---- import
for v in MACOS_CERTIFICATE_P12 MACOS_CERTIFICATE_PASSWORD APPLE_NOTARY_KEY_P8 APPLE_NOTARY_KEY_ID APPLE_NOTARY_ISSUER_ID; do
  [ -n "${!v:-}" ] || fail "$v is empty: the secret of that name is not set in the environment of this job"
done
# These two land in a file that later steps read line by line: a line break in them would
# add a variable of someone else's choosing.
for v in APPLE_NOTARY_KEY_ID APPLE_NOTARY_ISSUER_ID; do
  case ${!v} in *$'\n'* | *$'\r'*) fail "$v holds a line break: it is one line of text" ;; esac
done
TEAMFILE="$REPO/packaging/macos/team-id"
[ -f "$TEAMFILE" ] || fail "$TEAMFILE is missing"
TEAM=$(sed -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$TEAMFILE" | sed -n '1p')
[ -n "$TEAM" ] || fail "packaging/macos/team-id holds no line that is not a comment: the owner writes the Team ID there once"
[ -n "${RUNNER_TEMP:-}" ] || fail "RUNNER_TEMP is empty: there is no folder of this job to keep the keychain in"
[ -n "${GITHUB_ENV:-}" ] || fail "GITHUB_ENV is empty: there is no file to hand the identity to the next steps in"

# A folder of this job's own, readable by nobody else.
mkdir -p "$FOLDER"
chmod 700 "$FOLDER"
LOG="$FOLDER/last-output"
: > "$LOG"
# Whatever happens below, the key file does not stay on the disk; `remove` takes the rest.
trap 'rm -f "$FOLDER/cert.p12"' EXIT

# 2. The keychain. Its password is random, lives in this shell only and is never printed.
PASSWORD=$(openssl rand -hex 24)
run "reading the keychain search list" security list-keychains -d user
# One path per line, quotes and indentation taken off, kept to put the list back.
sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//' "$LOG" > "$OLD_LIST"
old=()
while IFS= read -r line; do
  if [ -n "$line" ]; then old+=("$line"); fi
done < "$OLD_LIST"
run "creating the keychain" security create-keychain -p "$PASSWORD" "$KEYCHAIN"
run "setting the keychain's lock time" security set-keychain-settings -lut 21600 "$KEYCHAIN"
run "unlocking the keychain" security unlock-keychain -p "$PASSWORD" "$KEYCHAIN"

printf '%s' "$MACOS_CERTIFICATE_P12" | base64 --decode > "$FOLDER/cert.p12" ||
  fail "MACOS_CERTIFICATE_P12 is not base64"
[ -s "$FOLDER/cert.p12" ] || fail "MACOS_CERTIFICATE_P12 decodes to nothing"
run "importing the certificate (a wrong MACOS_CERTIFICATE_PASSWORD gives this too)" \
  security import "$FOLDER/cert.p12" -k "$KEYCHAIN" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -f pkcs12
rm -f "$FOLDER/cert.p12"
run "letting codesign use the key without asking" \
  security set-key-partition-list -S apple-tool:,apple: -s -k "$PASSWORD" "$KEYCHAIN"
# The new keychain first, the user's others after it.
run "putting the keychain in the search list" \
  security list-keychains -d user -s "$KEYCHAIN" ${old[@]+"${old[@]}"}

# 3. The one valid identity, and whose it is.
run "listing the signing identities" security find-identity -v -p codesigning "$KEYCHAIN"
IDENT_RE='^[[:space:]]*[0-9]+\) ([0-9A-F]{40}) "(Developer ID Application: .*)"$'
HASH=""; NAME=""; COUNT=0
while IFS= read -r line; do
  if [[ $line =~ $IDENT_RE ]]; then
    HASH=${BASH_REMATCH[1]}; NAME=${BASH_REMATCH[2]}; COUNT=$((COUNT + 1))
  fi
done < "$LOG"
if [ "$COUNT" -eq 0 ]; then
  fail "no valid Developer ID Application identity in the .p12: export the certificate together with its key and Apple's Developer ID Certification Authority (G2) certificate"
fi
[ "$COUNT" -eq 1 ] || fail "$COUNT valid Developer ID Application identities in the .p12, and the signer is the one: export the one certificate with its key"
TEAM_RE='\(([A-Z0-9]+)\)$'
[[ $NAME =~ $TEAM_RE ]] || fail "the identity \"$NAME\" names no team in brackets"
IDENT_TEAM=${BASH_REMATCH[1]}
[ "$IDENT_TEAM" = "$TEAM" ] ||
  fail "the identity \"$NAME\" belongs to the team $IDENT_TEAM, and packaging/macos/team-id names $TEAM: nothing is signed with another team's certificate"
{
  printf 'SIGN_IDENTITY=%s\n' "$HASH"
  printf 'SIGN_KEYCHAIN=%s\n' "$KEYCHAIN"
} >> "$GITHUB_ENV"

# 4. The notary key: a file of mode 600, and the three values the packaging script reads.
KEYFILE="$FOLDER/AuthKey.p8"
printf '%s\n' "$APPLE_NOTARY_KEY_P8" > "$KEYFILE"
chmod 600 "$KEYFILE"
{
  printf 'NOTARY_KEY=%s\n' "$KEYFILE"
  printf 'NOTARY_KEY_ID=%s\n' "$APPLE_NOTARY_KEY_ID"
  printf 'NOTARY_ISSUER=%s\n' "$APPLE_NOTARY_ISSUER_ID"
} >> "$GITHUB_ENV"

# 5. The key is tried with Apple before anything is signed: `history` signs in and uploads
#    nothing.
if ! xcrun notarytool history --key "$KEYFILE" --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER_ID" > "$LOG" 2>&1; then
  cat "$LOG" >&2
  fail "the notary key was refused by Apple: check its id, its issuer and its access"
fi
: > "$LOG"

echo "keychain made; signing identity: $NAME; notary key accepted"
