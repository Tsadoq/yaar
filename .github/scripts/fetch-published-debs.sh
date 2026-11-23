#!/bin/bash
# The .deb files of the releases already published, for the pool of the apt repository, so
# that a new release adds its packages beside the older ones.
#
#   GH_TOKEN=<token> .github/scripts/fetch-published-debs.sh <out-dir>
#
# Lists the published releases of this repository (`gh release list --exclude-drafts
# --json tagName`); for each, downloads its *.deb and its SHA256SUMS into a folder of its own
# (`gh release download <tag> --pattern '*.deb' --pattern SHA256SUMS`), checks each .deb
# against that SHA256SUMS (refuses a mismatch), and moves the .deb files into <out-dir>.
# None published: <out-dir> is made empty. Reaches GitHub's own addresses only.
#
# Exit codes: 0 done; 1 a mismatch or a failed download; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "fetch-published-debs.sh: not built yet" >&2
exit 99
