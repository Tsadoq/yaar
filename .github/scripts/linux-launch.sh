#!/bin/bash
# Starts the installed app from its desktop entry on a virtual display: linux-launch.sh <user>.
# Xvfb -ac: the app runs as another user. `gio launch` is retried under dbus-run-session when GLib
# has no session bus; dbus is installed only then, which linux-login-path.sh relies on.
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

# The pids of the user's processes whose program is $1. Read as the user: a container's root has
# no CAP_SYS_PTRACE and cannot read another user's /proc/<pid>/exe.
pids_of() {
  local pid
  sudo -n -u "$USER_NAME" sh -c '
    for d in /proc/[0-9]*; do
      if [ "$(readlink "$d/exe" 2> /dev/null)" = "$1" ]; then echo "${d#/proc/}"; fi
    done' sh "$1" | while IFS= read -r pid; do
    if alive "$pid"; then echo "$pid"; fi
  done
}

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

as_menu() {
  sudo -n -u "$USER_NAME" env -i HOME="$HOME_DIR" USER="$USER_NAME" DISPLAY=:99 PATH="$MENU_PATH" "$@"
}

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
# fc-match is not required (installing fontconfig would hide a missing dependency): the font is
# looked up in the package's file list.
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

STEP=2
apt-get install -y --no-install-recommends desktop-file-utils > "$SCRATCH/apt-dfu" 2>&1 || fail "installing desktop-file-utils failed: $(tail -3 "$SCRATCH/apt-dfu")"
[ -f "$DESKTOP" ] || fail "$DESKTOP is not installed"
for line in Terminal=false Type=Application Exec=yaar-review; do
  grep -qx "$line" "$DESKTOP" || fail "$DESKTOP holds no line '$line': $(grep -E '^(Terminal|Type|Exec)=' "$DESKTOP" | tr '\n' ' ')"
done
# Read as GLib reads a key file: group [Desktop Entry] only, blanks around key and '=', booleans
# "true" or "1".
HIDES=$(awk '
  /^[ \t]*\[/ { group = ($0 ~ /^\[Desktop Entry\][ \t]*$/); next }
  !group { next }
  {
    eq = index($0, "=")
    if (eq == 0) next
    key = substr($0, 1, eq - 1); value = substr($0, eq + 1)
    gsub(/^[ \t]+/, "", key); gsub(/[ \t]+$/, "", key)
    gsub(/^[ \t]+/, "", value); gsub(/[ \t]+$/, "", value)
    if ((key == "NoDisplay" || key == "Hidden") && (value == "true" || value == "1")) print $0
    else if ((key == "OnlyShowIn" || key == "NotShowIn") && value != "") print $0
  }' "$DESKTOP") || fail "awk could not read $DESKTOP"
if [ -n "$HIDES" ]; then
  fail "$DESKTOP holds a line that hides the app from a menu: $(printf '%s' "$HIDES" | tr '\n' ';')"
fi
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

STEP=3
apt-get install -y --no-install-recommends xvfb xauth x11-utils libglib2.0-bin > "$SCRATCH/apt-x" 2>&1 || fail "installing xvfb, xauth, x11-utils and libglib2.0-bin failed: $(tail -3 "$SCRATCH/apt-x")"
# An earlier step's Keeper lingers 5 s after its last Window: wait for it, so the Keeper met is the
# app's own.
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

STEP=4
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
echo "$NAME: 4. recorded: tty of the Window '$(ps -o tty= -p "$WINDOW_PID" | tr -d ' ')'; TERM in its environment: $(if sudo -n -u "$USER_NAME" cat "/proc/$WINDOW_PID/environ" | tr '\0' '\n' | grep '^TERM=' > /dev/null; then echo yes; else echo no; fi)"

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
