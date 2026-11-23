#!/bin/bash
# The step "The index and the checksums verify with the key the site publishes" of
# assemble-release: what a user's apt and a user who checks by hand will verify.
#
#   .github/scripts/verify-signed.sh <final-dir>
#
# With a keyring made in a temporary folder from <final-dir>/site/apt/yaar-review.asc alone:
# `gpgv` accepts site/apt/dists/stable/InRelease, site/apt/dists/stable/Release.gpg against
# Release, and release/SHA256SUMS.asc against release/SHA256SUMS; the key's fingerprint
# equals packaging/apt/project-key.fingerprint; every Packages file's hash is the one
# InRelease names; every .deb the Packages files name is in the pool with the size and SHA-256
# they name. And the site's install.sh is the one a user may run: it holds neither
# "@BASE_URL@" nor "@TEAM_ID@", its DEFAULT_BASE_URL is the line of packaging/site/base-url,
# and its team id is the line of packaging/macos/team-id.
#
# Exit codes: 0 all verify; 1 one does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "verify-signed.sh: not built yet" >&2
exit 99
