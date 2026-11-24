#!/bin/bash
# The steps of the job publish that are not GitHub's own actions. Runs only on a run that
# publishes; reviewed by reading.
#
#   GH_TOKEN=<token> .github/scripts/publish.sh draft   <release-dir> <version>
#   GH_TOKEN=<token> .github/scripts/publish.sh publish <release-dir> <version>
#   GH_TOKEN=<token> .github/scripts/publish.sh check   <release-dir> <version>
#
# Refuses before any gh call when GH_TOKEN or GITHUB_REPOSITORY is empty.
#
# draft: the notes are packaging/release/notes.md.in with @VERSION@ replaced by <version> and
#   @BASE_URL@ by the one line of packaging/site/base-url (`head -n 1`), written to a
#   temporary file. When no release of tag v<version> exists, `gh release create
#   v<version> --draft --verify-tag --title "YAAR <version>" --notes-file <notes>` with the
#   eight files of <release-dir> (the six packages, SHA256SUMS, SHA256SUMS.asc). When a draft
#   of that tag exists, its assets are replaced by these files (`gh release upload
#   --clobber`) and its notes by <notes>. When a published release of that tag exists with
#   assets byte for byte these files (its SHA256SUMS downloaded and compared with
#   <release-dir>/SHA256SUMS, and the asset names equal), nothing is done and it says so:
#   that is the owner re-running the job publish alone of a run that already published, to
#   deploy that run's site again (PLAN § 7, taking a release back). A published release with
#   other assets is refused: a published version is never changed.
# publish: `gh release edit v<version> --draft=false --latest`; then waits (60 s) until
#   https://github.com/<repository>/releases/latest/download/SHA256SUMS answers with the
#   bytes of <release-dir>/SHA256SUMS.
# check: the release's tag points at this run's commit (GITHUB_SHA); the source archive
#   GitHub made for the tag holds LICENSE with the bytes of the repository's (row PKG-11);
#   the site answers https://<base>/install.sh and https://<base>/apt/dists/stable/InRelease
#   with the bytes of final/site (the Pages cache may lag: up to 10 minutes of retries).
#
# Exit codes: 0 done; 1 refused or failed (a sentence says which step); 2 usage error.
#
# Beyond the header's list:
#   - <release-dir> must hold the eight files and no other, else it refuses before any gh
#     call; <version> is letters, digits, '.', '+' and '~' (a usage error, exit 2);
#   - which of the three states a release is in (none, a draft, published) is read from
#     `gh release list --json tagName,isDraft`, 1000 entries; a list that fails is a failure,
#     never "none". A draft's assets that are not among the eight are deleted
#     (`gh release delete-asset`), so that the draft holds these files and no others;
#   - publish and check wait for what they check: the first 5 s every 0.2 s, then every 2 s,
#     for YAAR_PUBLISH_WAIT seconds (default 60) and YAAR_CHECK_WAIT seconds (default 600).
#     The workflow sets neither: they are there so that the refusal can be seen in seconds.
#     Past the deadline the sentence says what the last look found;
#   - check asks GitHub for the tag with `git ls-remote` and the source archive with curl,
#     both anonymously: the repository is public, and the row is about what the public sees;
#     GITHUB_SHA must be set for it. It also compares the site's apt/yaar-review.asc;
#   - the site is <release-dir>/../site, as the job lays the artifact out.
# No token reaches a command line or an output: gh reads GH_TOKEN from the environment, and
# curl and git are given none.
set -euo pipefail

NAME=publish.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
usage() {
  echo "usage: GH_TOKEN=<token> $NAME draft|publish|check <release-dir> <version>" >&2
  exit 2
}
[ $# -eq 3 ] || usage
ACTION=$1
RELDIR=$2
VERSION=$3
case $ACTION in
  draft | publish | check) ;;
  *) usage ;;
esac
case $VERSION in
  '' | [!0-9]* | *[!0-9A-Za-z.+~]*)
    echo "$NAME: a version starts with a digit and holds only letters, digits, '.', '+' and '~', not: $VERSION" >&2
    exit 2
    ;;
esac
# Before any gh call.
[ -n "${GH_TOKEN:-}" ] || fail "GH_TOKEN is empty: gh needs the workflow's token, and nothing was called"
[ -n "${GITHUB_REPOSITORY:-}" ] || fail "GITHUB_REPOSITORY is empty: it names the repository the release lives in, and nothing was called"
REPO=$GITHUB_REPOSITORY
TAG=v$VERSION
TITLE="YAAR $VERSION"

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
[ -d "$RELDIR" ] || fail "$RELDIR is not a folder"
RELDIR=$(cd "$RELDIR" && pwd)

FILES="yaar-review_${VERSION}_amd64.deb
yaar-review_${VERSION}_arm64.deb
yaar-review-${VERSION}-linux-x86_64.tar.gz
yaar-review-${VERSION}-linux-aarch64.tar.gz
yaar-review-${VERSION}-macos-universal.zip
yaar-review-${VERSION}-macos-universal.dmg
SHA256SUMS
SHA256SUMS.asc"
while IFS= read -r f; do
  [ -f "$RELDIR/$f" ] || fail "$RELDIR/$f is missing: a release of $VERSION holds the eight files"
done <<< "$FILES"
HAVE=$(find "$RELDIR" -mindepth 1 -maxdepth 1 | LC_ALL=C sort) || fail "$RELDIR cannot be read"
while IFS= read -r path; do
  name=$(basename "$path")
  printf '%s\n' "$FILES" | grep -qxF -- "$name" || fail "$path is not one of the eight files of a release: it would not be published"
done <<< "$HAVE"

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-publish.XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT

# The base address: the first line of its file.
base_url() {
  local f=$ROOT/packaging/site/base-url b
  [ -f "$f" ] || fail "$f is missing"
  b=$(head -n 1 "$f") || fail "$f cannot be read"
  case $b in
    *[[:space:]\&\|\\]* | */ | '') fail "$f: the base address is not a https:// address with no '/' at its end: ${b:-nothing}" ;;
    https://?*) ;;
    *) fail "$f: the base address is not a https:// address with no '/' at its end: $b" ;;
  esac
  printf '%s\n' "$b"
}

# The notes: notes.md.in with its two placeholders replaced, written to a file of the scratch.
render_notes() {
  local in=$ROOT/packaging/release/notes.md.in b
  [ -f "$in" ] || fail "$in is missing"
  b=$(base_url)
  sed -e "s|@VERSION@|$VERSION|g" -e "s|@BASE_URL@|$b|g" "$in" > "$SCRATCH/notes.md" || fail "the notes could not be written"
  if LEFT=$(grep -o -m1 '@[A-Za-z_][A-Za-z_]*@' "$SCRATCH/notes.md"); then
    fail "$in still holds the placeholder $LEFT after the replacements: the notes must not go out with it"
  fi
  if grep -q '@' "$SCRATCH/notes.md"; then
    fail "the notes rendered from $in hold an '@'"
  fi
}

# none | draft | published, for the tag.
release_state() {
  local listing
  listing=$(gh release list --repo "$REPO" --limit 1000 --json tagName,isDraft --jq '.[] | "\(.tagName) \(.isDraft)"') \
    || fail "gh could not list the releases of $REPO"
  printf '%s\n' "$listing" | awk -v t="$TAG" '
    $1 == t { print ($2 == "true") ? "draft" : "published"; found = 1; exit }
    END { if (!found) print "none" }'
}

release_assets() {
  gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name' | LC_ALL=C sort
}

# poll <seconds> <probe...>: runs the probe until it succeeds or the seconds are up. The
# probe leaves what it saw in $SEEN. The first 5 s every 0.2 s, then every 2 s.
SEEN=
poll() {
  local deadline=$1 start=$SECONDS elapsed
  shift
  while :; do
    if "$@"; then return 0; fi
    elapsed=$((SECONDS - start))
    if [ "$elapsed" -ge "$deadline" ]; then return 1; fi
    if [ "$elapsed" -lt 5 ]; then sleep 0.2; else sleep 2; fi
  done
}

# probe_same <url> <file>: the address answers with the bytes of the file.
probe_same() {
  local url=$1 file=$2 code
  if ! code=$(curl -sSL --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 30 -o "$SCRATCH/got" -w '%{http_code}' "$url" 2> "$SCRATCH/curl.err"); then
    SEEN="curl failed: $(head -n 1 "$SCRATCH/curl.err")"
    return 1
  fi
  if [ "$code" != 200 ]; then
    SEEN="HTTP $code"
    return 1
  fi
  if ! cmp -s "$SCRATCH/got" "$file"; then
    SEEN="$(wc -c < "$SCRATCH/got" | awk '{ print $1 }') bytes that are not the bytes of $file"
    return 1
  fi
  return 0
}

# wait_same <seconds> <url> <file>: said as a sentence when it never answers right.
wait_same() {
  local seconds=$1 url=$2 file=$3
  if ! poll "$seconds" probe_same "$url" "$file"; then
    fail "$url did not answer with the bytes of $file within $seconds s; the last look found: $SEEN"
  fi
}

case $ACTION in
  draft)
    render_notes
    FILE_ARGS=()
    while IFS= read -r f; do
      FILE_ARGS+=("$RELDIR/$f")
    done <<< "$FILES"
    state=$(release_state)
    case $state in
      none)
        gh release create "$TAG" --repo "$REPO" --draft --verify-tag --title "$TITLE" --notes-file "$SCRATCH/notes.md" "${FILE_ARGS[@]}" \
          || fail "gh could not make the draft release $TAG (--verify-tag refuses a tag that is not on GitHub)"
        echo "$NAME: made the draft release $TAG with the eight files"
        ;;
      draft)
        # A draft takes the eight files and their notes, and no file of an earlier try.
        assets=$(release_assets) || fail "gh could not list the assets of the draft $TAG"
        while IFS= read -r existing; do
          [ -n "$existing" ] || continue
          if ! printf '%s\n' "$FILES" | grep -qxF -- "$existing"; then
            gh release delete-asset "$TAG" "$existing" --repo "$REPO" --yes \
              || fail "gh could not delete the asset $existing of the draft $TAG"
          fi
        done <<< "$assets"
        gh release upload "$TAG" --repo "$REPO" --clobber "${FILE_ARGS[@]}" || fail "gh could not upload the eight files to the draft $TAG"
        gh release edit "$TAG" --repo "$REPO" --title "$TITLE" --notes-file "$SCRATCH/notes.md" \
          || fail "gh could not replace the notes of the draft $TAG"
        echo "$NAME: the draft release $TAG took up the eight files and its notes again"
        ;;
      published)
        assets=$(release_assets) || fail "gh could not list the assets of the release $TAG"
        if [ "$assets" != "$(printf '%s\n' "$FILES" | LC_ALL=C sort)" ]; then
          fail "the release $TAG is published with other assets than these eight files: a published version is never changed"
        fi
        mkdir "$SCRATCH/published"
        gh release download "$TAG" --repo "$REPO" --pattern SHA256SUMS --dir "$SCRATCH/published" \
          || fail "gh could not download SHA256SUMS of the published release $TAG"
        cmp -s "$SCRATCH/published/SHA256SUMS" "$RELDIR/SHA256SUMS" \
          || fail "the SHA256SUMS of the published release $TAG is not $RELDIR/SHA256SUMS: a published version is never changed"
        echo "$NAME: the release $TAG is published already with these files: nothing to do"
        ;;
    esac
    ;;
  publish)
    gh release edit "$TAG" --repo "$REPO" --draft=false --latest || fail "gh could not publish the release $TAG as the latest"
    wait_same "${YAAR_PUBLISH_WAIT:-60}" "https://github.com/$REPO/releases/latest/download/SHA256SUMS" "$RELDIR/SHA256SUMS"
    echo "$NAME: $TAG is published as the latest release, and releases/latest/download/ answers with its SHA256SUMS"
    ;;
  check)
    [ -n "${GITHUB_SHA:-}" ] || fail "GITHUB_SHA is empty: the commit of this run is what the tag must point at"
    SITE=$(cd "$RELDIR/.." && pwd)/site
    [ -d "$SITE" ] || fail "$SITE is not a folder: the site of this run lies beside the release folder"
    LICENCE=$ROOT/LICENSE
    [ -f "$LICENCE" ] || fail "$LICENCE is missing"
    B=$(base_url)
    wait=${YAAR_CHECK_WAIT:-600}

    # The tag, on the public repository, at this run's commit. An annotated tag lists its
    # commit on the line "^{}".
    refs=$(GIT_TERMINAL_PROMPT=0 git ls-remote "https://github.com/$REPO.git" "refs/tags/$TAG" "refs/tags/$TAG^{}") \
      || fail "git could not ask https://github.com/$REPO.git for the tag $TAG"
    peeled=$(printf '%s\n' "$refs" | awk -v r="refs/tags/$TAG^{}" '$2 == r { print $1; exit }')
    plain=$(printf '%s\n' "$refs" | awk -v r="refs/tags/$TAG" '$2 == r { print $1; exit }')
    tagged=${peeled:-$plain}
    [ -n "$tagged" ] || fail "the public repository $REPO has no tag $TAG"
    [ "$tagged" = "$GITHUB_SHA" ] || fail "the tag $TAG of $REPO points at $tagged, not at the commit of this run, $GITHUB_SHA"

    # Its source archive holds this licence.
    probe_licence() {
      local url="https://github.com/$REPO/archive/refs/tags/$TAG.tar.gz" code entry
      if ! code=$(curl -sSL --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time 120 -o "$SCRATCH/source.tar.gz" -w '%{http_code}' "$url" 2> "$SCRATCH/curl.err"); then
        SEEN="curl failed: $(head -n 1 "$SCRATCH/curl.err")"
        return 1
      fi
      if [ "$code" != 200 ]; then
        SEEN="HTTP $code from $url"
        return 1
      fi
      if ! tar -tzf "$SCRATCH/source.tar.gz" > "$SCRATCH/source.list" 2> "$SCRATCH/tar.err"; then
        SEEN="the archive is not a tar.gz: $(head -n 1 "$SCRATCH/tar.err")"
        return 1
      fi
      entry=$(awk '$0 ~ /^[^\/]+\/LICENSE$/ { print; exit }' "$SCRATCH/source.list")
      if [ -z "$entry" ]; then
        SEEN="the archive holds no LICENSE at its top"
        return 1
      fi
      if ! tar -xzOf "$SCRATCH/source.tar.gz" "$entry" > "$SCRATCH/LICENSE.got" 2> "$SCRATCH/tar.err"; then
        SEEN="$entry could not be read from the archive: $(head -n 1 "$SCRATCH/tar.err")"
        return 1
      fi
      if ! cmp -s "$SCRATCH/LICENSE.got" "$LICENCE"; then
        SEEN="$entry is not the LICENSE of this repository"
        return 1
      fi
      return 0
    }
    if ! poll "$wait" probe_licence; then
      fail "the source archive of $TAG on $REPO does not hold the licence within $wait s; the last look found: $SEEN"
    fi

    # The site answers with the new files.
    wait_same "$wait" "$B/install.sh" "$SITE/install.sh"
    wait_same "$wait" "$B/apt/dists/stable/InRelease" "$SITE/apt/dists/stable/InRelease"
    wait_same "$wait" "$B/apt/yaar-review.asc" "$SITE/apt/yaar-review.asc"
    echo "$NAME: the tag $TAG of $REPO is at $GITHUB_SHA, its source holds LICENSE, and $B answers with the files of this run"
    ;;
esac
