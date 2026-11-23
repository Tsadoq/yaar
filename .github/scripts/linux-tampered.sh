#!/bin/bash
# Row PKG-07: apt refuses the repository when its index or a package was changed after
# signing. Run on a fresh client, before anything was installed.
#
#   .github/scripts/linux-tampered.sh <rehearsal-dir>
#
# Makes three copies of <rehearsal-dir>/site under a temporary folder, each changed after the
# signing, and serves each with serve-site.sh on ports 8001, 8002 and 8003:
#   index:    in every dists/stable/main/binary-*/Packages the text "Section: devel" becomes
#             "Section: devil" (same length), and Packages.gz is made again from it
#             (gzip -n -9); InRelease and Release.gpg are left as signed;
#   signed:   one byte inside the signed text of dists/stable/InRelease changed (the first
#             "stable" after "Suite: " becomes "stabla"), and the same in Release, so that
#             what apt refuses is the signature itself and not a hash;
#   package:  the last byte of each pool/**/*.deb flipped (same length).
# For each, as the user tester, runs the install line against that copy
# (YAAR_BASE_URL=http://127.0.0.1:<port>, YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download)
# and asserts:
#   - the install line exits with a status other than 0;
#   - its output holds, for index, "File has unexpected size" or "Hash Sum mismatch"; for
#     signed, "NOSPLIT", "BADSIG" or "signatures were invalid"; for package, "Hash Sum
#     mismatch";
#   - afterwards no yaar-review is installed (`dpkg-query -W yaar-review` fails), and neither
#     /etc/apt/keyrings/yaar-review.asc nor /etc/apt/sources.list.d/yaar-review.sources
#     exists (the install script put them back as they were).
# Then, for index and signed, the refusal is apt's own, not the script's: root writes a
# source and a key of the test's own, /etc/apt/keyrings/yaar-test.asc (the site's
# apt/yaar-review.asc copied) and /etc/apt/sources.list.d/yaar-test.sources (`Types: deb`,
# `URIs: http://127.0.0.1:<port>/apt`, `Suites: stable`, `Components: main`, `Signed-By:
# /etc/apt/keyrings/yaar-test.asc`), runs `apt-get update` on that source only (`-o
# Dir::Etc::sourcelist=/etc/apt/sources.list.d/yaar-test.sources -o
# Dir::Etc::sourceparts=-`), asserts exit 100, and removes both files and apt's lists of
# that source.
# Then the control: the unchanged site on port 8000 answers the same `apt-get update`, through
# the same two files pointed at port 8000, with exit 0 and no "W:" or "E:" line; both files
# are removed after.
# Stops its three servers at the end (serve-site.sh stop).
#
# Exit codes: 0 every copy refused and the control passed; 1 otherwise; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "linux-tampered.sh: not built yet" >&2
exit 99
