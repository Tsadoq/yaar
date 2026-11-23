#!/bin/bash
# Row PKG-08 on Linux: the installed app starts from the app menu with no terminal, and the
# desktop entry asks for no terminal.
#
#   .github/scripts/linux-launch.sh <user>
#
# Run as root after the install line installed the package. In this order:
#   1. Libraries, before anything else is installed, so that nothing the test adds can hide
#      a dependency the package lacks: each of the five installed programs' `ldd` lists no
#      "not found"; the packages libwayland-client0 and libvulkan1 are installed; a file
#      /usr/share/vulkan/icd.d/*.json exists; fc-match finds DejaVu Sans.
#   2. The entry: /usr/share/applications/io.github.tsadoq.yaar.desktop holds Terminal=false,
#      Type=Application, Exec=yaar-review, and `desktop-file-validate` (package
#      desktop-file-utils, installed now) reports nothing; Exec's program resolves to
#      /usr/bin/yaar-review in the system's search path
#      (`env -i PATH=/usr/local/bin:/usr/bin:/bin command -v yaar-review`).
#   3. The start: installs xvfb, xauth, x11-utils and libglib2.0-bin; starts `Xvfb :99` in the
#      background, detached as serve-site.sh detaches its server, and waits for
#      /tmp/.X11-unix/X99; as <user>, with an environment that holds HOME, USER, DISPLAY=:99
#      and the PATH above and nothing else (env -i), runs `gio launch <the desktop entry>`
#      (if GLib refuses for want of a session bus: the same under `dbus-run-session`, package
#      dbus), as a desktop's menu starts an app; waits up to 30 s until a process of <user>
#      whose /proc/<pid>/exe is /usr/bin/yaar-review runs, and one whose exe is
#      /usr/libexec/yaar-review/yaar-keeper.
#   4. Asserts, each a fact that a dead or windowless app cannot give:
#      - within 30 s, `xwininfo -root -tree` (DISPLAY=:99) lists a top-level window whose
#        `xprop WM_CLASS` holds "io.github.tsadoq.yaar", and `xwininfo -id <it>` says "Map
#        State: IsViewable" (the app mapped its window);
#      - `yaar-review --keeper-status` as <user> (same env -i environment) says started=false
#        (the app's own start started the Keeper) and names the Keeper's pid of step 3, whose
#        /proc/<pid>/exe is /usr/libexec/yaar-review/yaar-keeper;
#      - after that, the Window's pid of step 3 is still alive (`kill -0`), and the window of
#        the first point is still listed.
#      Recorded, not asserted (in a container's step no process has a terminal and env -i
#      sets no TERM, so these hold however the app was started): `ps -o tty= -p <pid>` and
#      whether /proc/<pid>/environ holds TERM.
#   5. Ends the app: SIGTERM to the Window's pid, waits for it to end (10 s), then for the
#      Keeper to leave by itself (it lingers 5 s; waits 20 s), then stops Xvfb by its pid.
# Prints what it saw at each step; on failure, the processes of <user>, their terminals and
# `xwininfo -root -tree`.
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "linux-launch.sh: not built yet" >&2
exit 99
