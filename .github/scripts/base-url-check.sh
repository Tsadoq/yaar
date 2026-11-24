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
# Beyond the header's list: a trailing full stop, comma, semicolon or colon is not part of an
# address ("see https://x/apt."), an address of a text is read up to a space, a quote, a
# backquote, a bracket or a pipe, and a loopback host is the whole host 127.0.0.1 or
# localhost with or without a port: a host that only starts like it (127.0.0.1.example.org,
# localhost@example.org) is another address. A copy of the base is B itself or B followed by
# "/": https://tsadoq.github.io/yaarx/apt is not one. Run with an argument it prints its
# usage and exits 2. It reads the files of the repository it lies in, whatever the folder it
# is run from.
set -eu

NAME=base-url-check.sh
if [ $# -ne 0 ]; then
  echo "usage: $NAME (no argument; it reads packaging/site/base-url and three texts)" >&2
  exit 2
fi
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
cd "$ROOT"

BASE_FILE=packaging/site/base-url
if [ ! -f "$BASE_FILE" ]; then
  echo "$NAME: $ROOT/$BASE_FILE is missing: the base address is its first line" >&2
  exit 1
fi
B=$(head -n 1 "$BASE_FILE") || { echo "$NAME: $ROOT/$BASE_FILE cannot be read" >&2; exit 1; }
case $B in
  *[[:space:]]*)
    echo "$NAME: $BASE_FILE: the base address holds a space: $B" >&2
    exit 1
    ;;
  */)
    echo "$NAME: $BASE_FILE: the base address must not end in '/': $B" >&2
    exit 1
    ;;
  https://[!/]*) ;;
  *)
    echo "$NAME: $BASE_FILE: the base address must be https:// and a host, not: ${B:-nothing}" >&2
    exit 1
    ;;
esac

FAILED=0
for f in README.md packaging/README.md packaging/release-day.md; do
  if [ ! -f "$f" ]; then
    echo "$NAME: $ROOT/$f is missing: a text that shows the base address must be there to be held to it" >&2
    FAILED=1
    continue
  fi
  # One line of output for each address that is not allowed: "<line> <address>".
  BAD=$(awk -v base="$B" '
    function is_loopback(a,   h) {
      if (a !~ /^https?:\/\//) return 0
      h = a
      sub(/^https?:\/\//, "", h)
      sub(/[\/?#].*$/, "", h)
      if (index(h, "@") > 0) return 0
      sub(/:[0-9]*$/, "", h)
      return (h == "127.0.0.1" || h == "localhost")
    }
    function allowed(a,   n) {
      if (is_loopback(a)) return 1
      n = length(base)
      if (substr(a, 1, n) == base && (length(a) == n || substr(a, n + 1, 1) == "/")) return 1
      if (substr(a, 1, 10) == "@BASE_URL@") return 1
      return 0
    }
    # The address a token holds, if it holds one: the token up to the first "/install.sh" or
    # "/apt" that is followed by "/" or by the end of the token.
    function address_of(tok,   best, k, suf, pos, i, st, en, nxt, sufs) {
      best = ""
      split("/install.sh /apt", sufs, " ")
      for (k = 1; k <= 2; k++) {
        suf = sufs[k]
        pos = 1
        while ((i = index(substr(tok, pos), suf)) > 0) {
          st = pos + i - 1
          en = st + length(suf) - 1
          nxt = substr(tok, en + 1, 1)
          if (nxt == "" || nxt == "/") {
            if (best == "" || en < length(best)) best = substr(tok, 1, en)
            break
          }
          pos = st + 1
        }
      }
      return best
    }
    {
      line = $0
      if (match(line, /^[ \t]*URIs:[ \t]*/)) {
        rest = substr(line, RLENGTH + 1)
        sub(/[ \t].*$/, "", rest)
        if (rest != "" && !allowed(rest)) print NR, rest
        next
      }
      while (match(line, /(https?:\/\/|@BASE_URL@)[^[:space:]"\047`<>()|]*/)) {
        tok = substr(line, RSTART, RLENGTH)
        line = substr(line, RSTART + RLENGTH)
        while (tok ~ /[.,;:]$/) tok = substr(tok, 1, length(tok) - 1)
        a = address_of(tok)
        if (a != "" && !allowed(a)) print NR, a
      }
    }' "$f") || { echo "$NAME: $f could not be read" >&2; FAILED=1; continue; }
  if [ -n "$BAD" ]; then
    FAILED=1
    printf '%s\n' "$BAD" | while IFS=' ' read -r n a; do
      echo "$NAME: $f:$n: the address $a is not on the base address $B (the first line of $BASE_FILE), nor on 127.0.0.1 or localhost, nor the placeholder @BASE_URL@" >&2
    done
  fi
done
[ "$FAILED" -eq 0 ] || exit 1
echo "$NAME: README.md, packaging/README.md and packaging/release-day.md name the base address $B"
