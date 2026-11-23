#!/bin/sh
# Serves a release as GitHub Pages and the release's download address would, on this machine
# only, for the install jobs.
#
#   .github/scripts/serve-site.sh start <site-dir> <release-dir> <port>
#   .github/scripts/serve-site.sh switch <site-dir> <port>
#
# start: makes $RUNNER_TEMP/served-<port> (or ${TMPDIR:-/tmp}/served-<port> without a
# runner), a symbolic link `root` in it to <site-dir> and a folder `root-download` holding
# <release-dir>'s files, and starts `python3 -m http.server <port> --bind 127.0.0.1` in the
# background with the folder served laid out as
#   http://127.0.0.1:<port>/            the site (install.sh, index.html, apt/)
#   http://127.0.0.1:<port>/download/   the release files, as releases/latest/download serves
#                                       them
# (a small python3 handler of its own may do the mapping; python3's standard library only).
# Writes the server's process id to served-<port>/pid and its log to served-<port>/log. Waits,
# up to 10 s, until GET /install.sh answers 200, and fails otherwise with the log.
# switch: points the site of the server on <port> at <site-dir> (the link `root`), so that
# the same address now serves another site, as a new deployment of Pages does; the
# download folder stays. Used by PKG-06.
#
# Exit codes: 0 served; 1 failed; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "serve-site.sh: not built yet" >&2
exit 99
