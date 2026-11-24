#!/bin/sh
# PUBLISH=<true|false> REF_TYPE=<branch|tag> REF_NAME=<name> what-this-run-is.sh
# Writes version, later (<version>+rehearsal, sorts after it for apt) and publish to GITHUB_OUTPUT.
# Exit 1: publishing from a non-tag or off main, or a tag other than v<version>.
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
