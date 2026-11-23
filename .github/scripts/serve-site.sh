#!/bin/sh
# Serves a release as GitHub Pages and the release's download address would, on this machine
# only, for the install jobs.
#
#   .github/scripts/serve-site.sh start <site-dir> <release-dir> <port>
#   .github/scripts/serve-site.sh switch <site-dir> <port>
#   .github/scripts/serve-site.sh stop <port>
#
# start: makes $RUNNER_TEMP/served-<port> (or ${TMPDIR:-/tmp}/served-<port> without a
# runner), a symbolic link `root` in it to <site-dir> and a folder `root-download` holding
# <release-dir>'s files, and starts `python3 -m http.server <port> --bind 127.0.0.1` in the
# background with the folder served laid out as
#   http://127.0.0.1:<port>/            the site (install.sh, index.html, apt/)
#   http://127.0.0.1:<port>/download/   the release files, as releases/latest/download serves
#                                       them
# (a small python3 handler of its own may do the mapping; python3's standard library only).
# The server is detached from the step entirely, or a container's step waits for it and
# hangs: `setsid` (else `nohup`), standard input from /dev/null, standard output and
# standard error both to served-<port>/log, never to the step's own streams.
# Writes the server's process id to served-<port>/pid. Waits, up to 10 s, until GET
# /install.sh answers 200, and fails otherwise with the log.
# switch: points the site of the server on <port> at <site-dir> (the link `root`), so that
# the same address now serves another site, as a new deployment of Pages does; the
# download folder stays. Used by PKG-06.
# stop: ends the server of <port> by the pid it wrote (never by name) and waits for it.
#
# Exit codes: 0 served; 1 failed; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "serve-site.sh: not built yet" >&2
exit 99
