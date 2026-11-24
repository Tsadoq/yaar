#!/bin/sh
# Fails when README.md, docs/development.md or docs/releasing.md shows an address ending in
# /install.sh or /apt, or an apt `URIs:` value, not on the base address (packaging/site/base-url),
# 127.0.0.1, localhost or @BASE_URL@. Exit codes: 0 holds; 1 a text or file is wrong; 2 usage.
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
for f in README.md docs/development.md docs/releasing.md; do
  if [ ! -f "$f" ]; then
    echo "$NAME: $ROOT/$f is missing: a text that shows the base address must be there to be held to it" >&2
    FAILED=1
    continue
  fi
  BAD=$(awk -v base="$B" '
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
    function trimmed(p) {
      while (p ~ /^[*_~]/) p = substr(p, 2)
      while (p ~ /[.,;:*_~]$/) p = substr(p, 1, length(p) - 1)
      return p
    }
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
      gsub(/\r/, " ", line)
      gsub(/["\047`]\//, "/", line)
      gsub(/["\047`]/, " ", line)
      gsub(/[<>()|\[\]{}&;]/, " ", line)
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
echo "$NAME: README.md, docs/development.md and docs/releasing.md name the base address $B"
