#!/bin/bash
# The rehearsal's second version, for row PKG-06 (a later version arrives with apt-get
# upgrade): beside <out-dir>/site it writes <out-dir>/site-later, the same site whose apt
# repository also holds both .deb files repacked as <later-version>, signed again with the
# same key. Never part of a release: the job assemble runs it, assemble-release does not.
#
#   GNUPGHOME=<dir> YAAR_APT_KEY=<fingerprint> packaging/release/later.sh <out-dir> <later-version>
#
# Copies <out-dir>/site to <out-dir>/site-later; for each .deb of <out-dir>/release runs
# packaging/apt/repack-version.sh into a temporary folder; adds the results with
# make-apt-repo.sh to site-later/apt. The pool then holds both versions of each processor,
# and the first version's .deb keeps its bytes (the check: cmp against release/).
#
# Exit codes: 0 written; 1 failed; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "later.sh: not built yet" >&2
exit 99
