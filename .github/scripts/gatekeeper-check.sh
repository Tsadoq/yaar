#!/bin/bash
# The step "Gatekeeper accepts the notarized disk image and the app" of release-sign-macos
# (row PKG-04, which runs at a release): what a user's Mac will say of the release, checked on
# the files that will be published.
#
#   .github/scripts/gatekeeper-check.sh <dir> <version>
#
# <dir> holds yaar-review-<version>-macos-universal.zip and .dmg, signed with the Developer ID
# and notarised. Unpacks the .zip with `ditto -x -k` and mounts the .dmg read-only
# (`hdiutil attach -nobrowse -readonly -mountpoint <temp>`, detached at the end whatever
# happens). Fails with a sentence when any of these does not hold:
#   - `codesign --verify --deep --strict` passes on both apps;
#   - `spctl --assess --type execute -vv` on both apps says "accepted" and
#     "source=Notarized Developer ID";
#   - `spctl --assess --type open --context context:primary-signature -vv` on the .dmg says
#     "accepted" and "source=Notarized Developer ID";
#   - `xcrun stapler validate` passes on the app of the .zip, on the app of the .dmg and on
#     the .dmg;
#   - the apps' CFBundleShortVersionString is <version>.
# Then runs the install line against these files (as .github/scripts/macos-install.sh
# install does, but without YAAR_ALLOW_UNNOTARIZED) into a temporary folder named by
# YAAR_INSTALL_DIR, and removes it again.
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "gatekeeper-check.sh: not built yet" >&2
exit 99
