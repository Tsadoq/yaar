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
# What the header leaves: a window is looked for among the children of the root window
# (`xwininfo -root -children`), the places a window without a window manager has; Xvfb is
# started with -ac, because the app runs as another user than the one that started the
# server; the process of the app is found by /proc/<pid>/exe, read as the user (a container's
# root has no CAP_SYS_PTRACE and cannot read another user's), and a process that is a zombie
# nobody reaped counts as gone; `gio launch` is tried plain
# first and, when it fails, under dbus-run-session, which the step then says (it installs
# the package dbus only in that case, so that linux-login-path.sh finds the program
# dbus-run-session exactly when the start needed it); step 2 also fails on a line NoDisplay=true
# or Hidden=true, which hide the app from the menu that "from the app menu" is about; step 3
# fails, naming its pid, when a Keeper of <user> from an earlier step still runs after the
# 20 s it waits for it to leave, because the Keeper that step 4 then names could be that one
# and not the one the app's own start made; a command that stops the script without a sentence
# of its own is named in one, and the exit code is 1.
set -euo pipefail

NAME=linux-launch.sh
if [ $# -ne 1 ]; then
  echo "usage: $NAME <user>" >&2
  exit 2
fi
USER_NAME=$1
id -u "$USER_NAME" > /dev/null 2>&1 || {
  echo "$NAME: no user $USER_NAME on this system" >&2
  exit 2
}
HOME_DIR=$(getent passwd "$USER_NAME" | cut -d: -f6)
export DEBIAN_FRONTEND=noninteractive

DESKTOP=/usr/share/applications/io.github.tsadoq.yaar.desktop
PROGRAMS=(
  /usr/bin/yaar-review
  /usr/libexec/yaar-review/yaar-keeper
  /usr/libexec/yaar-review/yaar-claude-hook
  /usr/libexec/yaar-review/yaar-claude-tools
  /usr/libexec/yaar-review/yaar-pi-report
)
WINDOW_EXE=/usr/bin/yaar-review
KEEPER_EXE=/usr/libexec/yaar-review/yaar-keeper
MENU_PATH=/usr/local/bin:/usr/bin:/bin
CLASS=io.github.tsadoq.yaar

SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/yaar-launch.XXXXXX")
XVFB_PID=""
WINDOW_PID=""
KEEPER_PID=""
STEP=0
SAID=0

# True while the process is there and is not a zombie.
alive() {
  local state
  [ -n "$1" ] && [ -r "/proc/$1/stat" ] || return 1
  state=$(sed -n 's/^[0-9]* (.*) \(.\) .*/\1/p' "/proc/$1/stat" 2> /dev/null) || return 1
  case $state in
    '' | Z | X) return 1 ;;
  esac
  return 0
}

# The pids of the user's processes whose program is $1. Read as the user: a container's root
# has no CAP_SYS_PTRACE, and without it /proc/<pid>/exe of another user's process cannot be
# read, root or not.
pids_of() {
  local pid
  sudo -n -u "$USER_NAME" sh -c '
    for d in /proc/[0-9]*; do
      if [ "$(readlink "$d/exe" 2> /dev/null)" = "$1" ]; then echo "${d#/proc/}"; fi
    done' sh "$1" | while IFS= read -r pid; do
    if alive "$pid"; then echo "$pid"; fi
  done
}

# What a failure shows: the user's processes with their terminals and the windows.
diagnose() {
  echo "$NAME: the processes of $USER_NAME:" >&2
  ps -u "$USER_NAME" -o pid=,ppid=,tty=,stat=,args= >&2 || echo "  (ps failed)" >&2
  if [ -n "$XVFB_PID" ] && alive "$XVFB_PID"; then
    echo "$NAME: xwininfo -root -tree:" >&2
    DISPLAY=:99 xwininfo -root -tree >&2 2>&1 || echo "  (xwininfo failed)" >&2
  fi
  if [ -s "$SCRATCH/xvfb.log" ]; then
    echo "$NAME: Xvfb said:" >&2
    sed 's/^/  /' "$SCRATCH/xvfb.log" >&2
  fi
}

fail() {
  SAID=1
  echo "$NAME: step $STEP: $*" >&2
  diagnose
  exit 1
}

# A command that set -e stopped on says so in a sentence, with the code this script's header
# names, instead of leaving the failing tool's own status.
cleanup() {
  local status=$1 cmd=$2 pid
  if [ "$status" -ne 0 ] && [ "$SAID" -eq 0 ]; then
    echo "$NAME: step $STEP: '$cmd' failed with status $status" >&2
    diagnose
    status=1
  fi
  for pid in "$WINDOW_PID" "$KEEPER_PID"; do
    if alive "$pid" && [ "$status" -ne 0 ]; then kill "$pid" 2> /dev/null || true; fi
  done
  if [ -n "$XVFB_PID" ] && alive "$XVFB_PID"; then kill "$XVFB_PID" 2> /dev/null || true; fi
  rm -rf "$SCRATCH"
  exit "$status"
}
trap 'cleanup "$?" "$BASH_COMMAND"' EXIT

# Runs a command as the user with the environment a desktop's menu gives and nothing else.
as_menu() {
  sudo -n -u "$USER_NAME" env -i HOME="$HOME_DIR" USER="$USER_NAME" DISPLAY=:99 PATH="$MENU_PATH" "$@"
}

# ------------------------------------------------------------------------------ 1. libraries
STEP=1
for prog in "${PROGRAMS[@]}"; do
  [ -x "$prog" ] || fail "$prog is not installed or not executable"
  ldd "$prog" > "$SCRATCH/ldd" 2>&1 || fail "ldd $prog failed: $(cat "$SCRATCH/ldd")"
  if grep -q 'not found' "$SCRATCH/ldd"; then
    fail "$prog needs a library the package does not bring: $(grep 'not found' "$SCRATCH/ldd" | tr -s ' \t' ' ' | tr '\n' ';')"
  fi
done
echo "$NAME: 1. ldd of the five programs lists no 'not found'"
for pkg in libwayland-client0 libvulkan1; do
  [ "$(dpkg-query -W -f='${db:Status-Abbrev}' "$pkg" 2> /dev/null)" = "ii " ] || fail "the package $pkg is not installed: the .deb does not depend on it"
done
ls /usr/share/vulkan/icd.d/*.json > "$SCRATCH/icd" 2> /dev/null || fail "no /usr/share/vulkan/icd.d/*.json: no Vulkan driver came with the package"
echo "$NAME: 1. libwayland-client0 and libvulkan1 are installed; Vulkan drivers: $(tr '\n' ' ' < "$SCRATCH/icd")"
# The header names fc-match, which a user's system without the package fontconfig does not
# have (the .deb does not depend on it, and installing it here would hide a dependency): the
# font is looked for as the package that brings it lists it, and fc-match is asked too when it
# is there.
[ "$(dpkg-query -W -f='${db:Status-Abbrev}' fonts-dejavu-core 2> /dev/null)" = "ii " ] || fail "the package fonts-dejavu-core is not installed: the .deb does not depend on it"
FONT_FILE=$(dpkg -L fonts-dejavu-core | grep -m 1 '/DejaVuSans\.ttf$') || fail "fonts-dejavu-core lists no DejaVuSans.ttf"
[ -f "$FONT_FILE" ] || fail "$FONT_FILE, which fonts-dejavu-core lists, is not there"
if command -v fc-match > /dev/null 2>&1; then
  FONT=$(fc-match "DejaVu Sans")
  case $FONT in
    *"DejaVu Sans"*) ;;
    *) fail "fc-match 'DejaVu Sans' found: $FONT" ;;
  esac
  echo "$NAME: 1. fc-match finds $FONT"
else
  echo "$NAME: 1. fc-match is not on this system; DejaVu Sans is the file $FONT_FILE"
fi

# ------------------------------------------------------------------------------ 2. the entry
STEP=2
apt-get install -y --no-install-recommends desktop-file-utils > "$SCRATCH/apt-dfu" 2>&1 || fail "installing desktop-file-utils failed: $(tail -3 "$SCRATCH/apt-dfu")"
[ -f "$DESKTOP" ] || fail "$DESKTOP is not installed"
for line in Terminal=false Type=Application Exec=yaar-review; do
  grep -qx "$line" "$DESKTOP" || fail "$DESKTOP holds no line '$line': $(grep -E '^(Terminal|Type|Exec)=' "$DESKTOP" | tr '\n' ' ')"
done
# An entry the menu hides starts from `gio launch` all the same: the menu is what the row is about.
for line in NoDisplay=true Hidden=true; do
  if grep -qx "$line" "$DESKTOP"; then fail "$DESKTOP holds the line '$line': the app menu would not show the app"; fi
done
STATUS=0
desktop-file-validate "$DESKTOP" > "$SCRATCH/validate" 2>&1 || STATUS=$?
if [ "$STATUS" -ne 0 ] || [ -s "$SCRATCH/validate" ]; then
  fail "desktop-file-validate says (status $STATUS): $(tr '\n' ' ' < "$SCRATCH/validate")"
fi
EXEC=$(sed -n 's/^Exec=//p' "$DESKTOP")
EXEC=${EXEC%% *}
# shellcheck disable=SC2016 # the shell that runs it expands $1
RESOLVED=$(env -i PATH="$MENU_PATH" sh -c 'command -v "$1"' sh "$EXEC") || fail "Exec=$EXEC does not resolve on PATH=$MENU_PATH"
[ "$RESOLVED" = /usr/bin/yaar-review ] || fail "Exec=$EXEC resolves to $RESOLVED on PATH=$MENU_PATH, not to /usr/bin/yaar-review"
echo "$NAME: 2. $DESKTOP: Terminal=false, Type=Application, Exec=$EXEC, not hidden, valid, resolves to $RESOLVED"

# ------------------------------------------------------------------------------ 3. the start
STEP=3
apt-get install -y --no-install-recommends xvfb xauth x11-utils libglib2.0-bin > "$SCRATCH/apt-x" 2>&1 || fail "installing xvfb, xauth, x11-utils and libglib2.0-bin failed: $(tail -3 "$SCRATCH/apt-x")"
# A Keeper that an earlier step started lingers after its last Window; the one this start is
# about must be the app's own, so the old one is waited for first (5 s of linger, 20 s here)
# and one that stays fails the step.
DEADLINE=$(($(date +%s) + 20))
while :; do
  pids=$(pids_of "$KEEPER_EXE")
  [ -n "$pids" ] || break
  if [ "$(date +%s)" -ge "$DEADLINE" ]; then
    fail "a Keeper of $USER_NAME from an earlier step still runs after 20 s (pid ${pids%%$'\n'*}): the Keeper this start meets would not be the app's own"
  fi
  sleep 0.2
done
[ ! -e /tmp/.X11-unix/X99 ] || fail "/tmp/.X11-unix/X99 is there already: the display :99 is not free"
# Detached from the step, as serve-site.sh detaches its server.
if command -v setsid > /dev/null 2>&1; then
  setsid Xvfb :99 -ac -screen 0 1280x800x24 < /dev/null > "$SCRATCH/xvfb.log" 2>&1 &
else
  nohup Xvfb :99 -ac -screen 0 1280x800x24 < /dev/null > "$SCRATCH/xvfb.log" 2>&1 &
fi
XVFB_STARTED=$!
DEADLINE=$(($(date +%s) + 20))
until [ -S /tmp/.X11-unix/X99 ]; do
  kill -0 "$XVFB_STARTED" 2> /dev/null || fail "Xvfb ended before it made /tmp/.X11-unix/X99"
  [ "$(date +%s)" -lt "$DEADLINE" ] || fail "/tmp/.X11-unix/X99 did not appear within 20 s"
  sleep 0.2
done
# The server's own pid, which is the one the socket belongs to (setsid may have forked).
XVFB_PID=$(tr -d ' \n' < /tmp/.X99-lock) || fail "/tmp/.X99-lock cannot be read"
alive "$XVFB_PID" || fail "/tmp/.X99-lock names '$XVFB_PID', which is no running Xvfb"
echo "$NAME: 3. Xvfb :99 runs, pid $XVFB_PID"

DBUS=""
if ! as_menu gio launch "$DESKTOP" > "$SCRATCH/gio" 2>&1; then
  echo "$NAME: 3. gio launch refused: $(tr '\n' ' ' < "$SCRATCH/gio")"
  apt-get install -y --no-install-recommends dbus > "$SCRATCH/apt-dbus" 2>&1 || fail "installing dbus failed: $(tail -3 "$SCRATCH/apt-dbus")"
  DBUS=dbus-run-session
  as_menu dbus-run-session -- gio launch "$DESKTOP" > "$SCRATCH/gio" 2>&1 || fail "gio launch failed, also under dbus-run-session: $(tr '\n' ' ' < "$SCRATCH/gio")"
fi
echo "$NAME: 3. gio launch ${DBUS:+under $DBUS }returned: $(tr '\n' ' ' < "$SCRATCH/gio")"
# gio returns before the app has started: wait for the two processes.
DEADLINE=$(($(date +%s) + 30))
while :; do
  pids=$(pids_of "$WINDOW_EXE")
  WINDOW_PID=${pids%%$'\n'*}
  pids=$(pids_of "$KEEPER_EXE")
  KEEPER_PID=${pids%%$'\n'*}
  [ -n "$WINDOW_PID" ] && [ -n "$KEEPER_PID" ] && break
  [ "$(date +%s)" -lt "$DEADLINE" ] || fail "within 30 s no process of $USER_NAME ran $WINDOW_EXE (saw: '${WINDOW_PID}') and $KEEPER_EXE (saw: '${KEEPER_PID}')"
  sleep 0.2
done
echo "$NAME: 3. the Window runs, pid $WINDOW_PID, and the Keeper, pid $KEEPER_PID"

# ------------------------------------------------------------------------------ 4. the facts
STEP=4
# A top-level window of the class, mapped.
find_window() {
  local id
  DISPLAY=:99 xwininfo -root -children 2> /dev/null | sed -n 's/^ *\(0x[0-9a-f][0-9a-f]*\) .*/\1/p' > "$SCRATCH/ids" || return 1
  while IFS= read -r id; do
    if DISPLAY=:99 xprop -id "$id" WM_CLASS 2> /dev/null | grep -F "\"$CLASS\"" > /dev/null; then
      if DISPLAY=:99 xwininfo -id "$id" 2> /dev/null | grep 'Map State: IsViewable' > /dev/null; then
        echo "$id"
        return 0
      fi
    fi
  done < "$SCRATCH/ids"
  return 1
}
DEADLINE=$(($(date +%s) + 30))
WINDOW_ID=""
while :; do
  WINDOW_ID=$(find_window) && break
  alive "$WINDOW_PID" || fail "the Window, pid $WINDOW_PID, ended before it mapped a window of class $CLASS"
  [ "$(date +%s)" -lt "$DEADLINE" ] || fail "within 30 s no top-level window of class $CLASS was mapped (IsViewable)"
  sleep 0.2
done
echo "$NAME: 4. a window of class $CLASS is mapped: $WINDOW_ID"

as_menu yaar-review --keeper-status > "$SCRATCH/status" 2>&1 || fail "yaar-review --keeper-status failed: $(tr '\n' ' ' < "$SCRATCH/status")"
STATUS_LINE=$(cat "$SCRATCH/status")
echo "$NAME: 4. --keeper-status: $STATUS_LINE"
case $STATUS_LINE in
  *started=false*) ;;
  *) fail "--keeper-status does not say started=false: the app's own start did not start the Keeper: $STATUS_LINE" ;;
esac
STATUS_PID=$(printf '%s\n' "$STATUS_LINE" | tr ' ' '\n' | sed -n 's/^pid=//p')
[ "$STATUS_PID" = "$KEEPER_PID" ] || fail "--keeper-status names the Keeper pid '$STATUS_PID', not the pid $KEEPER_PID that step 3 saw"
[ "$(sudo -n -u "$USER_NAME" readlink "/proc/$STATUS_PID/exe")" = "$KEEPER_EXE" ] || fail "/proc/$STATUS_PID/exe is not $KEEPER_EXE"

alive "$WINDOW_PID" || fail "the Window, pid $WINDOW_PID, is gone after the Keeper answered"
[ "$(sudo -n -u "$USER_NAME" readlink "/proc/$WINDOW_PID/exe")" = "$WINDOW_EXE" ] || fail "/proc/$WINDOW_PID/exe is not $WINDOW_EXE"
STILL=$(find_window) || fail "the window of class $CLASS is no longer listed after the Keeper answered"
echo "$NAME: 4. after the Keeper answered the Window is alive and its window is still listed: $STILL"
# Recorded, not asserted.
echo "$NAME: 4. recorded: tty of the Window '$(ps -o tty= -p "$WINDOW_PID" | tr -d ' ')'; TERM in its environment: $(if sudo -n -u "$USER_NAME" cat "/proc/$WINDOW_PID/environ" | tr '\0' '\n' | grep '^TERM=' > /dev/null; then echo yes; else echo no; fi)"

# ------------------------------------------------------------------------------ 5. the end
STEP=5
kill -TERM "$WINDOW_PID"
DEADLINE=$(($(date +%s) + 10))
while alive "$WINDOW_PID"; do
  [ "$(date +%s)" -lt "$DEADLINE" ] || fail "the Window, pid $WINDOW_PID, still runs 10 s after SIGTERM"
  sleep 0.2
done
echo "$NAME: 5. the Window ended"
DEADLINE=$(($(date +%s) + 20))
while alive "$KEEPER_PID"; do
  [ "$(date +%s)" -lt "$DEADLINE" ] || fail "the Keeper, pid $KEEPER_PID, still runs 20 s after the last Window ended"
  sleep 0.2
done
echo "$NAME: 5. the Keeper left by itself"
kill "$XVFB_PID"
DEADLINE=$(($(date +%s) + 10))
while alive "$XVFB_PID"; do
  [ "$(date +%s)" -lt "$DEADLINE" ] || fail "Xvfb, pid $XVFB_PID, still runs 10 s after SIGTERM"
  sleep 0.2
done
XVFB_PID=""
echo "$NAME: the app started from its desktop entry with no terminal, and ended"
