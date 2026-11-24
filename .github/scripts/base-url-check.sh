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
# "/install.sh" or "/apt" (followed by a space, a quote, a backquote, the end of the line,
# "/", "?" or "#"), and every "URIs: <address>" of an apt source shown there (at the start of
# a line or inside a command or a quote block), is either an address on 127.0.0.1 or
# localhost, or starts with B, or is the placeholder @BASE_URL@ itself. Any other fails, with
# its file, line number and the address. Also fails when a file of the three is missing, or
# when B is not https:// followed by a host, with no "/" at its end.
#
# Exit codes: 0 holds; 1 a text names another address (each named); 2 usage error.
#
# Beyond the header's list: an address is read the way curl and a reader read it, so none
# of these hides one. A quote or a backquote right before a "/" ends no address
# ("https://x"/install.sh is https://x/install.sh); any other quote or backquote ends what it
# follows (["https://x/install.sh","y"] holds https://x/install.sh). Markdown emphasis (*, _
# and ~) around an address is no part of it. A scheme is read in any case (HTTPS://), an
# address may have no scheme at all when it starts with a host that has a dot
# (tsadoq.example.org/install.sh) or is localhost, and "?" and "#" end an address as "/"
# does. A path of the repository (packaging/apt/..., .github/scripts/...) has no dot in its
# first part and is not an address. The scheme is compared in lower case, so
# HTTPS://tsadoq.github.io/yaar/install.sh is a copy of the base. A trailing full stop,
# comma, semicolon or colon is not part of an address ("see https://x/apt."), an address of a
# text is read up to a space, a bracket or a pipe, and a loopback host is the whole host
# 127.0.0.1 or localhost with or without a port: a host that only starts like it
# (127.0.0.1.example.org, localhost@example.org) is another address. A copy of the base is B
# itself or B followed by "/": https://tsadoq.github.io/yaarx/apt is not one. Run with an
# argument it prints its usage and exits 2. It reads the files of the repository it lies in,
# whatever the folder it is run from.
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
    # The scheme of an address in lower case, whatever case the text wrote it in.
    function lower_scheme(a,   s) {
      if (match(a, /^[A-Za-z][A-Za-z]*:\/\//)) {
        s = substr(a, 1, RLENGTH)
        return tolower(s) substr(a, RLENGTH + 1)
      }
      return a
    }
    function is_loopback(a,   h) {
      h = lower_scheme(a)
      if (h ~ /^https?:\/\//) sub(/^https?:\/\//, "", h)
      sub(/[\/?#].*$/, "", h)
      if (index(h, "@") > 0) return 0
      sub(/:[0-9]*$/, "", h)
      return (h == "127.0.0.1" || h == "localhost")
    }
    function allowed(a,   n) {
      a = lower_scheme(a)
      if (is_loopback(a)) return 1
      n = length(base)
      if (substr(a, 1, n) == base && (length(a) == n || substr(a, n + 1, 1) == "/")) return 1
      if (substr(a, 1, 10) == "@BASE_URL@") return 1
      return 0
    }
    # A piece of text without what a sentence or Markdown puts around an address: full
    # stops, commas, semicolons and colons at its end, and emphasis (*, _ and ~) at either end.
    function trimmed(p) {
      while (p ~ /^[*_~]/) p = substr(p, 2)
      while (p ~ /[.,;:*_~]$/) p = substr(p, 1, length(p) - 1)
      return p
    }
    # The address a piece of text holds, if it holds one: the piece up to the first
    # "/install.sh" or "/apt" that is followed by "/", "?", "#" or the end of the piece.
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
          if (nxt == "" || nxt == "/" || nxt == "?" || nxt == "#") {
            if (best == "" || en < length(best)) best = substr(tok, 1, en)
            break
          }
          pos = st + 1
        }
      }
      return best
    }
    # Where an address starts in a piece of text, or 0: a scheme (any case) followed by
    # "://", the placeholder, or a bare host with a dot (or localhost) before its first "/",
    # which curl accepts as an address too. A path of the repository (packaging/apt/...,
    # .github/...) has no dot in its first part and is not one.
    function start_of(piece,   low) {
      low = tolower(piece)
      if (match(low, /https?:\/\//)) return RSTART
      if (match(piece, /@BASE_URL@/)) return RSTART
      if (match(piece, /^[A-Za-z0-9][A-Za-z0-9-]*(\.[A-Za-z0-9-]+)+(:[0-9]+)?\//)) return 1
      if (match(piece, /^localhost(:[0-9]+)?\//)) return 1
      return 0
    }
    {
      line = $0
      # A text saved with CR LF line ends keeps the CR at the end of its line: it is no part
      # of an address, and an address that ends the line must still end there.
      gsub(/\r/, " ", line)
      # A quote or a backquote that stands right before a "/" joins what is before it to
      # what follows: "https://host"/install.sh is the address https://host/install.sh. Any
      # other quote or backquote ends what it follows, so that ["https://x/install.sh","y"]
      # holds the address https://x/install.sh and not https://x/install.sh,y.
      gsub(/["\047`]\//, "/", line)
      gsub(/["\047`]/, " ", line)
      gsub(/[<>()|\[\]]/, " ", line)
      # An apt source shown anywhere in a line (opening it, after "> ", or in a command that
      # writes it) names the address after "URIs:".
      # The field name is read in any case, as apt reads it, and its value is a list: every
      # address that follows the first, as long as each starts with a scheme or is the
      # placeholder, is held as the first is (a command that writes the source goes on with
      # words that are no address, and those are not).
      if (match(tolower(line), /uris:[ \t]*/)) {
        rest = substr(line, RSTART + RLENGTH)
        m = split(rest, uris, /[ \t]+/)
        for (u = 1; u <= m; u++) {
          uri = trimmed(uris[u])
          if (u > 1 && uri !~ /^([A-Za-z][A-Za-z0-9+.-]*:|@BASE_URL@)/) break
          if (uri != "" && !allowed(uri) && !((NR SUBSEP uri) in shown)) {
            shown[NR, uri] = 1
            print NR, uri
          }
        }
      }
      n = split(line, pieces, /[ \t]+/)
      for (j = 1; j <= n; j++) {
        piece = trimmed(pieces[j])
        st = start_of(piece)
        if (st == 0) continue
        a = address_of(substr(piece, st))
        if (a != "" && !allowed(a) && !((NR SUBSEP a) in shown)) {
          shown[NR, a] = 1
          print NR, a
        }
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
