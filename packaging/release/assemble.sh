#!/bin/bash
# Makes a release as files: the release files with one checksum file, and the Pages site with the
# install script and the apt repository. A rehearsal and a release run the same script, with a
# throwaway key or the project's. A refusal leaves nothing written, except the size check (site/
# over 900 MB), whose files stay. PREVIOUS: earlier releases' .deb files for the apt pool.
#
# GNUPGHOME=<dir> YAAR_APT_KEY=<fingerprint> [PREVIOUS=<dir>] \
# packaging/release/assemble.sh <version> <packages-dir> <out-dir>
set -euo pipefail
umask 022
# Any failure ends with 1, never the tool's own code (gpg's 2 would read as a usage error).
set -E
trap 'echo "$NAME: line $LINENO: a command failed" >&2; exit 1' ERR
NAME=assemble.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
if [ $# -ne 3 ]; then
  sed -n '7,8p' "$0" >&2
  exit 2
fi
VERSION=$1
PKGDIR=$2
OUT=$3
case $VERSION in
  '' | [!0-9]* | *[!0-9A-Za-z.+~]*)
    echo "$NAME: a version starts with a digit and holds only letters, digits, '.', '+' and '~', not: $VERSION" >&2
    exit 2
    ;;
esac
if [ -z "${YAAR_APT_KEY:-}" ] || [ -z "${GNUPGHOME:-}" ]; then
  echo "$NAME: set YAAR_APT_KEY to the fingerprint of the signing key and GNUPGHOME to the folder that holds it" >&2
  exit 2
fi
export GNUPGHOME

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
for tool in apt-ftparchive dpkg-deb dpkg gpg gzip sha256sum du awk; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done

# Reads the one value of packaging/site/base-url and of packaging/macos/team-id.
first_line() {
  awk '/^[ \t]*(#|$)/ { next } { print; exit }' "$1"
}

BASE_FILE=$ROOT/packaging/site/base-url
[ -f "$BASE_FILE" ] || fail "$BASE_FILE is missing"
BASE_URL=$(first_line "$BASE_FILE")
case $BASE_URL in
  *[[:space:]]*) fail "$BASE_FILE: the base address holds a space: $BASE_URL" ;;
  */) fail "$BASE_FILE: the base address must not end in '/': $BASE_URL" ;;
  https://?*) ;;
  *) fail "$BASE_FILE: the base address must be https:// and a host, not: ${BASE_URL:-nothing}" ;;
esac

TEAM_FILE=$ROOT/packaging/macos/team-id
TEAM_ID=
if [ -f "$TEAM_FILE" ]; then
  TEAM_ID=$(first_line "$TEAM_FILE")
  if [ -n "$TEAM_ID" ]; then
    case $TEAM_ID in
      *[!0-9A-Z]*) fail "$TEAM_FILE: a Team ID is ten capital letters or digits, not: $TEAM_ID" ;;
    esac
    [ "${#TEAM_ID}" -eq 10 ] || fail "$TEAM_FILE: a Team ID is ten capital letters or digits, not: $TEAM_ID"
  fi
fi

INSTALL_IN=$ROOT/packaging/install.sh
PAGE_IN=$ROOT/packaging/site/index.html.in
KEY_LINE='^[ \t]*DEFAULT_BASE_URL="@BASE_URL@"[ \t]*$'
TEAM_LINE='^[ \t]*TEAM_ID="@TEAM_ID@"[ \t]*$'
[ -f "$INSTALL_IN" ] || fail "$INSTALL_IN is missing"
[ -f "$PAGE_IN" ] || fail "$PAGE_IN is missing"
[ "$(awk -v re="$KEY_LINE" '$0 ~ re { n++ } END { print n + 0 }' "$INSTALL_IN")" -eq 1 ] \
  || fail "$INSTALL_IN has not exactly one line DEFAULT_BASE_URL=\"@BASE_URL@\" to replace"
if [ -n "$TEAM_ID" ]; then
  [ "$(awk -v re="$TEAM_LINE" '$0 ~ re { n++ } END { print n + 0 }' "$INSTALL_IN")" -eq 1 ] \
    || fail "$INSTALL_IN has not exactly one line TEAM_ID=\"@TEAM_ID@\" to replace, and $TEAM_FILE names a team"
fi

DEB_AMD64=yaar-review_${VERSION}_amd64.deb
DEB_ARM64=yaar-review_${VERSION}_arm64.deb
PACKAGES="$DEB_AMD64
$DEB_ARM64
yaar-review-${VERSION}-linux-x86_64.tar.gz
yaar-review-${VERSION}-linux-aarch64.tar.gz
yaar-review-${VERSION}-macos-universal.zip
yaar-review-${VERSION}-macos-universal.dmg"
[ -d "$PKGDIR" ] || fail "$PKGDIR is not a folder"
while IFS= read -r f; do
  [ -f "$PKGDIR/$f" ] || fail "$PKGDIR/$f is missing: a release holds the six packages of $VERSION"
done <<< "$PACKAGES"

for pair in "$DEB_AMD64 amd64" "$DEB_ARM64 arm64"; do
  deb=${pair% *}
  arch=${pair#* }
  have_pkg=$(dpkg-deb -f "$PKGDIR/$deb" Package 2> /dev/null) || fail "$PKGDIR/$deb is not a .deb that dpkg-deb can read"
  have_version=$(dpkg-deb -f "$PKGDIR/$deb" Version)
  have_arch=$(dpkg-deb -f "$PKGDIR/$deb" Architecture)
  [ "$have_pkg" = yaar-review ] || fail "$PKGDIR/$deb is the package $have_pkg, not yaar-review"
  [ "$have_version" = "$VERSION" ] || fail "$PKGDIR/$deb is named for version $VERSION but holds version $have_version"
  [ "$have_arch" = "$arch" ] || fail "$PKGDIR/$deb is named for $arch but holds a package for $have_arch"
done

# The .deb files of earlier releases. PREVIOUS is listed where a failure of find stops the
# script: an unreadable folder must not silently drop a published version.
PREVIOUS_DEBS=()
# Three parallel arrays: bash 3.2 has no maps.
SEEN_ARCH=()
SEEN_VERSION=()
SEEN_PATH=()
PREVIOUS_LIST=
if [ -n "${PREVIOUS:-}" ]; then
  [ -d "$PREVIOUS" ] || fail "PREVIOUS names $PREVIOUS, which is not a folder"
  PREVIOUS_LIST=$(find "$PREVIOUS" -type f -name '*.deb' | LC_ALL=C sort) \
    || fail "cannot list the .deb files under PREVIOUS, $PREVIOUS: a folder in it cannot be read"
  while IFS= read -r old; do
    [ -n "$old" ] || continue
    old_name=$(basename "$old")
    case $old_name in
      "$DEB_AMD64" | "$DEB_ARM64") fail "$old has the name of a new package: version $VERSION is published already" ;;
    esac
    # A PREVIOUS file must be what its name says, or the index would hold two stanzas for one
    # version or drop one.
    old_pkg=$(dpkg-deb -f "$old" Package 2> /dev/null) || fail "$old is not a .deb that dpkg-deb can read"
    old_version=$(dpkg-deb -f "$old" Version) || fail "$old has no Version that dpkg-deb can read"
    old_arch=$(dpkg-deb -f "$old" Architecture) || fail "$old has no Architecture that dpkg-deb can read"
    [ "$old_pkg" = yaar-review ] \
      || fail "$old is the package $old_pkg, not yaar-review: PREVIOUS holds only earlier releases of yaar-review"
    [ "$old_name" = "${old_pkg}_${old_version}_${old_arch}.deb" ] \
      || fail "$old is named $old_name but holds $old_pkg version $old_version for $old_arch, so its name should be ${old_pkg}_${old_version}_${old_arch}.deb"
    if dpkg --compare-versions "$old_version" eq "$VERSION"; then
      fail "$old holds version $old_version, which apt takes for $VERSION, the version being released, under the name $old_name: version $VERSION is published already"
    fi
    # Two spellings of one version (0.0.9, 0.0.09) would put both in the index and apt would pick
    # one.
    for ((j = 0; j < ${#SEEN_PATH[@]}; j++)); do
      if [ "${SEEN_ARCH[j]}" = "$old_arch" ] && dpkg --compare-versions "$old_version" eq "${SEEN_VERSION[j]}"; then
        fail "$old holds version $old_version for $old_arch, which apt takes for version ${SEEN_VERSION[j]} of ${SEEN_PATH[j]}, also in PREVIOUS: two files for one version"
      fi
    done
    SEEN_ARCH+=("$old_arch")
    SEEN_VERSION+=("$old_version")
    SEEN_PATH+=("$old")
    PREVIOUS_DEBS+=("$old")
  done <<< "$PREVIOUS_LIST"
fi

gpg --batch --list-secret-keys "$YAAR_APT_KEY" > /dev/null 2>&1 \
  || fail "no secret key $YAAR_APT_KEY in $GNUPGHOME"
if [ -e "$OUT" ] || [ -L "$OUT" ]; then fail "$OUT exists already: assemble into a folder that is not there"; fi

# Nothing is changed before this line. From here a failure takes back what was written, except for
# the size refusal, whose files stay to be looked at.
CLEAN=1
SCRATCH=
cleanup() {
  status=$?
  [ -z "$SCRATCH" ] || rm -rf "$SCRATCH"
  if [ "$status" -ne 0 ] && [ "$CLEAN" -eq 1 ]; then rm -rf "$OUT"; fi
}
trap cleanup EXIT
SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-assemble.XXXXXX")
mkdir "$OUT"
OUT=$(cd "$OUT" && pwd)

mkdir "$OUT/release"
while IFS= read -r f; do
  cp "$PKGDIR/$f" "$OUT/release/$f"
done <<< "$PACKAGES"
(
  cd "$OUT/release"
  LC_ALL=C sort <<< "$PACKAGES" | while IFS= read -r f; do sha256sum "$f"; done > SHA256SUMS
)
gpg --batch --yes --local-user "$YAAR_APT_KEY" --digest-algo SHA512 --armor --detach-sign \
  -o "$OUT/release/SHA256SUMS.asc" "$OUT/release/SHA256SUMS"

# The apt repository first, so the page can name the key it serves.
mkdir "$OUT/site"
# (an empty array is expanded this way so bash 3.2 accepts it under set -u)
"$ROOT/packaging/apt/make-apt-repo.sh" "$OUT/site/apt" "$PKGDIR/$DEB_AMD64" "$PKGDIR/$DEB_ARM64" \
  ${PREVIOUS_DEBS[@]+"${PREVIOUS_DEBS[@]}"}
# Checks that every given .deb is in the pool with its own bytes and in its processor's index.
# The list is PREVIOUS's own listing, not the array given to the repository, so a file the array
# lost is still checked.
INDEXED_LIST="$PKGDIR/$DEB_AMD64
$PKGDIR/$DEB_ARM64
$PREVIOUS_LIST"
while IFS= read -r deb; do
  [ -n "$deb" ] || continue
  deb_name=$(basename "$deb")
  deb_pkg=$(dpkg-deb -f "$deb" Package) || fail "$deb is not a .deb that dpkg-deb can read"
  deb_version=$(dpkg-deb -f "$deb" Version) || fail "$deb has no Version that dpkg-deb can read"
  deb_arch=$(dpkg-deb -f "$deb" Architecture) || fail "$deb has no Architecture that dpkg-deb can read"
  deb_path=pool/main/$(printf %.1s "$deb_pkg")/$deb_pkg/$deb_name
  [ -f "$OUT/site/apt/$deb_path" ] || fail "$deb_name (version $deb_version, $deb_arch) is not in the pool of $OUT/site/apt: $deb_path is missing"
  cmp -s "$deb" "$OUT/site/apt/$deb_path" \
    || fail "$OUT/site/apt/$deb_path is not the file $deb: a published version must keep its bytes"
  awk -v fn="$deb_path" -v ver="$deb_version" -v arch="$deb_arch" '
    BEGIN { RS = ""; FS = "\n" }
    { f = 0; v = 0; a = 0
      for (i = 1; i <= NF; i++) {
        if ($i == "Filename: " fn) f = 1
        if ($i == "Version: " ver) v = 1
        if ($i == "Architecture: " arch) a = 1
      }
      if (f && v && a) found = 1 }
    END { exit !found }' "$OUT/site/apt/dists/stable/main/binary-$deb_arch/Packages" \
    || fail "$deb_name (version $deb_version, $deb_arch) is in the pool but not in dists/stable/main/binary-$deb_arch/Packages of $OUT/site/apt"
done <<< "$INDEXED_LIST"
cp "$OUT/site/apt/yaar-review.asc" "$OUT/key.asc"
FPR=$(gpg --batch --show-keys --with-colons "$OUT/key.asc" | awk -F: '$1 == "fpr" { print $10; exit }')
[ -n "$FPR" ] || fail "no fingerprint in the public key $OUT/site/apt/yaar-review.asc"
FPR_GROUPED=$(printf '%s' "$FPR" | awk '{ s = ""; for (i = 1; i <= length($0); i += 4) s = s (i > 1 ? " " : "") substr($0, i, 4); print s }')

replace() {
  FROM=$2 TO=$3 awk '
    BEGIN { from = ENVIRON["FROM"]; to = ENVIRON["TO"] }
    { s = $0; out = ""
      while ((i = index(s, from)) > 0) { out = out substr(s, 1, i - 1) to; s = substr(s, i + length(from)) }
      print out s }' "$1" > "$1.new"
  mv "$1.new" "$1"
}
cp "$PAGE_IN" "$SCRATCH/index.html"
replace "$SCRATCH/index.html" '@BASE_URL@' "$BASE_URL"
replace "$SCRATCH/index.html" '@VERSION@' "$VERSION"
replace "$SCRATCH/index.html" '@FINGERPRINT@' "$FPR_GROUPED"
# A placeholder the replacements missed would be published as it stands.
if LEFT=$(grep -o -m1 '@[A-Z_][A-Z_]*@' "$SCRATCH/index.html"); then
  fail "$PAGE_IN still holds the placeholder $LEFT after the replacements: the page must not go out with it"
fi
cp "$SCRATCH/index.html" "$OUT/site/index.html"

awk -v base="$BASE_URL" -v team="$TEAM_ID" -v basere="$KEY_LINE" -v teamre="$TEAM_LINE" '
  $0 ~ basere { match($0, /^[ \t]*/); print substr($0, 1, RLENGTH) "DEFAULT_BASE_URL=\"" base "\""; next }
  team != "" && $0 ~ teamre { match($0, /^[ \t]*/); print substr($0, 1, RLENGTH) "TEAM_ID=\"" team "\""; next }
  { print }' "$INSTALL_IN" > "$OUT/site/install.sh"
chmod 0755 "$OUT/site/install.sh"
: > "$OUT/site/.nojekyll"

SIZE_KB=$(du -sk --apparent-size "$OUT/site" | awk '{ print $1 }')
case $SIZE_KB in
  '' | *[!0-9]*) fail "could not measure the size of $OUT/site" ;;
esac
if [ "$SIZE_KB" -gt $((900 * 1024)) ]; then
  CLEAN=0
  VERSIONS=$(find "$OUT/site/apt/pool" -name '*.deb' -exec dpkg-deb -f {} Version \; | LC_ALL=C sort -uV | paste -sd, - | sed 's/,/, /g')
  echo "$NAME: $OUT/site holds $((SIZE_KB / 1024)) MB, over the 900 MB it may hold (a Pages site may hold 1 GB); its pool holds the versions $VERSIONS, oldest first: the oldest must leave the pool before this release can go out (it stays on the releases page)" >&2
  exit 1
fi
echo "$NAME: wrote $OUT: release/ (6 packages, SHA256SUMS, SHA256SUMS.asc), site/ ($((SIZE_KB / 1024)) MB), key.asc, signed by $FPR"
