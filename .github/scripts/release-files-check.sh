#!/bin/sh
# The step "One checksum file lists every package of the release" (jobs assemble and
# assemble-release; the release row PKG-10's first clause).
#
#   .github/scripts/release-files-check.sh <release-dir> <version>
#
# Fails with a sentence when:
#   - <release-dir> holds any file but the six of packaging/release/assemble.sh for
#     <version>, SHA256SUMS and SHA256SUMS.asc;
#   - SHA256SUMS lists anything but those six, or one of them twice, or a sum that
#     `sha256sum -c` does not confirm;
#   - SHA256SUMS.asc does not verify against SHA256SUMS with the key beside <release-dir>
#     (<release-dir>/../key.asc, with gpgv and a keyring made from it in a temporary folder).
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "release-files-check.sh: not built yet" >&2
exit 99
