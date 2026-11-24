#!/bin/bash
# Verifies the signed apt index, the checksums and install.sh of an assembled release with only
# the key the site publishes. .github/scripts/verify-signed.sh <final-dir>
# Exit 1: a fault, each named on standard error.
set -euo pipefail

NAME=verify-signed.sh
FAILED=0
fault() {
  echo "$NAME: $*" >&2
  FAILED=1
}
if [ $# -ne 1 ]; then
  echo "usage: $NAME <final-dir>" >&2
  exit 2
fi
FINAL=$1
[ -d "$FINAL" ] || { echo "$NAME: $FINAL is not a folder" >&2; exit 1; }
for tool in gpg gpgv sha256sum sha1sum sha512sum md5sum awk wc find cmp gzip; do
  command -v "$tool" > /dev/null 2>&1 || { echo "$NAME: $tool is not installed" >&2; exit 1; }
done

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
TMP=$(mktemp -d "${TMPDIR:-/tmp}/yaar-verify-signed.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
# A keyring of its own: the job's GNUPGHOME holds the secret key.
GNUPGHOME=$TMP/gnupg
export GNUPGHOME
mkdir -m 700 "$GNUPGHOME"

SITE=$FINAL/site
APT=$SITE/apt
DISTS=$APT/dists/stable
REL=$FINAL/release

first_line() {
  awk '/^[ \t]*(#|$)/ { next } { print; exit }' "$1"
}
sha_of() { # algorithm file
  "$1sum" "$2" | awk '{ print $1 }'
}
size_of() {
  wc -c < "$1" | awk '{ print $1 }'
}

WANT=
FPR_FILE=$ROOT/packaging/apt/project-key.fingerprint
if [ ! -f "$FPR_FILE" ]; then
  fault "$FPR_FILE is missing"
else
  WANT=$(first_line "$FPR_FILE")
  case $WANT in
    '') fault "$FPR_FILE holds no fingerprint (only comments): a release is signed with a key whose fingerprint is on record"; WANT= ;;
    *[!0-9A-Fa-f]*) fault "$FPR_FILE: the fingerprint line is not forty hexadecimal digits: $WANT"; WANT= ;;
    *) if [ "${#WANT}" -ne 40 ]; then fault "$FPR_FILE: the fingerprint line is not forty hexadecimal digits: $WANT"; WANT=; fi ;;
  esac
  WANT=$(printf '%s' "$WANT" | tr 'a-f' 'A-F')
fi
BASE=
BASE_FILE=$ROOT/packaging/site/base-url
if [ ! -f "$BASE_FILE" ]; then
  fault "$BASE_FILE is missing"
else
  BASE=$(first_line "$BASE_FILE")
  case $BASE in
    *[[:space:]]* | */ | '') fault "$BASE_FILE: the base address is not a https:// address with no '/' at its end: ${BASE:-nothing}"; BASE= ;;
    https://?*) ;;
    *) fault "$BASE_FILE: the base address is not a https:// address with no '/' at its end: $BASE"; BASE= ;;
  esac
fi
TEAM=
TEAM_FILE=$ROOT/packaging/macos/team-id
if [ ! -f "$TEAM_FILE" ]; then
  fault "$TEAM_FILE is missing"
else
  TEAM=$(first_line "$TEAM_FILE")
  if [ -z "$TEAM" ]; then
    fault "$TEAM_FILE holds no Team ID (only comments): a release's install script names the team that signs the app"
  else
    case $TEAM in
      *[!0-9A-Z]*) fault "$TEAM_FILE: a Team ID is ten capital letters or digits, not: $TEAM"; TEAM= ;;
      *) if [ "${#TEAM}" -ne 10 ]; then fault "$TEAM_FILE: a Team ID is ten capital letters or digits, not: $TEAM"; TEAM=; fi ;;
    esac
  fi
fi

KEYRING=$TMP/site-key.gpg
HAVE_KEYRING=0
ASC=$APT/yaar-review.asc
if [ ! -f "$ASC" ]; then
  fault "$ASC is missing: the site publishes no key"
elif ! KEYINFO=$(gpg --batch --no-tty --show-keys --with-colons "$ASC" 2> "$TMP/gpg.err"); then
  fault "$ASC is not an OpenPGP key gpg can read: $(head -n 1 "$TMP/gpg.err")"
else
  KEYS=$(printf '%s\n' "$KEYINFO" | awk -F: '$1 == "pub" { n++ } END { print n + 0 }')
  PUBFPR=$(printf '%s\n' "$KEYINFO" | awk -F: '$1 == "pub" { p = 1; next } p && $1 == "fpr" { print $10; exit }')
  if [ "$KEYS" -ne 1 ]; then
    fault "$ASC holds $KEYS keys: the site publishes the project's one key"
  elif [ -n "$WANT" ] && [ "$PUBFPR" != "$WANT" ]; then
    fault "the key at $ASC has the fingerprint $PUBFPR, not the $WANT of $FPR_FILE"
  elif gpg --batch --no-tty --dearmor < "$ASC" > "$KEYRING" 2> "$TMP/gpg.err"; then
    HAVE_KEYRING=1
  else
    fault "$ASC cannot be made into a keyring: $(head -n 1 "$TMP/gpg.err")"
  fi
fi

# verify_sig <what> <gpgv args>: the signature must verify and be by the project's key; a
# failure records a fault, and it returns 0 either way so that every check runs.
verify_sig() {
  local what=$1
  shift
  if ! gpgv --keyring "$KEYRING" --status-fd 3 "$@" 3> "$TMP/status" 2> "$TMP/gpgv.err"; then
    fault "$what does not verify with the key the site publishes: $(tr '\n' ' ' < "$TMP/gpgv.err")"
    return 0
  fi
  if [ -n "$WANT" ] && ! awk -v want="$WANT" '$1 == "[GNUPG:]" && $2 == "VALIDSIG" && toupper($NF) == want { ok = 1 } END { exit !ok }' "$TMP/status"; then
    fault "$what verifies, but not by the key of the fingerprint $WANT"
  fi
  return 0
}

INRELEASE_TEXT=
if [ "$HAVE_KEYRING" -eq 1 ]; then
  for f in "$DISTS/InRelease" "$DISTS/Release" "$DISTS/Release.gpg" "$REL/SHA256SUMS" "$REL/SHA256SUMS.asc"; do
    [ -f "$f" ] || fault "$f is missing"
  done
  if [ -f "$DISTS/InRelease" ]; then
    if gpgv --keyring "$KEYRING" --status-fd 3 --output "$TMP/InRelease.text" "$DISTS/InRelease" 3> "$TMP/status" 2> "$TMP/gpgv.err"; then
      INRELEASE_TEXT=$TMP/InRelease.text
      # gpgv accepts the signed block anywhere in the file; apt refuses other text around it.
      if ! awk '
        { line[NR] = $0 }
        $0 == "-----BEGIN PGP SIGNED MESSAGE-----" { begun++ }
        $0 == "-----BEGIN PGP SIGNATURE-----" { signed++ }
        $0 == "-----END PGP SIGNATURE-----" { ended++ }
        END {
          exit !(NR > 0 && line[1] == "-----BEGIN PGP SIGNED MESSAGE-----" && line[NR] == "-----END PGP SIGNATURE-----" && begun == 1 && signed == 1 && ended == 1)
        }' "$DISTS/InRelease"; then
        fault "$DISTS/InRelease is not one clear-signed block and nothing else (it must start with -----BEGIN PGP SIGNED MESSAGE----- and end with -----END PGP SIGNATURE-----, each once): apt refuses it though gpgv verifies it"
      fi
      if [ -n "$WANT" ] && ! awk -v want="$WANT" '$1 == "[GNUPG:]" && $2 == "VALIDSIG" && toupper($NF) == want { ok = 1 } END { exit !ok }' "$TMP/status"; then
        fault "$DISTS/InRelease verifies, but not by the key of the fingerprint $WANT"
      fi
    else
      fault "$DISTS/InRelease does not verify with the key the site publishes: $(tr '\n' ' ' < "$TMP/gpgv.err")"
    fi
  fi
  if [ -f "$DISTS/Release.gpg" ] && [ -f "$DISTS/Release" ]; then
    verify_sig "$DISTS/Release.gpg against $DISTS/Release" "$DISTS/Release.gpg" "$DISTS/Release"
  fi
  if [ -f "$REL/SHA256SUMS.asc" ] && [ -f "$REL/SHA256SUMS" ]; then
    verify_sig "$REL/SHA256SUMS.asc against $REL/SHA256SUMS" "$REL/SHA256SUMS.asc" "$REL/SHA256SUMS"
  fi
  if [ -n "$INRELEASE_TEXT" ] && [ -f "$DISTS/Release" ] && ! cmp -s "$INRELEASE_TEXT" "$DISTS/Release"; then
    fault "the signed text of $DISTS/InRelease is not the file $DISTS/Release beside it"
  fi
fi

# Hashes are read from the text gpgv returned, never from the file as it lies.
if [ -n "$INRELEASE_TEXT" ]; then
  awk '
    /^[^ \t]/ { sec = ""; if ($1 == "MD5Sum:") sec = "md5"; else if ($1 == "SHA1:") sec = "sha1"; else if ($1 == "SHA256:") sec = "sha256"; else if ($1 == "SHA512:") sec = "sha512"; next }
    sec != "" && NF == 3 { print sec, $1, $2, $3 }' "$INRELEASE_TEXT" > "$TMP/named"
  if ! awk '$1 == "sha256" { found = 1 } END { exit !found }' "$TMP/named"; then
    fault "$DISTS/InRelease names no SHA256 hash of any file"
  fi
  while read -r alg hash size path; do
    case $path in
      *..* | /*) fault "$DISTS/InRelease names the path $path"; continue ;;
    esac
    if [ ! -f "$DISTS/$path" ]; then
      fault "$DISTS/InRelease names $path ($alg), which is not there"
      continue
    fi
    if [ "$(size_of "$DISTS/$path")" != "$size" ]; then
      fault "$DISTS/$path has the size $(size_of "$DISTS/$path"), not the $size that InRelease names"
      continue
    fi
    if [ "$(sha_of "$alg" "$DISTS/$path")" != "$hash" ]; then
      fault "$DISTS/$path has not the $alg hash that InRelease names: it was changed after it was signed"
    fi
  done < "$TMP/named"
  ALL_PACKAGES=$(find "$DISTS" -type f -name 'Packages*' | LC_ALL=C sort) || { fault "the files under $DISTS cannot be listed"; ALL_PACKAGES=; }
  while IFS= read -r pf; do
    [ -n "$pf" ] || continue
    rel=${pf#"$DISTS"/}
    awk -v p="$rel" '$1 == "sha256" && $4 == p { found = 1 } END { exit !found }' "$TMP/named" \
      || fault "$pf is not named in the SHA256 section of $DISTS/InRelease"
  done <<< "$ALL_PACKAGES"
fi

NPACK=0
: > "$TMP/indexed"
PACKAGES_FILES=$(find "$DISTS" -type f -name Packages 2> /dev/null | LC_ALL=C sort) || { fault "the files under $DISTS cannot be listed"; PACKAGES_FILES=; }
while IFS= read -r pf; do
  [ -n "$pf" ] || continue
  NPACK=$((NPACK + 1))
  awk 'BEGIN { RS = ""; FS = "\n" }
    { fn = ""; sz = ""; sh = ""
      for (i = 1; i <= NF; i++) {
        if ($i ~ /^Filename: /) fn = substr($i, 11)
        else if ($i ~ /^Size: /) sz = substr($i, 7)
        else if ($i ~ /^SHA256: /) sh = substr($i, 9)
      }
      print fn "|" sz "|" sh }' "$pf" > "$TMP/stanzas"
  [ -s "$TMP/stanzas" ] || fault "$pf names no package"
  while IFS='|' read -r fn sz sh; do
    case $fn in
      pool/*) ;;
      *) fault "$pf names a package whose Filename is not under pool/: ${fn:-nothing}"; continue ;;
    esac
    case $fn in *..*) fault "$pf names the path $fn"; continue ;; esac
    if [ -z "$sz" ] || [ -z "$sh" ]; then
      fault "$pf: the stanza of $fn has no Size or no SHA256"
      continue
    fi
    printf '%s\n' "$fn" >> "$TMP/indexed"
    if [ ! -f "$APT/$fn" ]; then
      fault "$pf names $fn, which is not in the pool"
    elif [ "$(size_of "$APT/$fn")" != "$sz" ]; then
      fault "$APT/$fn has the size $(size_of "$APT/$fn"), not the $sz that $pf names"
    elif [ "$(sha_of sha256 "$APT/$fn")" != "$sh" ]; then
      fault "$APT/$fn has not the SHA-256 that $pf names"
    fi
  done < "$TMP/stanzas"
done <<< "$PACKAGES_FILES"
[ "$NPACK" -gt 0 ] || fault "$DISTS holds no Packages file"
if [ -d "$APT/pool" ]; then
  POOL_DEBS=$(find "$APT/pool" -type f -name '*.deb' | LC_ALL=C sort) || { fault "the files under $APT/pool cannot be listed"; POOL_DEBS=; }
  while IFS= read -r deb; do
    [ -n "$deb" ] || continue
    rel=${deb#"$APT"/}
    grep -qxF "$rel" "$TMP/indexed" || fault "$deb is in the pool but no Packages file names it: it cannot be installed"
  done <<< "$POOL_DEBS"
else
  fault "$APT/pool is missing"
fi
if [ -d "$REL" ]; then
  for deb in "$REL"/*.deb; do
    [ -e "$deb" ] || continue
    name=$(basename "$deb")
    found=$(find "$APT/pool" -type f -name "$name" | awk 'NR == 1 { print }') || { fault "the files under $APT/pool cannot be listed"; found=$deb; }
    if [ -z "$found" ]; then
      fault "$deb is not in the pool of $APT"
    elif ! cmp -s "$deb" "$found"; then
      fault "$found is not the file $deb of the release: a package has two sets of bytes"
    fi
  done
fi

INSTALL=$SITE/install.sh
if [ ! -f "$INSTALL" ]; then
  fault "$INSTALL is missing"
else
  for ph in '@BASE_URL@' '@TEAM_ID@'; do
    n=$(awk -v ph="$ph" 'index($0, ph) { c++ } END { print c + 0 }' "$INSTALL") || { fault "$INSTALL cannot be read"; n=0; }
    [ "$n" = 0 ] || fault "$INSTALL still holds the placeholder $ph, on $n line(s): the script is not one a user may run"
  done
  assigned() { # variable, wanted line
    awk -v var="$1" -v want="$2" '
      index($0, var "=") == 1 { n++; if ($0 == want) good++ }
      END { printf "%d %d\n", n + 0, good + 0 }' "$INSTALL"
  }
  if [ -n "$BASE" ]; then
    read -r n good <<< "$(assigned DEFAULT_BASE_URL "DEFAULT_BASE_URL=\"$BASE\"")"
    if [ "$n" -ne 1 ] || [ "$good" -ne 1 ]; then
      fault "$INSTALL does not have exactly one line DEFAULT_BASE_URL=\"$BASE\", the line of $BASE_FILE (it has $n assignment(s), $good right)"
    fi
  fi
  if [ -n "$TEAM" ]; then
    read -r n good <<< "$(assigned TEAM_ID "TEAM_ID=\"$TEAM\"")"
    if [ "$n" -ne 1 ] || [ "$good" -ne 1 ]; then
      fault "$INSTALL does not have exactly one line TEAM_ID=\"$TEAM\", the line of $TEAM_FILE (it has $n assignment(s), $good right)"
    fi
  fi
fi
if [ ! -f "$SITE/index.html" ]; then
  fault "$SITE/index.html is missing"
elif LEFT=$(grep -o -m1 '@[A-Z_][A-Z_]*@' "$SITE/index.html"); then
  fault "$SITE/index.html still holds the placeholder $LEFT"
fi

[ "$FAILED" -eq 0 ] || exit 1
echo "$NAME: the index, the checksums and the install script of $FINAL verify with the key the site publishes ($WANT)"
