#!/bin/bash
# The steps of the job publish that are not GitHub's own actions. Runs only on a run that
# publishes; reviewed by reading.
#
#   GH_TOKEN=<token> .github/scripts/publish.sh draft   <release-dir> <version>
#   GH_TOKEN=<token> .github/scripts/publish.sh publish <release-dir> <version>
#   GH_TOKEN=<token> .github/scripts/publish.sh check   <release-dir> <version>
#
# draft: when no release of tag v<version> exists, `gh release create v<version> --draft
#   --verify-tag --title "YAAR <version>" --notes-file <notes>` with the eight files of
#   <release-dir> (the six packages, SHA256SUMS, SHA256SUMS.asc); the notes say what the
#   release holds and the install line, from packaging/site/base-url. When a draft of that
#   tag exists, its assets are replaced by these files. When a published release of that tag
#   exists with assets byte for byte these files (compared by SHA256SUMS), nothing is done
#   and it says so: a re-run of a finished publish only deploys the site again. A published
#   release with other assets is refused: a published version is never changed.
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
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "publish.sh: not built yet" >&2
exit 99
