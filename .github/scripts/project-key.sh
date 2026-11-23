#!/bin/bash
# The project's key of the apt repository, in the job assemble-release only, on a run that
# publishes. Reviewed by reading: no test runs it.
#
#   .github/scripts/project-key.sh import
#   .github/scripts/project-key.sh remove
#
# import, with APT_SIGNING_KEY (the secret: the armoured OpenPGP secret key the owner made
# once, ed25519, for signing, with no passphrase on this copy): refuses when it is empty;
# makes $RUNNER_TEMP/yaar-apt-key (mode 700) as GNUPGHOME and imports the key there from
# standard input (never from a command line); reads its fingerprint; refuses, changing
# nothing more, unless it equals the one line of packaging/apt/project-key.fingerprint
# that is not a comment (a key
# that is not the project's would cut every installed copy off from updates); writes
# GNUPGHOME and YAAR_APT_KEY=<fingerprint> into $GITHUB_ENV. Prints the fingerprint only.
# remove: kills the gpg agent of that folder and deletes the folder. Succeeds when there is
# nothing to remove.
#
# Exit codes: 0 done; 1 refused or failed; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "project-key.sh: not built yet" >&2
exit 99
