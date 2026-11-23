#!/bin/bash
# The rehearsal's second version, for row PKG-06 (a later version arrives with apt-get
# upgrade): beside <out-dir>/site it writes <out-dir>/site-later, the same site whose apt
# repository also holds both .deb files repacked as <later-version>, signed again with the
# same key. Never part of a release: the job assemble runs it, assemble-release does not.
#
#   GNUPGHOME=<dir> YAAR_APT_KEY=<fingerprint> packaging/release/later.sh <out-dir> <later-version>
#
# Copies <out-dir>/site to <out-dir>/site-later; for each .deb of <out-dir>/release runs
# packaging/apt/repack-version.sh into a temporary folder; adds the results with
# make-apt-repo.sh to site-later/apt. The pool then holds both versions of each processor,
# and the first version's .deb keeps its bytes (the check: cmp against release/).
#
# Exit codes: 0 written; 1 failed; 2 usage error.
set -euo pipefail
umask 022
NAME=later.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
if [ $# -ne 2 ]; then
  sed -n '7p' "$0" >&2
  exit 2
fi
OUTDIR=$1
LATER=$2
if [ -z "${YAAR_APT_KEY:-}" ] || [ -z "${GNUPGHOME:-}" ]; then
  echo "$NAME: set YAAR_APT_KEY to the fingerprint of the signing key and GNUPGHOME to the folder that holds it" >&2
  exit 2
fi
export GNUPGHOME
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)

[ -d "$OUTDIR/site" ] && [ -d "$OUTDIR/release" ] || fail "$OUTDIR holds no site/ and release/ of assemble.sh"
OUTDIR=$(cd "$OUTDIR" && pwd)
[ ! -e "$OUTDIR/site-later" ] && [ ! -L "$OUTDIR/site-later" ] || fail "$OUTDIR/site-later exists already"
DEBS=$(find "$OUTDIR/release" -maxdepth 1 -type f -name '*.deb' | LC_ALL=C sort)
[ -n "$DEBS" ] || fail "$OUTDIR/release holds no .deb"

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-later.XXXXXX")
MADE=0
cleanup() {
  status=$?
  rm -rf "$SCRATCH"
  if [ "$status" -ne 0 ] && [ "$MADE" -eq 1 ]; then rm -rf "$OUTDIR/site-later"; fi
}
trap cleanup EXIT

# Repack first: a version that does not sort after the old one is refused before anything is
# copied.
REPACKED=()
while IFS= read -r deb; do
  REPACKED+=("$("$ROOT/packaging/apt/repack-version.sh" "$deb" "$LATER" "$SCRATCH/later")")
done <<< "$DEBS"

cp -a "$OUTDIR/site" "$OUTDIR/site-later"
MADE=1
"$ROOT/packaging/apt/make-apt-repo.sh" "$OUTDIR/site-later/apt" "${REPACKED[@]}"

# The first version keeps its bytes, and both versions are in the pool.
while IFS= read -r deb; do
  name=$(basename "$deb")
  pooled=$(find "$OUTDIR/site-later/apt/pool" -type f -name "$name")
  [ -n "$pooled" ] || fail "$name is not in the pool of $OUTDIR/site-later/apt"
  cmp -s "$deb" "$pooled" || fail "$pooled is not the .deb of $OUTDIR/release/$name: a published version must keep its bytes"
done <<< "$DEBS"
for later in "${REPACKED[@]}"; do
  name=$(basename "$later")
  pooled=$(find "$OUTDIR/site-later/apt/pool" -type f -name "$name")
  [ -n "$pooled" ] || fail "$name is not in the pool of $OUTDIR/site-later/apt"
done
echo "$NAME: wrote $OUTDIR/site-later: the pool holds both versions of each processor, $LATER the later"
