#!/bin/bash
# Makes a release as files: the release files with one checksum file, the Pages site with the
# install script and the apt repository. The rehearsal runs it with a throwaway key (job
# assemble), a release with the project's key (job assemble-release): the same script both
# times.
#
#   GNUPGHOME=<dir> YAAR_APT_KEY=<fingerprint> [PREVIOUS=<dir>] \
#     packaging/release/assemble.sh <version> <packages-dir> <out-dir>
#
# <packages-dir> holds the six packages of <version> (the two .deb, the two archives, the
# .zip and the .dmg). PREVIOUS, when set, is a folder of the .deb files of earlier published
# releases (fetch-published-debs.sh), which go into the apt repository's pool beside the new
# ones; unset or empty, the pool holds only this version's.
#
# Writes <out-dir>, which must not exist:
#   release/   the six files, named exactly:
#                yaar-review_<version>_amd64.deb      yaar-review_<version>_arm64.deb
#                yaar-review-<version>-linux-x86_64.tar.gz
#                yaar-review-<version>-linux-aarch64.tar.gz
#                yaar-review-<version>-macos-universal.zip
#                yaar-review-<version>-macos-universal.dmg
#              SHA256SUMS   one line per file, "<sha256>  <name>", sorted by name, these six
#                           and nothing else
#              SHA256SUMS.asc   a detached armoured signature of SHA256SUMS by YAAR_APT_KEY
#   site/      what the Pages site serves, and nothing else:
#              index.html   from packaging/site/index.html.in, with @BASE_URL@ and @VERSION@
#                           replaced: what the app is, the install line, the manual routes
#                           (the .dmg, the .deb, the apt source), the uninstall line; no
#                           script, no style or font from another host, no analytics
#              install.sh   packaging/install.sh with its line DEFAULT_BASE_URL="@BASE_URL@"
#                           replaced by the one value of packaging/site/base-url
#              apt/         packaging/apt/make-apt-repo.sh's repository of the new .deb files
#                           and those of PREVIOUS: pool/, dists/stable/{InRelease,Release,
#                           Release.gpg,main/binary-amd64/,main/binary-arm64/}, and
#                           yaar-review.asc, the public key
#              .nojekyll    empty, so that Pages serves the tree as it is
#   key.asc    the public key of YAAR_APT_KEY, beside the two folders (for the tests)
#
# The base address is read from packaging/site/base-url and nowhere else. Refuses, changing
# nothing, when a package is missing, when a file of PREVIOUS has the name of a new one, or
# when a .deb's Version or Architecture does not match its name.
#
# Exit codes: 0 written; 1 refused or failed (a sentence says why); 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "assemble.sh: not built yet" >&2
exit 99
