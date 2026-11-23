#!/bin/bash
# Row PKG-09: after the uninstall no file of the app remains in the user's folders, the
# Keeper's lock file included.
#
#   .github/scripts/user-files.sh snapshot <user> <file>
#   .github/scripts/user-files.sh check <user> <file>
#
# The user's folders: the user's home; on Linux $XDG_RUNTIME_DIR of that user when it exists
# (/run/user/<uid>); /tmp; on macOS also the user's own temporary and cache folders
# (`getconf DARWIN_USER_TEMP_DIR`, `getconf DARWIN_USER_CACHE_DIR`, run as that user).
# snapshot: writes to <file> the sorted list of every path under those folders that belongs
# to the user, without following links (run as root via sudo on Linux, as the user on macOS).
# check, after the job's uninstall step (`--uninstall`, which keeps the user's reviews). A
# "new path" is one the current snapshot holds and <file> does not. "Judged": on Linux every
# new path; on macOS, where the runner's own services write into the home while the job runs,
# every new path whose name holds "yaar" or "io.github.tsadoq", case ignored (the other new
# paths are printed, not judged).
#   1. A snapshot. Among the judged new paths only these may be there: the data folder's
#      store.db (with -wal and -shm), its folder states and what is under it, and the folders
#      above them up to the home. The data folder is ~/.local/share/yaar-review on Linux and
#      ~/Library/Application Support/yaar-review on macOS. Anything else fails, the lock file
#      keeper.sock.lock first of all.
#   2. Runs the install line once more, as the user, with `--uninstall --purge`
#      (YAAR_BASE_URL=http://127.0.0.1:8000, as the job's other install steps).
#   3. A snapshot: no judged new path may be there, but on Linux these, which Mesa and sudo
#      write for any program and the app does not name: ~/.cache/mesa_shader_cache with
#      what is under it, ~/.cache (when only that is under it) and
#      ~/.sudo_as_admin_successful.
#   Another path that the system writes for any program may join that list only by the
#   orchestrator's ruling, with the program that writes it named.
#   The job's own scratch files lie under $RUNNER_TEMP, never in the user's folders; the
#   steps that make files in the home (macos-login-path.sh) remove them before this one.
#   Every path that fails is printed, with its kind and size.
#
# Exit codes: 0 holds; 1 a leftover (each named); 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "user-files.sh: not built yet" >&2
exit 99
