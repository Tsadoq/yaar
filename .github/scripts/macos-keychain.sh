#!/bin/bash
# The signing keychain of the job release-sign-macos, made from the secrets and removed at the end.
# .github/scripts/macos-keychain.sh import | remove  (import writes SIGN_*, NOTARY_* to $GITHUB_ENV)
# The .p12 must hold Apple's Developer ID CA (G2) too, or the identity is invalid elsewhere.
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

[ -n "${RUNNER_TEMP:-}" ] || [ "$MODE" = import ] || fail "RUNNER_TEMP is empty: the keychain of this job cannot be found"
FOLDER="${RUNNER_TEMP:-}/yaar-signing"
KEYCHAIN="${RUNNER_TEMP:-}/yaar-signing.keychain-db"
OLD_LIST="$FOLDER/search-list"

LOG=""
run() { # <what> <command>...: runs it with its output kept; on failure says what it was
  local what=$1; shift
  if ! "$@" > "$LOG" 2>&1; then
    cat "$LOG" >&2
    fail "$what failed"
  fi
}

if [ "$MODE" = remove ]; then
  # Every removal is tried even after one fails, so a keychain left behind does not keep the key
  # file with it; each failure is named and the exit status is 1.
  status=0
  if [ -f "$OLD_LIST" ]; then
    old=()
    while IFS= read -r line; do
      if [ -n "$line" ]; then old+=("$line"); fi
    done < "$OLD_LIST"
    if [ ${#old[@]} -gt 0 ] && ! security list-keychains -d user -s "${old[@]}" > /dev/null 2>&1; then
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

mkdir -p "$FOLDER"
chmod 700 "$FOLDER"
LOG="$FOLDER/last-output"
: > "$LOG"
trap 'rm -f "$FOLDER/cert.p12"' EXIT

PASSWORD=$(openssl rand -hex 24)
run "reading the keychain search list" security list-keychains -d user
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
run "putting the keychain in the search list" \
  security list-keychains -d user -s "$KEYCHAIN" ${old[@]+"${old[@]}"}

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

KEYFILE="$FOLDER/AuthKey.p8"
printf '%s\n' "$APPLE_NOTARY_KEY_P8" > "$KEYFILE"
chmod 600 "$KEYFILE"
{
  printf 'NOTARY_KEY=%s\n' "$KEYFILE"
  printf 'NOTARY_KEY_ID=%s\n' "$APPLE_NOTARY_KEY_ID"
  printf 'NOTARY_ISSUER=%s\n' "$APPLE_NOTARY_ISSUER_ID"
} >> "$GITHUB_ENV"

# `history` signs in to Apple and uploads nothing, so a bad key shows before anything is signed.
if ! xcrun notarytool history --key "$KEYFILE" --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER_ID" > "$LOG" 2>&1; then
  cat "$LOG" >&2
  fail "the notary key was refused by Apple: check its id, its issuer and its access"
fi
: > "$LOG"

echo "keychain made; signing identity: $NAME; notary key accepted"
