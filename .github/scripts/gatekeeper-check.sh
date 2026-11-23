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
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "gatekeeper-check.sh: not built yet" >&2
exit 99
