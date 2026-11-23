#!/bin/bash
# The same .deb at another version: unpacks <in.deb>, changes only the Version field of its
# control file, packs it again.
#
#   packaging/apt/repack-version.sh <in.deb> <new-version> <out-dir>
#
# Writes <out-dir>/<Package>_<new-version>_<Architecture>.deb and prints its path. Packs as
# packaging/package-linux.sh does (`dpkg-deb --root-owner-group -Zxz --build`), with
# SOURCE_DATE_EPOCH kept. Refuses a version with a '-' (apt would order it wrongly) or one
# that does not sort after the old one (`dpkg --compare-versions <new> gt <old>`).
#
# Exit codes: 0 written; 1 refused or failed; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "repack-version.sh: not built yet" >&2
exit 99
