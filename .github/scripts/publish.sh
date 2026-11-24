#!/bin/bash
# The steps of the job publish that are not GitHub's own actions; needs GH_TOKEN, GITHUB_REPOSITORY.
# GH_TOKEN=<token> .github/scripts/publish.sh draft|latest|publish|check <release-dir> <version>
# A published release with other assets is refused. `latest` says whether a published release has
# a greater version: it writes latest=true or latest=false to GITHUB_OUTPUT, and publish and check
# take that answer in YAAR_LATEST. Waits (s): YAAR_CHECK_WAIT 600, YAAR_PUBLISH_WAIT 60.
set -euo pipefail

NAME=publish.sh
fail() {
  echo "$NAME: $*" >&2
  exit 1
}
usage() {
  echo "usage: GH_TOKEN=<token> $NAME draft|latest|publish|check <release-dir> <version>" >&2
  exit 2
}
[ $# -eq 3 ] || usage
ACTION=$1
RELDIR=$2
VERSION=$3
case $ACTION in
  draft | latest | publish | check) ;;
  *) usage ;;
esac
case $VERSION in
  '' | [!0-9]* | *[!0-9A-Za-z.+~]*)
    echo "$NAME: a version starts with a digit and holds only letters, digits, '.', '+' and '~', not: $VERSION" >&2
    exit 2
    ;;
esac
# Whole seconds, no leading zero: the shell reads 08 as a bad octal number and 010 as 8.
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

# The notes: the changes of the tag, then the text of notes.md.in.
render_notes() {
  local in=$ROOT/packaging/release/notes.md.in b changes
  [ -f "$in" ] || fail "$in is missing"
  b=$(base_url)
  sed -e "s|@VERSION@|$VERSION|g" -e "s|@BASE_URL@|$b|g" "$in" > "$SCRATCH/text.md" || fail "the notes could not be written"
  if LEFT=$(grep -o -m1 '@[A-Za-z_][A-Za-z0-9_]*@' "$SCRATCH/text.md"); then
    fail "$in still holds the placeholder $LEFT after the replacements: the notes must not go out with it"
  fi
  changes=$(cd "$ROOT" && packaging/release/next-version.sh --notes "$TAG") \
    || fail "the changes of $TAG could not be listed: the checkout needs its whole history and the tag"
  {
    if [ -n "$changes" ]; then printf '%s\n\n' "$changes"; fi
    cat "$SCRATCH/text.md"
  } > "$SCRATCH/notes.md" || fail "the notes could not be written"
}

# The releases of the repository, one per line: the tag and whether it is a draft.
list_releases() {
  gh release list --repo "$REPO" --limit 1000 --json tagName,isDraft --jq '.[] | "\(.tagName) \(.isDraft)"' \
    || fail "gh could not list the releases of $REPO"
}

release_state() {
  local listing
  listing=$(list_releases) || exit 1
  printf '%s\n' "$listing" | awk -v t="$TAG" '
    $1 == t { print ($2 == "true") ? "draft" : "published"; found = 1; exit }
    END { if (!found) print "none" }'
}

# The tags of the published releases (a draft is not one) whose version is greater than this one,
# compared number by number.
greater_published() {
  local listing
  listing=$(list_releases) || exit 1
  printf '%s\n' "$listing" | awk -v mine="$VERSION" '
    function greater(a, b,   x, y, i) {
      split(a, x, ".")
      split(b, y, ".")
      for (i = 1; i <= 3; i++) {
        if (x[i] + 0 > y[i] + 0) return 1
        if (x[i] + 0 < y[i] + 0) return 0
      }
      return 0
    }
    $2 == "true" { next }
    $1 !~ /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/ { next }
    greater(substr($1, 2), mine) { print $1 }'
}

need_latest() {
  case ${YAAR_LATEST:-} in
    true | false) ;;
    *) fail "YAAR_LATEST must be true or false, the answer of '$NAME latest', not: ${YAAR_LATEST:-nothing}" ;;
  esac
}

release_assets() {
  gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name' | LC_ALL=C sort
}

# poll <seconds> <probe...>: runs the probe until it succeeds or time is up; leaves what it saw in
# $SEEN. First 5 s every 0.2 s, then every 2 s, so GitHub is not asked thousands of times.
SEEN=
POLL_END=0
poll() {
  local deadline=$1 start=$SECONDS elapsed pause
  shift
  case $deadline in
    '' | *[!0-9]*)
      SEEN="the deadline given to the wait is not a whole number of seconds: ${deadline:-nothing}"
      return 1
      ;;
  esac
  POLL_END=$((start + deadline))
  while :; do
    if "$@"; then return 0; fi
    elapsed=$((SECONDS - start))
    if [ "$elapsed" -ge "$deadline" ]; then return 1; fi
    if [ "$elapsed" -lt 5 ]; then
      sleep 0.2
    else
      pause=$((deadline - elapsed))
      [ "$pause" -le 2 ] || pause=2
      sleep "$pause"
    fi
  done
}

# look_limit <cap>: one look's time limit: <cap> or what is left of the wait, at least 1,
# since curl's --max-time 0 is no limit.
look_limit() {
  local cap=$1 left=$((POLL_END - SECONDS))
  [ "$left" -ge 1 ] || left=1
  if [ "$left" -lt "$cap" ]; then echo "$left"; else echo "$cap"; fi
}

probe_same() {
  local url=$1 file=$2 code
  if ! code=$(curl -sSL --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time "$(look_limit 30)" -o "$SCRATCH/got" -w '%{http_code}' "$url" 2> "$SCRATCH/curl.err"); then
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
  latest)
    printf '%s\n' "$VERSION" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
      || fail "a release version is three numbers x.y.z with no leading zeros, not '$VERSION'"
    greater=$(greater_published) || exit 1
    if [ -z "$greater" ]; then
      said="no published release of $REPO has a version greater than $VERSION: $TAG is published as the latest, and the site is deployed"
      answer=true
    else
      said="$(printf '%s' "$greater" | tr '\n' ' ')of $REPO is published with a greater version than $VERSION: $TAG is published, not as the latest, and the site is not deployed"
      answer=false
    fi
    echo "$NAME: $said"
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$said" >> "$GITHUB_STEP_SUMMARY"; fi
    echo "latest=$answer" >> "${GITHUB_OUTPUT:-/dev/null}"
    ;;
  publish)
    need_latest
    if [ "$YAAR_LATEST" = true ]; then
      gh release edit "$TAG" --repo "$REPO" --draft=false --latest || fail "gh could not publish the release $TAG as the latest"
      wait_same "${YAAR_PUBLISH_WAIT:-60}" "https://github.com/$REPO/releases/latest/download/SHA256SUMS" "$RELDIR/SHA256SUMS"
      echo "$NAME: $TAG is published as the latest release, and releases/latest/download/ answers with its SHA256SUMS"
    else
      gh release edit "$TAG" --repo "$REPO" --draft=false --latest=false || fail "gh could not publish the release $TAG"
      echo "$NAME: $TAG is published, not as the latest: a greater version is published"
    fi
    ;;
  check)
    need_latest
    [ -n "${GITHUB_SHA:-}" ] || fail "GITHUB_SHA is empty: the commit of this run is what the tag must point at"
    SITE=$(cd "$RELDIR/.." && pwd)/site
    [ -d "$SITE" ] || fail "$SITE is not a folder: the site of this run lies beside the release folder"
    LICENCE=$ROOT/LICENSE
    [ -f "$LICENCE" ] || fail "$LICENCE is missing"
    B=$(base_url)
    wait=${YAAR_CHECK_WAIT:-600}
    WAITED="the $wait s the whole check waits"
    CHECK_START=$SECONDS
    left() {
      local l=$((wait - (SECONDS - CHECK_START)))
      [ "$l" -gt 0 ] || l=0
      echo "$l"
    }

    # An annotated tag lists its commit on the line "^{}". git has a deadline of its own and runs
    # under setsid: its pid is its process group's id, killed whole at the deadline (git remote-http
    # too).
    command -v setsid > /dev/null || fail "setsid is missing: it detaches the git that asks GitHub for the tag, so that the deadline can end it with its helper"
    git_limit=60
    [ "$(left)" -ge "$git_limit" ] || git_limit=$(left)
    [ "$git_limit" -ge 1 ] || git_limit=1
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

    probe_licence() {
      local url="https://github.com/$REPO/archive/refs/tags/$TAG.tar.gz" code entry
      if ! code=$(curl -sSL --proto '=https' --proto-redir '=https' --tlsv1.2 --max-time "$(look_limit 120)" -o "$SCRATCH/source.tar.gz" -w '%{http_code}' "$url" 2> "$SCRATCH/curl.err"); then
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

    if [ "$YAAR_LATEST" = true ]; then
      wait_same "$(left)" "$B/install.sh" "$SITE/install.sh"
      wait_same "$(left)" "$B/apt/dists/stable/InRelease" "$SITE/apt/dists/stable/InRelease"
      wait_same "$(left)" "$B/apt/yaar-review.asc" "$SITE/apt/yaar-review.asc"
      echo "$NAME: the tag $TAG of $REPO is at $GITHUB_SHA, its source holds LICENSE, and $B answers with the files of this run"
    else
      echo "$NAME: the tag $TAG of $REPO is at $GITHUB_SHA and its source holds LICENSE; the site was not deployed by this run, so it is not compared"
    fi
    ;;
esac
