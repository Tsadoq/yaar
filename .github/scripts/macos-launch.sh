#!/bin/bash
# Row PKG-08 on macOS: the installed app starts from the Dock with no terminal, as an
# application. Runs on GitHub's Mac only: it opens a window, which no agent may do on the
# owner's Mac.
#
#   .github/scripts/macos-launch.sh
#
# After the install line put the app in /Applications:
#   1. `open -a /Applications/YAAR.app`, which asks LaunchServices to start it, as a click in
#      the Dock or the Finder does; `open` itself returns at once.
#   2. Waits up to 30 s for a process whose command is
#      /Applications/YAAR.app/Contents/MacOS/yaar-review, and one whose command is
#      /Applications/YAAR.app/Contents/Helpers/yaar-keeper.
#   3. Asserts: the Window's parent is launchd (`ps -o ppid=` is 1); it has no controlling
#      terminal (`ps -o tty=` is "??"); the entry of `lsappinfo list` for the bundle
#      io.github.tsadoq.yaar says type="Foreground" (an app with a Dock icon, not a
#      background agent);
#      `/Applications/YAAR.app/Contents/MacOS/yaar-review --keeper-status` says
#      started=false and names the Keeper's pid above; `codesign --verify` of the running
#      Keeper's pid passes.
#   4. Ends the app with SIGTERM to the Window and waits for both processes to be gone (the
#      Keeper lingers 5 s; waits 20 s).
# On failure prints the processes of the user, their parents and terminals, and the last 50
# lines of `log show --last 2m --predicate 'process == "yaar-review"'`.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-launch.sh: not built yet" >&2
exit 99
