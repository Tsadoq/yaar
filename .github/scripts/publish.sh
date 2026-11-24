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
#   with the bytes of final/site (the Pages cache may lag: about 10 minutes of retries in
#   all, one deadline for the whole check, see below).
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
#     Each is a whole number of seconds with no leading zero (the shell reads 08 as an
#     error and 010 as 8), else it refuses with exit 2 before any gh call, whatever the
#     action (a value that is not a number would make a wait that never ends); a wait also
#     refuses to start on a deadline that is no number;
#     Past the deadline the sentence says what the last look found;
#   - check has one deadline, YAAR_CHECK_WAIT seconds from its first look, for its four waits
#     together (the archive's licence, install.sh, InRelease, the key): each wait gets what is
#     left, and a wait that finds none left still looks once. So check ends within the
#     seconds it names, plus the time of its last look, and well inside the job's 20 minutes
#     with the 60 s of publish and the deployment;
#   - check asks GitHub for the tag with `git ls-remote` (given 60 s, or what is left of
#     the check's deadline if that is less but at least 10 s, then stopped by its pid and
#     named) and the source archive with curl, both anonymously: the repository is public,
#     and the row is about what the public sees; GITHUB_SHA must be set for it. It also
#     compares the site's apt/yaar-review.asc;
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
# Both waits are whole numbers of seconds, written without a leading zero (the shell reads
# 08 as a bad octal number and 010 as 8): anything else would end no wait.
for var in YAAR_PUBLISH_WAIT YAAR_CHECK_WAIT; do
  if [ -n "${!var+set}" ]; then
    case ${!var} in
      '' | *[!0-9]* | 0?*)
        echo "$NAME: $var must be a whole number of seconds with no leading zero, not: ${!var}" >&2
        exit 2
        ;;
    esac
  fi
done
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
  # A placeholder is a name in capitals between two '@'. Any other '@' (a contact address, a
  # mention) is the text's own.
  if LEFT=$(grep -o -m1 '@[A-Z][A-Z0-9_]*@' "$SCRATCH/notes.md"); then
    fail "$in still holds the placeholder $LEFT after the replacements: the notes must not go out with it"
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
  # A deadline that is no whole number can never be reached: a wait must not start on one.
  case $deadline in
    '' | *[!0-9]*)
      SEEN="the deadline given to the wait is not a whole number of seconds: ${deadline:-nothing}"
      return 1
      ;;
  esac
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

# wait_same <seconds> <url> <file>: said as a sentence when it never answers right. WAITED
# names the whole wait in the sentence when the seconds are what is left of a larger one.
WAITED=
wait_same() {
  local seconds=$1 url=$2 file=$3
  if ! poll "$seconds" probe_same "$url" "$file"; then
    fail "$url did not answer with the bytes of $file within ${WAITED:-$seconds s}; the last look found: $SEEN"
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
    # One deadline for the four waits below: each is given what is left of it.
    WAITED="the $wait s the whole check waits"
    CHECK_START=$SECONDS
    left() {
      local l=$((wait - (SECONDS - CHECK_START)))
      [ "$l" -gt 0 ] || l=0
      echo "$l"
    }

    # The tag, on the public repository, at this run's commit. An annotated tag lists its
    # commit on the line "^{}".
    # git gets a deadline of its own, 60 s or what is left of the check's if that is less,
    # but never under 10 s so that it looks once whatever is left: a connection that stalls
    # would otherwise hold the step until the job's timeout, saying nothing of what it waited
    # for. It runs in the background in a session of its own (setsid, standard input from
    # /dev/null, output to files), so that its pid is the id of its whole process group: at
    # the deadline the group is ended, and git's own helper (git remote-http) goes with it.
    command -v setsid > /dev/null || fail "setsid is missing: it detaches the git that asks GitHub for the tag, so that the deadline can end it with its helper"
    git_limit=60
    [ "$(left)" -ge "$git_limit" ] || git_limit=$(left)
    [ "$git_limit" -ge 10 ] || git_limit=10
    GIT_TERMINAL_PROMPT=0 GIT_HTTP_LOW_SPEED_LIMIT=1 GIT_HTTP_LOW_SPEED_TIME=30 \
      setsid git ls-remote "https://github.com/$REPO.git" "refs/tags/$TAG" "refs/tags/$TAG^{}" \
      > "$SCRATCH/refs" 2> "$SCRATCH/git.err" < /dev/null &
    git_pid=$!
    git_start=$SECONDS
    while kill -0 "$git_pid" 2> /dev/null; do
      if [ $((SECONDS - git_start)) -ge "$git_limit" ]; then
        if kill -KILL -- "-$git_pid" 2> /dev/null; then
          wait "$git_pid" 2> /dev/null || :
          git_said=$(tail -n 1 "$SCRATCH/git.err") || git_said=
          fail "git did not answer within $git_limit s when asked https://github.com/$REPO.git for the tag $TAG; the last it printed: ${git_said:-nothing}"
        fi
      fi
      sleep 0.2
    done
    wait "$git_pid" || fail "git could not ask https://github.com/$REPO.git for the tag $TAG: $(head -n 1 "$SCRATCH/git.err")"
    refs=$(cat "$SCRATCH/refs")
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
    if ! poll "$(left)" probe_licence; then
      fail "the source archive of $TAG on $REPO does not hold the licence within $WAITED; the last look found: $SEEN"
    fi

    # The site answers with the new files.
    wait_same "$(left)" "$B/install.sh" "$SITE/install.sh"
    wait_same "$(left)" "$B/apt/dists/stable/InRelease" "$SITE/apt/dists/stable/InRelease"
    wait_same "$(left)" "$B/apt/yaar-review.asc" "$SITE/apt/yaar-review.asc"
    echo "$NAME: the tag $TAG of $REPO is at $GITHUB_SHA, its source holds LICENSE, and $B answers with the files of this run"
    ;;
esac
