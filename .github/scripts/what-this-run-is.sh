#!/bin/sh
# The body of the step "What this run is" of the job `version`: the one version number, the
# later version the rehearsal makes, and whether this run publishes. Written by the contract
# of slice 10 and complete; no builder edits it.
#
#   PUBLISH=<true|false> REF_TYPE=<branch|tag> REF_NAME=<name> what-this-run-is.sh
#
# Environment:
#   PUBLISH    THE ONE CONDITION, computed by the workflow: true only for a run the owner
#              started by hand with the input `publish` on.
#   REF_TYPE   github.ref_type of the run: "tag" or "branch".
#   REF_NAME   github.ref_name: the tag's or the branch's short name.
#   GITHUB_OUTPUT, GITHUB_STEP_SUMMARY   set by the runner; when unset (a run by hand) the
#              outputs go to standard output only.
#
# Outputs (lines `key=value` appended to GITHUB_OUTPUT):
#   version    packaging/version.sh's number
#   later      "<version>+rehearsal": the version of the second .deb the rehearsal makes
#              from the first, to show an upgrade with apt (row PKG-06). It sorts after
#              <version> for apt and is never published.
#   publish    "true" or "false"
#
# Refusals, each a sentence on standard error and exit 1:
#   - PUBLISH is true and the run is not on a tag;
#   - PUBLISH is true and the commit of the tag is not on main: `git merge-base --is-ancestor
#     HEAD origin/main` fails (the job checks out with fetch-depth 0, so origin/main is
#     there). A tag on an unmerged branch, whose workflow may have been changed, never
#     reaches the secrets;
#   - the run is on a tag (published or rehearsed) and the tag is not v<version>: a tag
#     that differs from the manifest is refused, so a release always carries the version
#     its programs print.
# Exit 0 otherwise.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
VERSION=$("$HERE/../../packaging/version.sh")
LATER="$VERSION+rehearsal"
case "${PUBLISH:-false}" in
  true) PUBLISH=true ;;
  false | "") PUBLISH=false ;;
  *) echo "what-this-run-is.sh: PUBLISH must be true or false, not '$PUBLISH'" >&2; exit 1 ;;
esac
if [ "$PUBLISH" = true ] && [ "${REF_TYPE:-}" != tag ]; then
  echo "what-this-run-is.sh: a release is published from a tag, and this run is on the ${REF_TYPE:-unknown} '${REF_NAME:-}'" >&2
  exit 1
fi
if [ "$PUBLISH" = true ] && ! git -C "$HERE/../.." merge-base --is-ancestor HEAD origin/main; then
  echo "what-this-run-is.sh: a release is published only from a commit on main, and the commit of '${REF_NAME:-}' is not on origin/main" >&2
  exit 1
fi
if [ "${REF_TYPE:-}" = tag ] && [ "${REF_NAME:-}" != "v$VERSION" ]; then
  echo "what-this-run-is.sh: the tag '${REF_NAME:-}' is not v$VERSION, the version of Cargo.toml" >&2
  exit 1
fi
if [ "$PUBLISH" = true ]; then
  said="this run PUBLISHES version $VERSION: it signs with the Developer ID, notarises, and makes the GitHub release and the Pages site"
else
  said="this run is a rehearsal of version $VERSION: it publishes nothing, sends nothing to Apple, signs the app ad hoc and the apt repository with a throwaway key"
fi
echo "$said"
{
  echo "version=$VERSION"
  echo "later=$LATER"
  echo "publish=$PUBLISH"
} | tee -a "${GITHUB_OUTPUT:-/dev/null}"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$said" >> "$GITHUB_STEP_SUMMARY"; fi
