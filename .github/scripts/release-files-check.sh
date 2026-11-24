#!/bin/sh
# One checksum file lists every package of the release, and its signature verifies.
# .github/scripts/release-files-check.sh <release-dir> <version>
# SHA256SUMS.asc is verified with ../key.asc in a fresh keyring; a missing tool is a fault.
set -eu

NAME=release-files-check.sh
if [ $# -ne 2 ]; then
  echo "usage: $NAME <release-dir> <version>" >&2
  exit 2
fi
DIR=$1
VERSION=$2
case $VERSION in
  '' | *[!0-9A-Za-z.+~]*)
    echo "$NAME: the version must be letters, digits, '.', '+' and '~' only, not: $VERSION" >&2
    exit 2
    ;;
esac
if [ ! -d "$DIR" ]; then
  echo "$NAME: $DIR is not a folder" >&2
  exit 1
fi

TMP=$(mktemp -d "${TMPDIR:-/tmp}/yaar-release-files-check.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
FAILED=0
fault() {
  echo "$NAME: $*" >&2
  FAILED=1
}

{
  printf '%s\n' \
    "yaar-review_${VERSION}_amd64.deb" \
    "yaar-review_${VERSION}_arm64.deb" \
    "yaar-review-${VERSION}-linux-x86_64.tar.gz" \
    "yaar-review-${VERSION}-linux-aarch64.tar.gz" \
    "yaar-review-${VERSION}-macos-universal.zip" \
    "yaar-review-${VERSION}-macos-universal.dmg"
} | LC_ALL=C sort > "$TMP/packages"
{ cat "$TMP/packages"; printf '%s\n' SHA256SUMS SHA256SUMS.asc; } | LC_ALL=C sort > "$TMP/expected"

if ! LC_ALL=C ls -A "$DIR" > "$TMP/present" 2> "$TMP/ls.err"; then
  echo "$NAME: $DIR cannot be listed: $(tr '\n' ' ' < "$TMP/ls.err")" >&2
  exit 1
fi
LC_ALL=C comm -13 "$TMP/expected" "$TMP/present" > "$TMP/extra"
LC_ALL=C comm -23 "$TMP/expected" "$TMP/present" > "$TMP/missing"
while IFS= read -r f; do
  fault "$DIR holds $f, which is no file of the release of $VERSION"
done < "$TMP/extra"
while IFS= read -r f; do
  fault "$DIR/$f is missing"
done < "$TMP/missing"
while IFS= read -r f; do
  if [ -e "$DIR/$f" ] || [ -L "$DIR/$f" ]; then
    if [ -L "$DIR/$f" ] || [ ! -f "$DIR/$f" ]; then
      fault "$DIR/$f is not a plain file"
    fi
  fi
done < "$TMP/expected"

SUMS=$DIR/SHA256SUMS
LISTING_OK=1
if [ -f "$SUMS" ] && [ ! -L "$SUMS" ]; then
  : > "$TMP/listed"
  LINE=0
  while IFS= read -r l || [ -n "$l" ]; do
    LINE=$((LINE + 1))
    hash=${l%%  *}
    name=${l#*  }
    case $hash in
      *[!0-9a-f]* | '') bad=1 ;;
      *) if [ "${#hash}" -eq 64 ] && [ "$l" = "$hash  $name" ] && [ -n "$name" ]; then bad=0; else bad=1; fi ;;
    esac
    if [ "$bad" -eq 1 ]; then
      fault "$SUMS line $LINE is not '<sha-256>  <name>' as sha256sum writes it: $l"
      LISTING_OK=0
      continue
    fi
    printf '%s\n' "$name" >> "$TMP/listed"
  done < "$SUMS"
  if ! LC_ALL=C sort -c "$TMP/listed" 2> /dev/null; then
    fault "$SUMS is not sorted by name, as assemble.sh writes it"
  fi
  LC_ALL=C sort "$TMP/listed" > "$TMP/listed.sorted"
  LC_ALL=C uniq -d "$TMP/listed.sorted" > "$TMP/twice"
  while IFS= read -r f; do
    fault "$SUMS lists $f more than once"
    LISTING_OK=0
  done < "$TMP/twice"
  LC_ALL=C sort -u "$TMP/listed.sorted" > "$TMP/listed.unique"
  LC_ALL=C comm -13 "$TMP/packages" "$TMP/listed.unique" > "$TMP/unknown"
  LC_ALL=C comm -23 "$TMP/packages" "$TMP/listed.unique" > "$TMP/unlisted"
  while IFS= read -r f; do
    fault "$SUMS lists $f, which is not one of the six packages of $VERSION"
    LISTING_OK=0
  done < "$TMP/unknown"
  while IFS= read -r f; do
    fault "$SUMS does not list $f"
    LISTING_OK=0
  done < "$TMP/unlisted"
else
  LISTING_OK=0 # SHA256SUMS missing or not a plain file: named in step 1
fi

if ! command -v sha256sum > /dev/null 2>&1; then
  fault "sha256sum is not installed, so no sum was checked"
elif [ "$LISTING_OK" -eq 1 ] && [ ! -s "$TMP/missing" ]; then
  if ! (cd "$DIR" && sha256sum -c --strict SHA256SUMS) > "$TMP/sum.out" 2>&1; then
    fault "$SUMS does not match the files: $(grep -v ': OK$' "$TMP/sum.out" | tr '\n' ' ')"
  fi
fi

KEY=$DIR/../key.asc
if ! command -v gpg > /dev/null 2>&1 || ! command -v gpgv > /dev/null 2>&1; then
  fault "gpg and gpgv are needed to check $DIR/SHA256SUMS.asc and one is not installed"
elif [ ! -f "$KEY" ]; then
  fault "$KEY is missing, so $DIR/SHA256SUMS.asc cannot be checked"
elif [ ! -f "$DIR/SHA256SUMS.asc" ] || [ ! -f "$SUMS" ]; then
  : # named in step 1
else
  mkdir -m 700 "$TMP/gnupg"
  if ! GNUPGHOME=$TMP/gnupg gpg --batch --quiet --dearmor < "$KEY" > "$TMP/key.gpg" 2> "$TMP/gpg.err" \
    || [ ! -s "$TMP/key.gpg" ]; then
    fault "$KEY is not an armoured public key: $(tr '\n' ' ' < "$TMP/gpg.err")"
  elif ! GNUPGHOME=$TMP/gnupg gpgv --keyring "$TMP/key.gpg" "$DIR/SHA256SUMS.asc" "$SUMS" > "$TMP/gpgv.out" 2>&1; then
    fault "$DIR/SHA256SUMS.asc does not verify SHA256SUMS with $KEY: $(tr '\n' ' ' < "$TMP/gpgv.out")"
  fi
fi

if [ "$FAILED" -ne 0 ]; then
  exit 1
fi
echo "$NAME: $DIR holds the six packages of $VERSION, one SHA256SUMS that lists each once and its signature, and nothing else"
