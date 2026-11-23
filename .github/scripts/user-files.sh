#!/bin/bash
# Row PKG-09: after the uninstall no file of the app remains in the user's folders, the
# Keeper's lock file included, but the user's reviews (and pi's extension file, which pi's
# own settings may still name) until the uninstall is asked to remove them too; and nothing
# of the user's is removed.
#
#   .github/scripts/user-files.sh plant <user>
#   .github/scripts/user-files.sh snapshot <user> <file>
#   .github/scripts/user-files.sh present <user>
#   .github/scripts/user-files.sh check <user> <file>
#
# Run as root on Linux (the job's steps are root; what is done as <user> goes through
# `sudo -u <user>`), as the user on macOS (the runner's user is the user).
#
# THE ROOTS. The folders walked are the colon-separated list YAAR_USER_FILES_ROOTS when it
# is set (a builder on the owner's Mac sets it to its scratch HOME, sets HOME to the same
# folder, and walks nothing else). Unset, they are: the user's home; on Linux
# /run/user/<uid> when it exists, and /tmp; on macOS /private/tmp (never /tmp, which is a
# link there and which a walk that follows no link would not enter) and the user's own
# temporary and cache folders (`getconf DARWIN_USER_TEMP_DIR`, `getconf
# DARWIN_USER_CACHE_DIR`, run as that user). Each root is resolved once with `cd -P`.
# PRUNED, never entered: on GitHub's Mac, which runs every job in its runner user's home,
# the folder above $RUNNER_TEMP (that is $HOME/work, which holds the checkout and the job's
# scratch), $RUNNER_TOOL_CACHE, $HOME/Library/Android, $HOME/.android, $HOME/.rustup,
# $HOME/.cargo, $HOME/.npm, $HOME/.nvm and $HOME/Library/Developer, each only when it exists
# and lies under a root; on Linux nothing is pruned (the job's checkout and scratch lie
# under /__w, outside the roots). YAAR_USER_FILES_PRUNE (colon-separated) adds to the list.
#
# JUDGED PATHS. On Linux every path under the roots that belongs to <user>. On macOS, where
# the runner's own services write into the home while the job runs, a path is judged when
# its FULL path, case ignored, holds "yaar-review", "io.github.tsadoq" or "yaar.app" (never
# the bare "yaar": the runner's checkout path holds it); other new paths are printed, not
# judged.
#
# THE USER'S OWN FILES, which no uninstall may touch (`plant` makes them before the install
# and before the job's first snapshot, owned by <user>):
#   Linux:  ~/.local/share/yaar-review-notes/notes.txt (a file beside the data folder, whose
#           name begins as the app's does) and ~/projects/mine, a git repository with one
#           commit (an identity in its own .git/config; GIT_CONFIG_GLOBAL=/dev/null,
#           GIT_CONFIG_NOSYSTEM=1);
#   macOS:  ~/Library/Application Support/yaar-review-notes/notes.txt and
#           ~/yaar-review-user-repo, the same repository.
#   Their paths are judged paths on both systems.
#
# THE DATA FOLDER is ~/.local/share/yaar-review on Linux (the job sets no XDG_DATA_HOME and,
# through sudo and env -i, no XDG_RUNTIME_DIR, so the Keeper's socket and its lock lie there
# too) and ~/Library/Application Support/yaar-review on macOS.
#
# snapshot: writes to <file> the sorted list of every path under the roots that belongs to
#   the user, without following links (`find -P <root> ... -user <uid>`, pruned as above),
#   each as "<kind> <path>" (kind: d, f, l, s for a socket, o other).
# present, run after the steps that ran the app and before the uninstall: asserts that the
#   data folder holds keeper.sock.lock (the Keeper ran as the user and took its lock) and
#   store.db (the Keeper opened its Store: the worktree step made a workspace). Fails naming
#   each missing file. This is what makes "the Keeper's lock file included" and "the reviews
#   stay" clauses that can fail.
# check, after the job's uninstall step (`--uninstall`, which keeps the reviews), with <file>
#   the snapshot taken before the install:
#   1. A snapshot. (a) Among the judged paths that <file> does not hold, only these may be
#      there: the data folder's store.db (with -wal and -shm), its folder states and what is
#      under it, its folder agents/pi and what is under it, and the folders above them up to
#      the home. Anything else fails, the lock file keeper.sock.lock first of all. (b) store.db
#      must be there (the reviews stayed). (c) Every path of <file> must still be there, on
#      Linux every one, on macOS every judged one: an uninstall that removed a file of the
#      user's (the planted ones among them) fails, naming it.
#   2. Runs the install line once more, as the user, with `--uninstall --purge`
#      (YAAR_BASE_URL=http://127.0.0.1:8000, as the job's other install steps).
#   3. A snapshot: no judged path that <file> does not hold may be there, but on Linux these,
#      which Mesa and sudo write for any program and the app does not name:
#      ~/.cache/mesa_shader_cache* (Mesa's shader cache, whose name changes between Mesa
#      releases: mesa_shader_cache, mesa_shader_cache_db) with what is under them, ~/.cache
#      (when only those are under it) and ~/.sudo_as_admin_successful. Every path of <file>
#      must still be there, as in 1 (c).
#   Another path that the system writes for any program may join that list only by the
#   orchestrator's ruling, with the program that writes it named.
#   The job's own scratch files lie under $RUNNER_TEMP, never in the user's folders; the
#   steps that make files in the home (macos-login-path.sh, linux-login-path.sh) remove them
#   before this one, and put back byte for byte the start-up file they changed.
#   Every path that fails is printed, with its kind and size.
#
# Exit codes: 0 holds; 1 a leftover, a missing file or a removed file (each named); 2 usage
# error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "user-files.sh: not built yet" >&2
exit 99
