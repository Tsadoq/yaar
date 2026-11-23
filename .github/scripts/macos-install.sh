#!/bin/bash
# Row PKG-05M: the install line installs the app on macOS and the uninstall removes it.
#
#   .github/scripts/macos-install.sh install
#   .github/scripts/macos-install.sh uninstall
#
# The site and the release files are served on port 8000 (serve-site.sh). The rehearsal's app
# is signed ad hoc and not notarised, so both runs set YAAR_ALLOW_UNNOTARIZED=1, which the
# install script takes, and only from a loopback address, to skip its checks of the
# notarisation and of the team; it still verifies the signature. No other variable changes
# the routes: YAAR_BASE_URL=http://127.0.0.1:8000,
# YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download; the app goes to its default place,
# /Applications, which every assertion below means by /Applications: YAAR_INSTALL_DIR, when
# set (a builder on the owner's Mac sets it to a scratch folder, and HOME and TMPDIR too),
# replaces it. Port: YAAR_TEST_PORT when set (a builder picks a free one), else 8000.
# install:
#   1. The door stays shut away from a loopback address: the install line with
#      YAAR_ALLOW_UNNOTARIZED=1 and YAAR_DOWNLOAD_URL=https://example.invalid/download exits
#      1, says that the variable is taken only from 127.0.0.1 or localhost, downloads
#      nothing (it says so before any download) and leaves no YAAR.app in /Applications.
#   2. `curl -fsSL $YAAR_BASE_URL/install.sh | bash`, exit 0, and its output says the route
#      is macos and that the SHA-256 matched SHA256SUMS;
#   3. /Applications/YAAR.app exists, `codesign --verify --deep --strict` passes on it, its
#      CFBundleShortVersionString is $VERSION, no file of it carries com.apple.quarantine
#      (`xattr -r`), and /Applications holds no `.YAAR.app.new.*` and no `.yaar-review-old.*`;
#   4. the same line run a second time exits 0, says the version is already installed, and
#      leaves the bundle's files as they were (a listing with sizes and modification times of
#      every file, before and after, equal).
# uninstall:
#   1. `curl -fsSL $YAAR_BASE_URL/install.sh | bash -s -- --uninstall`, exit 0;
#   2. /Applications/YAAR.app is gone; no process runs whose program is
#      /Applications/YAAR.app/Contents/MacOS/yaar-review or
#      /Applications/YAAR.app/Contents/Helpers/yaar-keeper (matched by the full path, as the
#      install script's own uninstall matches them: `ps -axo pid=,comm=`; a Keeper of a build
#      folder, another agent's on the owner's Mac among them, is not the app's); the user's
#      lock file of the Keeper is gone
#      (~/Library/Application Support/yaar-review/keeper.sock.lock). What stays (the
#      reviews) is user-files.sh's to judge.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-install.sh: not built yet" >&2
exit 99
