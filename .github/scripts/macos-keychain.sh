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
#                                key, exported as a .p12 file, in base64
#   MACOS_CERTIFICATE_PASSWORD   the password the .p12 was exported with
#   APPLE_NOTARY_KEY_P8          the text of the App Store Connect API key file
#                                AuthKey_<key id>.p8
#   APPLE_NOTARY_KEY_ID          that key's id
#   APPLE_NOTARY_ISSUER_ID       the issuer id of the team's keys (a UUID)
#   1. Refuses, naming the variable and never its value, when any is empty.
#   2. Makes a random password (`openssl rand -hex 24`), never printed; creates
#      $RUNNER_TEMP/yaar-signing.keychain-db with it; `security set-keychain-settings -lut
#      21600`; unlocks it; decodes the .p12 to $RUNNER_TEMP/yaar-signing/cert.p12 (folder of
#      mode 700); `security import ... -k <keychain> -P "$MACOS_CERTIFICATE_PASSWORD" -T
#      /usr/bin/codesign -f pkcs12`; `security set-key-partition-list -S apple-tool:,apple:
#      -s -k <password> <keychain>`; puts the keychain first in the user's search list,
#      keeping the others (`security list-keychains -d user -s <keychain> <the old ones>`);
#      deletes cert.p12.
#   3. Finds the one identity "Developer ID Application: ..." in that keychain (`security
#      find-identity -v -p codesigning <keychain>`); refuses when there is none or more than
#      one. Writes its SHA-1 hash as SIGN_IDENTITY and the keychain's path as SIGN_KEYCHAIN
#      into $GITHUB_ENV.
#   4. Writes APPLE_NOTARY_KEY_P8 to $RUNNER_TEMP/yaar-signing/AuthKey.p8 (mode 600), and
#      NOTARY_KEY=<that path>, NOTARY_KEY_ID, NOTARY_ISSUER into $GITHUB_ENV.
#   Prints only: "keychain made; signing identity: <the identity's name>".
# remove: deletes the keychain (`security delete-keychain`), puts the search list back as it
# was before import (kept in $RUNNER_TEMP/yaar-signing/search-list), and deletes
# $RUNNER_TEMP/yaar-signing. Succeeds when there is nothing to remove.
#
# No value of a secret is ever printed, put on a command line other tools log, or written
# outside $RUNNER_TEMP/yaar-signing.
#
# Exit codes: 0 done; 1 a refusal or a failed command (a sentence says which); 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-keychain.sh: not built yet" >&2
exit 99
