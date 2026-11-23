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
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-keychain.sh: not built yet" >&2
exit 99
