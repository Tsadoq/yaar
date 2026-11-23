#!/bin/bash
# Row PKG-07: apt refuses the repository when its index or a package was changed after
# signing. Run on a fresh client, before anything was installed.
#
#   .github/scripts/linux-tampered.sh <rehearsal-dir>
#
# Makes two copies of <rehearsal-dir>/site under a temporary folder, each changed after the
# signing, and serves each with serve-site.sh on ports 8001 and 8002:
#   index:    in every dists/stable/main/binary-*/Packages the text "Section: devel" becomes
#             "Section: devil" (same length), and Packages.gz is made again from it
#             (gzip -n -9);
#   package:  the last byte of each pool/**/*.deb flipped (same length).
# For each, as the user tester, runs the install line against that copy
# (YAAR_BASE_URL=http://127.0.0.1:<port>, YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download)
# and asserts:
#   - the install line exits with a status other than 0;
#   - its output holds, for index, "File has unexpected size" or "Hash Sum mismatch", and for
#     package, "Hash Sum mismatch";
#   - for index, `apt-get update` run once more by root on that source exits 100;
#   - afterwards no yaar-review is installed (`dpkg-query -W yaar-review` fails), and neither
#     /etc/apt/keyrings/yaar-review.asc nor /etc/apt/sources.list.d/yaar-review.sources
#     exists (the install script put them back as they were).
# Then the control: the unchanged site on port 8000 answers `apt-get update` with exit 0 from
# the same client, through a source file written by the test itself and removed after.
# Stops its two servers at the end.
#
# Exit codes: 0 both refused and the control passed; 1 otherwise; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "linux-tampered.sh: not built yet" >&2
exit 99
