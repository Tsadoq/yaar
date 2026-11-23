#!/bin/bash
# Row PKG-06: a later version arrives with apt-get upgrade.
#
#   .github/scripts/linux-upgrade.sh <user> <site-later-dir> <later-version>
#
# Run after the install line installed <version> from the site served on port 8000.
#   1. Before: `dpkg-query -W -f='${Version}' yaar-review` is not <later-version>, and
#      `apt-cache madison yaar-review` lists one version.
#   2. serve-site.sh switch <site-later-dir> 8000: the same address now serves the
#      repository with both versions, signed by the same key (a new deployment of the site).
#   3. As <user>, with sudo: `apt-get update`, then `apt-get upgrade -y` (not install).
#   4. After: the installed version is <later-version>; `apt-cache madison yaar-review` lists
#      both versions from 127.0.0.1:8000 (the first was not unsigned by adding the second);
#      `apt-get update` printed no "W:" and no "E:" line; /usr/bin/yaar-review is still the
#      program of the package (`dpkg -S /usr/bin/yaar-review`).
#   5. serve-site.sh switch back to the site of <version>, so that the steps after this one
#      find the site they expect.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "linux-upgrade.sh: not built yet" >&2
exit 99
