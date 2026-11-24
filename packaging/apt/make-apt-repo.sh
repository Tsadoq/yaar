#!/bin/bash
# Builds or refreshes a signed apt repository made of static files only.
#
#   GNUPGHOME=<dir> YAAR_APT_KEY=<fingerprint> make-apt-repo.sh <repo-dir> [<deb>...]
#
# Each .deb named is copied into pool/main/<first letter>/<package>/ (a file of the same
# name that is already there is never overwritten); then the index is rebuilt from
# everything in pool/ and signed again. Give the .deb of both processors to publish both.
# Refuses, naming the file, when apt-ftparchive complains of a file of the pool (it would
# otherwise leave it out of the index and exit 0) or when a .deb of the pool is not in the
# Packages file of its processor.
#
# Result, all under <repo-dir>:
#   pool/main/y/yaar-review/*.deb
#   dists/stable/main/binary-<arch>/Packages, Packages.gz    one folder per architecture
#                                                            found in pool/
#   dists/stable/Release       hashes of the Packages files
#   dists/stable/Release.gpg   detached signature of Release
#   dists/stable/InRelease     Release with the signature inline (the file apt asks for)
#   yaar-review.asc            the public key, armoured; install.sh fetches it from here
#
# Needs: apt-ftparchive (package apt-utils), dpkg-deb (dpkg), gpg (gnupg), gzip.
#
# Exit codes: 0 written; 1 refused or failed (a sentence says why; whatever fails without a
# sentence of its own, such as gzip or gpg, ends it with 1 and the line, never with the
# tool's own code, gpg's 2 reading as a usage error); 2 usage error (no <repo-dir>).
#
# Environment:
#   YAAR_APT_KEY   fingerprint or id of the signing key; its secret half is in GNUPGHOME
#   GNUPGHOME      the keyring folder gpg reads
#   SUITE          default stable
#   ORIGIN, LABEL  default YAAR
set -euo pipefail
umask 022
# Whatever fails without a sentence of its own ends the script with 1, never with the failing
# tool's own code (gpg's 2 would read as a usage error).
set -E
trap 'echo "make-apt-repo.sh: line $LINENO: a command failed" >&2; exit 1' ERR
[ $# -ge 1 ] || { sed -n '2,4p' "$0" >&2; exit 2; }
: "${YAAR_APT_KEY:?set YAAR_APT_KEY to the fingerprint of the signing key}"
: "${GNUPGHOME:?set GNUPGHOME to the folder that holds the signing key}"
export GNUPGHOME
REPO=$1; shift
SUITE=${SUITE:-stable}; COMP=main; ORIGIN=${ORIGIN:-YAAR}; LABEL=${LABEL:-YAAR}
gpg --batch --list-secret-keys "$YAAR_APT_KEY" > /dev/null 2>&1 \
  || { echo "make-apt-repo.sh: no secret key $YAAR_APT_KEY in $GNUPGHOME" >&2; exit 1; }

mkdir -p "$REPO/pool/$COMP"
for deb in "$@"; do
  [ -f "$deb" ] || { echo "make-apt-repo.sh: $deb is not a file" >&2; exit 1; }
  pkg=$(dpkg-deb -f "$deb" Package 2> /dev/null) \
    || { echo "make-apt-repo.sh: $deb is not a .deb that dpkg-deb can read" >&2; exit 1; }
  [ -n "$pkg" ] || { echo "make-apt-repo.sh: $deb has no Package field" >&2; exit 1; }
  d="$REPO/pool/$COMP/$(printf %.1s "$pkg")/$pkg"; mkdir -p "$d"
  [ -e "$d/$(basename "$deb")" ] || cp "$deb" "$d/"
done
cd "$REPO"
# A file in pool/ that dpkg-deb cannot read would be left out of the index without a word.
POOL_DEBS=$(find pool -name '*.deb') \
  || { echo "make-apt-repo.sh: cannot list the .deb files under $REPO/pool: a folder in it cannot be read" >&2; exit 1; }
while IFS= read -r deb; do
  [ -n "$deb" ] || continue
  dpkg-deb -f "$deb" Architecture > /dev/null 2>&1 \
    || { echo "make-apt-repo.sh: $REPO/$deb is not a .deb that dpkg-deb can read" >&2; exit 1; }
done <<< "$POOL_DEBS"
ARCHS=$(find pool -name '*.deb' -exec dpkg-deb -f {} Architecture \; | sort -u | grep -v '^all$' | tr '\n' ' ')
ARCHS=${ARCHS% }
[ -n "$ARCHS" ] || { echo "make-apt-repo.sh: no package in $REPO/pool" >&2; exit 1; }
# apt-ftparchive can refuse a file of the pool, say so on standard error ("E: Errors apply to
# file ...") and still exit 0, leaving the file out of the index. Its standard error goes to a
# file, and anything in it ends this script: a published version must stay installable by its
# number.
ERRS=$(mktemp -d "${TMPDIR:-/tmp}/yaar-apt-repo.XXXXXX")
trap 'rm -rf "$ERRS"' EXIT
ftparchive() {
  apt-ftparchive "$@" 2> "$ERRS/stderr" \
    || { echo "make-apt-repo.sh: apt-ftparchive failed: $(cat "$ERRS/stderr")" >&2; return 1; }
  if [ -s "$ERRS/stderr" ]; then
    echo "make-apt-repo.sh: apt-ftparchive refused part of $REPO/pool and would leave it out of the index: $(tr '\n' ' ' < "$ERRS/stderr")" >&2
    return 1
  fi
}
rm -rf "dists/$SUITE"
for a in $ARCHS; do
  b="dists/$SUITE/$COMP/binary-$a"; mkdir -p "$b"
  ftparchive --arch "$a" packages "pool/$COMP" > "$b/Packages"
  gzip -n -9 -k "$b/Packages"
done
# Every file of the pool is in the index of its processor, whatever the indexer said.
while IFS= read -r deb; do
  [ -n "$deb" ] || continue
  arch=$(dpkg-deb -f "$deb" Architecture)
  [ "$arch" != all ] || continue
  grep -qxF "Filename: $deb" "dists/$SUITE/$COMP/binary-$arch/Packages" \
    || { echo "make-apt-repo.sh: $REPO/$deb is in the pool but not in dists/$SUITE/$COMP/binary-$arch/Packages" >&2; exit 1; }
done <<< "$POOL_DEBS"
ftparchive \
  -o "APT::FTPArchive::Release::Origin=$ORIGIN" -o "APT::FTPArchive::Release::Label=$LABEL" \
  -o "APT::FTPArchive::Release::Suite=$SUITE" -o "APT::FTPArchive::Release::Codename=$SUITE" \
  -o "APT::FTPArchive::Release::Architectures=$ARCHS" -o "APT::FTPArchive::Release::Components=$COMP" \
  release "dists/$SUITE" > Release.tmp
mv Release.tmp "dists/$SUITE/Release"
gpg --batch --yes --local-user "$YAAR_APT_KEY" --digest-algo SHA512 --clearsign -o "dists/$SUITE/InRelease" "dists/$SUITE/Release"
gpg --batch --yes --local-user "$YAAR_APT_KEY" --digest-algo SHA512 --armor --detach-sign -o "dists/$SUITE/Release.gpg" "dists/$SUITE/Release"
gpg --batch --yes --armor --export "$YAAR_APT_KEY" > yaar-review.asc.tmp
[ -s yaar-review.asc.tmp ] || { echo "make-apt-repo.sh: could not export the public key" >&2; exit 1; }
mv yaar-review.asc.tmp yaar-review.asc
echo "repository in $REPO: suite $SUITE, component $COMP, architectures $ARCHS, signed by $YAAR_APT_KEY"
