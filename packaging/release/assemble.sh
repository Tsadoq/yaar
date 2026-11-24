#!/bin/bash
# Makes a release as files: the release files with one checksum file, the Pages site with the
# install script and the apt repository. The rehearsal runs it with a throwaway key (job
# assemble), a release with the project's key (job assemble-release): the same script both
# times.
#
#   GNUPGHOME=<dir> YAAR_APT_KEY=<fingerprint> [PREVIOUS=<dir>] \
#     packaging/release/assemble.sh <version> <packages-dir> <out-dir>
#
# <packages-dir> holds the six packages of <version> (the two .deb, the two archives, the
# .zip and the .dmg). PREVIOUS, when set, is a folder of the .deb files of earlier published
# releases (fetch-published-debs.sh), which go into the apt repository's pool beside the new
# ones; unset or empty, the pool holds only this version's.
#
# Writes <out-dir>, which must not exist:
#   release/   the six files, named exactly:
#                yaar-review_<version>_amd64.deb      yaar-review_<version>_arm64.deb
#                yaar-review-<version>-linux-x86_64.tar.gz
#                yaar-review-<version>-linux-aarch64.tar.gz
#                yaar-review-<version>-macos-universal.zip
#                yaar-review-<version>-macos-universal.dmg
#              SHA256SUMS   one line per file, "<sha256>  <name>", sorted by name, these six
#                           and nothing else
#              SHA256SUMS.asc   a detached armoured signature of SHA256SUMS by YAAR_APT_KEY
#   site/      what the Pages site serves, and nothing else:
#              index.html   from packaging/site/index.html.in, with @BASE_URL@ and @VERSION@
#                           replaced: what the app is, the install line, the manual routes
#                           (the .dmg, the .deb, the apt source), the uninstall line; no
#                           script, no style or font from another host, no analytics
#              install.sh   packaging/install.sh with its line DEFAULT_BASE_URL="@BASE_URL@"
#                           replaced by the one value of packaging/site/base-url, and its line
#                           TEAM_ID="@TEAM_ID@" replaced by the one line of
#                           packaging/macos/team-id that is not a comment; while that file
#                           holds no such line (before the owner wrote it, and in every
#                           rehearsal until then) @TEAM_ID@ stays, which install.sh accepts
#                           only from a loopback address (verify-signed.sh refuses it in a
#                           release)
#              apt/         packaging/apt/make-apt-repo.sh's repository of the new .deb files
#                           and those of PREVIOUS: pool/, dists/stable/{InRelease,Release,
#                           Release.gpg,main/binary-amd64/,main/binary-arm64/}, and
#                           yaar-review.asc, the public key
#              .nojekyll    empty, so that Pages serves the tree as it is
#   key.asc    the public key of YAAR_APT_KEY, beside the two folders (for the tests)
#
# The base address is read from packaging/site/base-url and nowhere else. Refuses, changing
# nothing, when a package is missing, when a file of PREVIOUS has the name of a new one, or
# when a .deb's Version or Architecture does not match its name. Refuses, after writing, when
# site/ holds more than 900 MB (`du -sk`): a Pages site may hold 1 GB, and the pool grows by
# two .deb files a release; the sentence names the versions in the pool, oldest first, and
# says that the oldest must leave the pool (a later decision of the owner's: they stay on the
# releases page).
#
# Exit codes: 0 written; 1 refused or failed (a sentence says why); 2 usage error.
#
# Needs: apt-ftparchive (package apt-utils), dpkg-deb (dpkg), gpg (gnupg), gzip, sha256sum,
# du with --apparent-size (GNU coreutils: the size of a file is its length, not the blocks it
# holds, which a sparse file of a test has none of).
#
# Beyond the header's list, it also refuses, changing nothing, when YAAR_APT_KEY or GNUPGHOME
# is unset (a usage error, exit 2), when GNUPGHOME holds no secret key YAAR_APT_KEY, when
# <out-dir> exists, when packaging/install.sh lacks its line DEFAULT_BASE_URL="@BASE_URL@"
# (or its line TEAM_ID="@TEAM_ID@" while packaging/macos/team-id has a line), and when the
# base address or the team id is not what the file's one line should be (an address of
# https:// and a host, with no '/' at its end; ten capital letters or digits). index.html also takes
# the key's fingerprint, grouped by four, in place of @FINGERPRINT@, as the key at
# apt/yaar-review.asc gives it.
# After the repository is made it refuses when a .deb given to it (the two new ones and every one
# of PREVIOUS) is not in the pool with its own bytes, or is not in the Packages file of its
# processor with its Version.
set -euo pipefail
umask 022
# Whatever fails without a sentence of its own ends the script with 1, never with the failing
# tool's own code (gpg's 2 would read as a usage error).
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
for tool in apt-ftparchive dpkg-deb gpg gzip sha256sum du awk; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done

# The first line of a file that is not blank and not a comment: how the one value of
# packaging/site/base-url and of packaging/macos/team-id is read, never typed here.
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

# The six packages, by the names of the header.
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

# Each .deb holds what its name says.
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

# The .deb files of earlier releases. The list is read where a failure of find stops the
# script: a folder of PREVIOUS that cannot be read must not drop a published version from the
# repository without a word.
PREVIOUS_DEBS=()
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
    PREVIOUS_DEBS+=("$old")
  done <<< "$PREVIOUS_LIST"
fi

gpg --batch --list-secret-keys "$YAAR_APT_KEY" > /dev/null 2>&1 \
  || fail "no secret key $YAAR_APT_KEY in $GNUPGHOME"
if [ -e "$OUT" ] || [ -L "$OUT" ]; then fail "$OUT exists already: assemble into a folder that is not there"; fi

# Nothing is changed before this line. From here a failure takes back what was written,
# but for the size refusal, whose files stay for the owner to look at.
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

# release/: the six packages, SHA256SUMS sorted by name, its signature.
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

# site/: the apt repository first, so that the page can name the key the repository serves.
mkdir "$OUT/site"
# (the expansion of an empty array is spelled so that bash 3.2 under set -u accepts it)
"$ROOT/packaging/apt/make-apt-repo.sh" "$OUT/site/apt" "$PKGDIR/$DEB_AMD64" "$PKGDIR/$DEB_ARM64" \
  ${PREVIOUS_DEBS[@]+"${PREVIOUS_DEBS[@]}"}
# Every .deb given to the repository, the two new ones and those of PREVIOUS, is in the pool
# with its own bytes and in the index of its processor with its Version, whatever the folder it
# came from or the tool that indexed it did: an earlier release that dropped out here would
# stay out of the repository with every other step green.
# The list is the folder's own listing, not the array the repository was given: a loop that
# dropped a file from the array would otherwise drop it from this check too.
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

# Replaces every occurrence of one literal string by another (no pattern, no '&' or
# backslash trouble): replace <file> <from> <to>.
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
# A placeholder the replacements above missed would be published as it stands.
if LEFT=$(grep -o -m1 '@[A-Z_][A-Z_]*@' "$SCRATCH/index.html"); then
  fail "$PAGE_IN still holds the placeholder $LEFT after the replacements: the page must not go out with it"
fi
cp "$SCRATCH/index.html" "$OUT/site/index.html"

# install.sh: its two lines, whole, and nothing else of it.
awk -v base="$BASE_URL" -v team="$TEAM_ID" -v basere="$KEY_LINE" -v teamre="$TEAM_LINE" '
  $0 ~ basere { match($0, /^[ \t]*/); print substr($0, 1, RLENGTH) "DEFAULT_BASE_URL=\"" base "\""; next }
  team != "" && $0 ~ teamre { match($0, /^[ \t]*/); print substr($0, 1, RLENGTH) "TEAM_ID=\"" team "\""; next }
  { print }' "$INSTALL_IN" > "$OUT/site/install.sh"
chmod 0755 "$OUT/site/install.sh"
: > "$OUT/site/.nojekyll"

# A Pages site may hold 1 GB; the pool grows by two .deb files a release.
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
