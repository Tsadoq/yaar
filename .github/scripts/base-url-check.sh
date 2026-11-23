#!/bin/sh
# The step "The texts name the one base address" of the job assemble: the site's base
# address is one line, packaging/site/base-url, and a text that has to show it holds a copy
# this step holds to that line, so that moving the site to a domain is that line and the
# copies this step then names.
#
#   .github/scripts/base-url-check.sh
#
# Reads the base, B, as the first line of packaging/site/base-url (`head -n 1`). In
# README.md, packaging/README.md and packaging/release-day.md, every address that ends in
# "/install.sh" or "/apt" (followed by a space, a quote, a backquote, the end of the line or
# "/"), and every line "URIs: <address>" of an apt source shown there, is either an address
# on 127.0.0.1 or localhost, or starts with B, or is the placeholder @BASE_URL@ itself. Any
# other fails, with its file, line number and the address. Also fails when a file of the
# three is missing, or when B is not https:// followed by a host, with no "/" at its end.
#
# Exit codes: 0 holds; 1 a text names another address (each named); 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "base-url-check.sh: not built yet" >&2
exit 99
