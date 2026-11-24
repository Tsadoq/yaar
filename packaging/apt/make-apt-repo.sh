#!/bin/bash
# Builds or refreshes a signed apt repository of static files.
#
# GNUPGHOME=<dir> YAAR_APT_KEY=<fingerprint> make-apt-repo.sh <repo-dir> [<deb>...]
set -euo pipefail
umask 022
# Any failure ends with 1, never the tool's own code (gpg's 2 would read as a usage error).
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
# apt-ftparchive can print "E: Errors apply to file ..." and still exit 0, leaving the file out
# of the index. Its stderr goes to a file and anything in it ends the script.
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
