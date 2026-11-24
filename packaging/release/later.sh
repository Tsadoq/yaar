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
# and the first version's .deb keeps its bytes (the check: cmp against release/). Each repacked
# file is also looked up in the pool, where it must hold the later version.
#
# The later index is signed in a later second than the first: apt keeps its copy of InRelease
# when the server's copy is not newer to the second, so a machine that updated from the first
# site would not see the second. The script waits, polling, for the clock to pass the first
# one's second, and fails if the later InRelease is not newer.
#
# Exit codes: 0 written; 1 failed; 2 usage error.
set -euo pipefail
umask 022
# Whatever fails without a sentence of its own ends the script with 1, never with the failing
# tool's own code (gpg's 2 would read as a usage error).
set -E
trap 'echo "$NAME: line $LINENO: a command failed" >&2; exit 1' ERR
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

if [ ! -d "$OUTDIR/site" ] || [ ! -d "$OUTDIR/release" ]; then fail "$OUTDIR holds no site/ and release/ of assemble.sh"; fi
OUTDIR=$(cd "$OUTDIR" && pwd)
if [ -e "$OUTDIR/site-later" ] || [ -L "$OUTDIR/site-later" ]; then fail "$OUTDIR/site-later exists already"; fi
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

# apt asks for InRelease with If-Modified-Since the time of the copy it holds, to the second,
# and keeps its copy on a 304. A second site signed in the very second of the first would not
# replace the index of a machine that updated from the first, so wait, polling, until the
# clock has passed the second of the first one's InRelease.
FIRST_SIGNED=$(stat -c %Y "$OUTDIR/site/apt/dists/stable/InRelease") || fail "cannot read the time of $OUTDIR/site/apt/dists/stable/InRelease"
DEADLINE=$(($(date +%s) + 10))
while [ "$(date +%s)" -le "$FIRST_SIGNED" ]; do
  [ "$(date +%s)" -lt "$DEADLINE" ] || fail "the clock stays at the second $(date +%s) of the first InRelease, $FIRST_SIGNED"
  sleep 0.2
done

cp -a "$OUTDIR/site" "$OUTDIR/site-later"
MADE=1
"$ROOT/packaging/apt/make-apt-repo.sh" "$OUTDIR/site-later/apt" "${REPACKED[@]}"

[ "$(stat -c %Y "$OUTDIR/site-later/apt/dists/stable/InRelease")" -gt "$FIRST_SIGNED" ] \
  || fail "$OUTDIR/site-later/apt/dists/stable/InRelease is not newer than the first site's, by the second"

# The first version keeps its bytes, and both versions are in the pool.
while IFS= read -r deb; do
  name=$(basename "$deb")
  pooled=$(find "$OUTDIR/site-later/apt/pool" -type f -name "$name")
  [ -n "$pooled" ] || fail "$name is not in the pool of $OUTDIR/site-later/apt"
  cmp -s "$deb" "$pooled" || fail "$pooled is not the .deb of $OUTDIR/release/$name: a published version must keep its bytes"
done <<< "$DEBS"
# The later version: each repacked file is in the pool under its name with the bytes it was
# made with, and holds the later version (a repack named for the old version would meet the
# first version's file under the same name, which a name alone would take for its own). The
# pool then has a file of the later version for every processor of the release.
for later in "${REPACKED[@]}"; do
  name=$(basename "$later")
  pooled=$(find "$OUTDIR/site-later/apt/pool" -type f -name "$name")
  [ -n "$pooled" ] || fail "$name is not in the pool of $OUTDIR/site-later/apt"
  have=$(dpkg-deb -f "$pooled" Version)
  [ "$have" = "$LATER" ] || fail "$pooled holds version $have, not the later version $LATER"
  cmp -s "$later" "$pooled" || fail "$pooled is not the repacked $name"
done
# The list of the pool is read in an assignment, where a failure of find stops the script, and
# each field in an assignment of its own, never inside a test.
POOLED_ALL=$(find "$OUTDIR/site-later/apt/pool" -type f -name '*.deb') \
  || fail "cannot list the .deb files under $OUTDIR/site-later/apt/pool"
while IFS= read -r deb; do
  arch=$(dpkg-deb -f "$deb" Architecture) || fail "$deb is not a .deb that dpkg-deb can read"
  found=0
  while IFS= read -r pooled; do
    [ -n "$pooled" ] || continue
    pooled_version=$(dpkg-deb -f "$pooled" Version) || fail "$pooled is not a .deb that dpkg-deb can read"
    pooled_arch=$(dpkg-deb -f "$pooled" Architecture) || fail "$pooled is not a .deb that dpkg-deb can read"
    if [ "$pooled_version" = "$LATER" ] && [ "$pooled_arch" = "$arch" ]; then found=1; fi
  done <<< "$POOLED_ALL"
  [ "$found" -eq 1 ] || fail "the pool of $OUTDIR/site-later/apt holds no $arch package of version $LATER"
done <<< "$DEBS"
echo "$NAME: wrote $OUTDIR/site-later: the pool holds both versions of each processor, $LATER the later"
