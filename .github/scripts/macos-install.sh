#!/bin/bash
# Row PKG-05M: the install line installs the app on macOS and the uninstall removes it.
#
#   .github/scripts/macos-install.sh install
#   .github/scripts/macos-install.sh uninstall
#
# The site and the release files are served on port 8000 (serve-site.sh). The rehearsal's app
# is signed ad hoc and not notarised, so both runs set YAAR_ALLOW_UNNOTARIZED=1, which the
# install script takes only to skip its check of the notarisation; it still verifies the
# signature. No other variable changes the routes: YAAR_BASE_URL=http://127.0.0.1:8000,
# YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download; the app goes to its default place,
# /Applications, which every assertion below means by /Applications: YAAR_INSTALL_DIR, when
# set (a builder on the owner's Mac sets it to a scratch folder, and HOME too), replaces it.
# install:
#   1. `curl -fsSL $YAAR_BASE_URL/install.sh | bash`, exit 0, and its output says the route
#      is macos and that the SHA-256 matched SHA256SUMS;
#   2. /Applications/YAAR.app exists, `codesign --verify --deep --strict` passes on it, its
#      CFBundleShortVersionString is $VERSION, and no file of it carries
#      com.apple.quarantine (`xattr -r`);
#   3. the same line run a second time exits 0, says the version is already installed, and
#      leaves the bundle's files as they were (a listing with sizes and modification times of
#      every file, before and after, equal).
# uninstall:
#   1. `curl -fsSL $YAAR_BASE_URL/install.sh | bash -s -- --uninstall`, exit 0;
#   2. /Applications/YAAR.app is gone; no process yaar-review or yaar-keeper of the user
#      runs; the user's lock file of the Keeper is gone
#      (~/Library/Application Support/yaar-review/keeper.sock.lock).
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-install.sh: not built yet" >&2
exit 99
