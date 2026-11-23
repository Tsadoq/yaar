#!/bin/sh
# The step "One checksum file lists every package of the release" (jobs assemble and
# assemble-release; the release row PKG-10's first clause).
#
#   .github/scripts/release-files-check.sh <release-dir> <version>
#
# Fails with a sentence when:
#   - <release-dir> holds any file but the six of packaging/release/assemble.sh for
#     <version>, SHA256SUMS and SHA256SUMS.asc;
#   - SHA256SUMS lists anything but those six, or one of them twice, or a sum that
#     `sha256sum -c` does not confirm;
#   - SHA256SUMS.asc does not verify against SHA256SUMS with the key beside <release-dir>
#     (<release-dir>/../key.asc, with gpgv and a keyring made from it in a temporary folder).
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Every fault found is named on its own line of standard error, then the script exits 1: a
# user of the step sees all that is wrong at once. A missing tool (sha256sum, gpg, gpgv) is a
# fault, never a pass.
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

# The six packages, the checksum file and its signature, one name a line, sorted by byte.
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

# 1. The folder holds these eight and nothing else (a folder, a link or a hidden file is
# something else too).
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

# 2. SHA256SUMS lists the six, each once, as sha256sum writes a line: 64 hexadecimal digits,
# two spaces, the name.
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

# 3. Every sum is the file's own. Only when the listing is the six, so that the verdict of
# sha256sum is not buried in the faults of step 2.
if ! command -v sha256sum > /dev/null 2>&1; then
  fault "sha256sum is not installed, so no sum was checked"
elif [ "$LISTING_OK" -eq 1 ] && [ ! -s "$TMP/missing" ]; then
  if ! (cd "$DIR" && sha256sum -c --strict SHA256SUMS) > "$TMP/sum.out" 2>&1; then
    fault "$SUMS does not match the files: $(grep -v ': OK$' "$TMP/sum.out" | tr '\n' ' ')"
  fi
fi

# 4. SHA256SUMS.asc verifies SHA256SUMS with the key beside the folder only: a keyring made
# from that file in a folder of its own, so that no key of the machine can stand in.
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
