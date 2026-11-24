#!/bin/bash
# The .deb files of the releases already published, for the pool of the apt repository, so
# that a new release adds its packages beside the older ones.
#
#   GH_TOKEN=<token> .github/scripts/fetch-published-debs.sh <out-dir>
#
# Lists the published releases of this repository (`gh release list --exclude-drafts
# --json tagName`); for each, downloads its *.deb and its SHA256SUMS into a folder of its own
# (`gh release download <tag> --pattern '*.deb' --pattern SHA256SUMS`), checks each .deb
# against that SHA256SUMS (refuses a mismatch), and moves the .deb files into <out-dir>.
# None published: <out-dir> is made empty. Reaches GitHub's own addresses only.
#
# Exit codes: 0 done; 1 a mismatch or a failed download; 2 usage error.
#
# Beyond the header's list: GH_TOKEN and GITHUB_REPOSITORY (the repository, Owner/Name, which
# the runner sets) must be set, else it refuses before any gh call, exit 1. The list asks for
# 1000 releases (gh's own default is 30, and a published version that dropped out of the list
# would drop out of the apt repository). A list that fails is a failure, never "none
# published". A release tag must be letters, digits, '.', '_', '+', '~' and '-', and start
# with none of '-' or '.'. A .deb with no line of its own in its release's SHA256SUMS, a
# release with .deb files and no SHA256SUMS, and one .deb name in two releases are refused.
# Nothing is put into <out-dir> before every release has been checked, so a refusal leaves it
# empty; an <out-dir> that holds files is refused.
set -euo pipefail

NAME=fetch-published-debs.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
if [ $# -ne 1 ]; then
  echo "usage: GH_TOKEN=<token> $NAME <out-dir>" >&2
  exit 2
fi
OUT=$1
[ -n "${GH_TOKEN:-}" ] || fail "GH_TOKEN is empty: gh needs the workflow's token to list the releases"
[ -n "${GITHUB_REPOSITORY:-}" ] || fail "GITHUB_REPOSITORY is empty: it names the repository whose releases are listed"
for tool in gh sha256sum awk; do
  command -v "$tool" > /dev/null 2>&1 || fail "$tool is not installed"
done
if [ -e "$OUT" ] || [ -L "$OUT" ]; then
  if [ ! -d "$OUT" ] || [ -L "$OUT" ]; then fail "$OUT exists and is not a folder"; fi
  [ -z "$(ls -A "$OUT")" ] || fail "$OUT holds files already: the .deb files of the published releases go into an empty folder"
fi

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-published.XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT
mkdir "$SCRATCH/all"

TAGS=$(gh release list --repo "$GITHUB_REPOSITORY" --exclude-drafts --limit 1000 --json tagName --jq '.[].tagName') \
  || fail "gh could not list the published releases of $GITHUB_REPOSITORY"

RELEASES=0
DEBS=0
while IFS= read -r tag; do
  [ -n "$tag" ] || continue
  case $tag in
    -* | .* | *[!0-9A-Za-z._+~-]*) fail "the release tag '$tag' is not a name this script puts into a folder" ;;
  esac
  RELEASES=$((RELEASES + 1))
  dir=$SCRATCH/release-$tag
  mkdir "$dir"
  gh release download "$tag" --repo "$GITHUB_REPOSITORY" --pattern '*.deb' --pattern SHA256SUMS --dir "$dir" \
    || fail "the download of the .deb files and SHA256SUMS of release $tag failed"
  found=0
  for deb in "$dir"/*.deb; do
    [ -e "$deb" ] || continue
    found=1
    deb_name=$(basename "$deb")
    [ -f "$dir/SHA256SUMS" ] || fail "release $tag holds $deb_name and no SHA256SUMS to check it against"
    want=$(awk -v n="$deb_name" '$2 == n || $2 == "*" n { print $1; exit }' "$dir/SHA256SUMS")
    [ -n "$want" ] || fail "SHA256SUMS of release $tag has no line for $deb_name"
    have=$(sha256sum "$deb" | awk '{ print $1 }')
    [ "$have" = "$want" ] \
      || fail "$deb_name of release $tag has the SHA-256 $have, not the $want its SHA256SUMS names: it is not what was published"
    [ ! -e "$SCRATCH/all/$deb_name" ] || fail "$deb_name is in more than one published release: a published version keeps its one file"
    mv "$deb" "$SCRATCH/all/$deb_name"
    DEBS=$((DEBS + 1))
  done
  [ "$found" -eq 1 ] || echo "$NAME: release $tag holds no .deb" >&2
done <<< "$TAGS"

mkdir -p "$OUT"
for deb in "$SCRATCH"/all/*.deb; do
  [ -e "$deb" ] || continue
  mv "$deb" "$OUT/"
done
echo "$NAME: $DEBS .deb files of $RELEASES published releases are in $OUT"
