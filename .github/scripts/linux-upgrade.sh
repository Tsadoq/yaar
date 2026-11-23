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
# The server stays on <site-later-dir>: the steps after this one run on the newest site, as a
# user's system does after a release (the second run of the install line then meets the
# version it has installed as the newest, which is the state a first run leaves).
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "linux-upgrade.sh: not built yet" >&2
exit 99
