#!/bin/sh
# Writes <version> into [workspace.package] of Cargo.toml and into the entry of every workspace
# member in Cargo.lock, and nothing else. Refuses a version that is not x.y.z, one not greater than
# the present one, and a tree with uncommitted changes. packaging/release/bump.sh <x.y.z>
set -eu

NAME=bump.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
if [ $# -ne 1 ]; then
  echo "usage: packaging/release/bump.sh <x.y.z>" >&2
  exit 2
fi
NEW=$1
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)

if ! printf '%s\n' "$NEW" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
  echo "$NAME: a release version is three numbers x.y.z with no leading zeros, not '$NEW'" >&2
  exit 2
fi
[ -f "$ROOT/Cargo.toml" ] || fail "$ROOT/Cargo.toml is missing"
[ -f "$ROOT/Cargo.lock" ] || fail "$ROOT/Cargo.lock is missing"
OLD=$("$ROOT/packaging/version.sh") || exit 1
if ! awk -v a="$NEW" -v b="$OLD" 'BEGIN {
    split(a, x, "."); split(b, y, ".")
    for (i = 1; i <= 3; i++) {
      if (x[i] + 0 > y[i] + 0) exit 0
      if (x[i] + 0 < y[i] + 0) exit 1
    }
    exit 1 }'; then
  fail "$NEW is not greater than the present version $OLD"
fi
DIRTY=$(git -C "$ROOT" status --porcelain) || fail "$ROOT is not a git repository whose state can be read"
[ -z "$DIRTY" ] || fail "the tree has uncommitted changes, and a bump is the only change a commit of a release holds: $(printf '%s\n' "$DIRTY" | head -n 1)"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bump.XXXXXX") || fail "no scratch folder could be made"
trap 'rm -rf "$TMP"' EXIT

awk -v new="$NEW" '
  /^\[/ { inside = ($0 == "[workspace.package]") }
  inside && !done && /^version[ \t]*=/ { sub(/"[^"]*"/, "\"" new "\""); done = 1 }
  { print }
  END { if (!done) exit 1 }' "$ROOT/Cargo.toml" > "$TMP/Cargo.toml" \
  || fail "Cargo.toml has no version under [workspace.package]"

# A root package is a member the member list does not show, and it is not counted.
grep -Eq '^\[package[].]' "$ROOT/Cargo.toml" \
  && fail "Cargo.toml has a [package] beside [workspace]: its crate is a member this script does not count. Nothing was written"

# The members of the workspace, by name and without cargo: the job that bumps has no toolchain. A
# crate under vendor/ has no source in the lock file either, and can stand at the same version.
# A path dependency inside the workspace folder is a member too, which only cargo can see.
workspace_list() { # <key>: the strings of the array `key` under [workspace] in Cargo.toml
  awk -v key="$1" '
    /^\[/ { inside = ($0 == "[workspace]"); collecting = 0 }
    inside {
      line = $0
      sub(/[ \t]*#.*$/, "", line)
      if (!collecting && line ~ ("^" key "[ \t]*=[ \t]*\\[")) {
        collecting = 1
        sub(/^[^\[]*\[/, "", line)
      }
      if (collecting) {
        if (index(line, "\047")) {
          print "bump.sh: Cargo.toml writes a path of " key " in single quotes, which this script does not read: write it in double quotes. Nothing was written" > "/dev/stderr"
          exit 3
        }
        closed = (line ~ /\]/)
        sub(/\].*$/, "", line)
        while (match(line, /"[^"]*"/)) {
          print substr(line, RSTART + 1, RLENGTH - 2)
          line = substr(line, RSTART + RLENGTH)
        }
        if (closed) collecting = 0
      }
    }' "$ROOT/Cargo.toml"
}
is_glob() { case $1 in *'*'* | *'?'* | *'['*) return 0 ;; *) return 1 ;; esac; }

workspace_list members > "$TMP/patterns" || exit 1
workspace_list exclude > "$TMP/excluded" || exit 1
: > "$TMP/dirs"
while IFS= read -r pattern; do
  pattern=${pattern%/}
  if is_glob "$pattern"; then
    case $pattern in
      */\*) base=${pattern%/\*} ;;
      *) fail "the member pattern '$pattern' of Cargo.toml is a glob this script does not read: only a folder and /* at its end" ;;
    esac
    if is_glob "$base"; then
      fail "the member pattern '$pattern' of Cargo.toml is a glob this script does not read: only a folder and /* at its end"
    fi
    [ -d "$ROOT/$base" ] || fail "the member pattern '$pattern' of Cargo.toml names a folder that is not there"
    for d in "$ROOT/$base"/*/; do
      if [ -f "${d}Cargo.toml" ]; then printf '%s/%s\n' "$base" "$(basename "$d")" >> "$TMP/dirs"; fi
    done
  else
    printf '%s\n' "$pattern" >> "$TMP/dirs"
  fi
done < "$TMP/patterns"
: > "$TMP/members"
while IFS= read -r dir; do
  if grep -qxF -- "$dir" "$TMP/excluded"; then continue; fi
  [ -f "$ROOT/$dir/Cargo.toml" ] || fail "the workspace member $dir has no Cargo.toml"
  name=$(awk '/^\[/ { inside = ($0 == "[package]") }
    inside && /^name[ \t]*=/ { sub(/^name[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit }' "$ROOT/$dir/Cargo.toml") \
    || fail "$dir/Cargo.toml cannot be read"
  [ -n "$name" ] || fail "$dir/Cargo.toml holds no name under [package]: the lock entry of that member cannot be found"
  printf '%s\n' "$name" >> "$TMP/members"
done < "$TMP/dirs"
MEMBERS=$(wc -l < "$TMP/members" | tr -d ' ')
[ "$MEMBERS" -gt 0 ] || fail "Cargo.toml names no workspace member"

: > "$TMP/changed"
awk -v members="$TMP/members" -v changed="$TMP/changed" -v old="version = \"$OLD\"" -v new="version = \"$NEW\"" '
  BEGIN { while ((getline line < members) > 0) member[line] = 1 }
  function flush(   i, name) {
    name = ""
    for (i = 1; i <= n; i++) {
      if (buf[i] ~ /^name = "/) { name = buf[i]; sub(/^name = "/, "", name); sub(/"$/, "", name) }
    }
    for (i = 1; i <= n; i++) {
      if (!sourced && name in member && buf[i] == old) { buf[i] = new; print name >> changed }
      print buf[i]
    }
    n = 0
    sourced = 0
  }
  $0 == "[[package]]" { flush() }
  /^source = / { sourced = 1 }
  { buf[++n] = $0 }
  END { flush() }' "$ROOT/Cargo.lock" > "$TMP/Cargo.lock" || fail "Cargo.lock cannot be read"
CHANGED=$(wc -l < "$TMP/changed" | tr -d ' ')
if [ "$CHANGED" -ne "$MEMBERS" ]; then
  LC_ALL=C sort "$TMP/members" > "$TMP/members.sorted"
  LC_ALL=C sort "$TMP/changed" > "$TMP/changed.sorted"
  LEFT=$(comm -23 "$TMP/members.sorted" "$TMP/changed.sorted" | tr '\n' ' ')
  fail "Cargo.lock has $CHANGED entries at $OLD that are members, and the workspace has $MEMBERS members; those not found at $OLD without a source: ${LEFT:-none}. Nothing was written"
fi

cat "$TMP/Cargo.toml" > "$ROOT/Cargo.toml"
cat "$TMP/Cargo.lock" > "$ROOT/Cargo.lock"
echo "$NAME: Cargo.toml and $CHANGED workspace members of Cargo.lock go from $OLD to $NEW"
