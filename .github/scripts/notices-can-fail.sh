#!/bin/sh
# The control half of row LIC-02: shows that packaging/notices-count.sh fails when it should,
# so that a count that has never refused anything is not taken for proof.
#
#   .github/scripts/notices-can-fail.sh <packages-dir>
#
# Works on copies under a temporary folder; never changes <packages-dir>. From the archive of
# <packages-dir> that ends in -linux-x86_64.tar.gz it makes, one at a time, a folder that
# holds only a broken copy of that archive, and runs notices-count.sh on it:
#   1. the "Used by:" line of one crate removed (the first crate of PART 1): must fail, and
#      its standard error must name that crate;
#   2. the licence text of one block emptied (the block that names that crate): must fail,
#      naming "no licence text";
#   3. a "Used by:" line added for a crate the lock file does not hold ("planted-crate 9.9.9"):
#      must fail, naming planted-crate;
#   4. the copy unchanged: must pass (the control: the archive as built counts).
# Each check prints what it did and what the count said.
#
# Exit codes: 0 all four as above; 1 one of them not (a sentence says which); 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "notices-can-fail.sh: not built yet" >&2
exit 99
