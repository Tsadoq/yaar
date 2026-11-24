#!/bin/bash
# The checks on a Linux machine with a graphics driver: HW-01, HW-02, HW-03, SYS-02 and
# SYS-03.
#
#   packaging/checklists/linux-hardware.sh [--server http://127.0.0.1:<port>] [--dry-run]
#
# Run it on the Linux machine (x86-64, a hardware graphics driver). The virtual machine draws
# in software and gives no speed figure.
#
# What it does:
#   1. With --server, installs the release from that server: the install line that
#      serve-release.sh prints, run here (start serve-release.sh on this machine, on a copy
#      of the folder of release-final, and give its address). The address must be
#      127.0.0.1 or localhost. apt asks for sudo's password on the terminal; the script asks
#      nothing. SYS-02 is this install on the machine you have; its other releases are the
#      `install-linux` jobs.
#   2. Records the system: the release, the kernel, the session type, the desktop, the
#      graphics line of `lspci` when there is one.
#   3. Runs `memory.sh --linux` (HW-03's memory, 60 s idle): it asks you to open the app on
#      one workspace and leave it idle, then waits for you to press Return. Its last line goes
#      into the header of the results file.
#   4. Prints HW-01, HW-02, HW-03 (typing, scrolling and the flood by hand: nothing here posts
#      keys or events on Linux, and the measuring kit of phase 4 is for macOS), SYS-02 and
#      SYS-03 one at a time and asks "yes / no / note / skip" (the answers can come from a
#      file, see lib.sh). A "no" on HW-01 is a decision for the owner: the promise of X11 on
#      the two releases whose software driver leaves the window empty returns to him.
#
# Typing against foot, HW-03: Ghostty has no official build for Ubuntu 24.04, so the
# reference is the terminal foot; that is the owner's to accept. By hand: open YAAR's terminal
# pane and foot side by side, type the same text in each, record the screen with a tool of
# yours that stamps frames, and compare: at most 10 ms later than foot, at the median and the
# 95th percentile.
#
# --dry-run: runs no install, no memory.sh and no program of the system or the app, and
# writes linux-hardware-dry-run.md instead of linux-hardware.md.
#
# What it needs of the owner: a Linux machine with a hardware driver, sudo for the install, a
# graphical session, the app installed (or --server), `foot` for the typing check, and the
# system with no Vulkan driver for SYS-03 (a second boot or a container-less test machine:
# `VK_ICD_FILENAMES=/nonexistent yaar-review` is a way to take the driver away).
#
# Where it writes: ~/yaar-checklists/<YYYY-MM-DD>/linux-hardware.md (date, version, system,
# the memory line, one row a check, the sign-off) and memory-linux-<HHMMSS>.txt.
#
# Exit codes: 0 the walk is written (whatever its answers); 1 it could not be walked or
# written (a sentence says which); 2 usage error.
set -euo pipefail

CK_NAME=linux-hardware.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

DRY=0
SERVER=""
while [ "$#" -gt 0 ]; do
  case $1 in
    -h | --help)
      ck_help "$0"
      exit 0
      ;;
    --dry-run)
      DRY=1
      shift
      ;;
    --server)
      [ "$#" -ge 2 ] || ck_usage_error "--server needs an address"
      SERVER=$2
      shift 2
      case $SERVER in
        http://127.0.0.1:[0-9]* | http://localhost:[0-9]*) ;;
        *) ck_usage_error "--server takes the address serve-release.sh printed, http://127.0.0.1:<port> or http://localhost:<port>, not '$SERVER'" ;;
      esac
      SERVER=${SERVER%/}
      ;;
    *) ck_usage_error "unknown argument '$1'" ;;
  esac
done

ck_begin
if [ "$DRY" -eq 0 ]; then
  [ "$(ck_os)" = Linux ] || ck_die "this is $(ck_os), not Linux: run it on the Linux machine"
  if [ -n "$SERVER" ]; then
    printf '%s: installing from %s\n' "$CK_NAME" "$SERVER"
    curl -fsSL "$SERVER/install.sh" | YAAR_BASE_URL="$SERVER" YAAR_DOWNLOAD_URL="$SERVER/download" bash \
      || ck_die "the install line failed against $SERVER (what it said is above)"
  fi
fi
ck_walker_context "$DRY"

if [ "$DRY" -eq 1 ]; then
  ck_context "Graphics" "not asked: dry run"
  ck_context "Memory (HW-03)" "not run: dry run"
else
  KERNEL=$(uname -sr)
  GRAPHICS=$(ck_graphics_line)
  ck_context "Kernel" "$KERNEL"
  ck_context "Graphics" "$GRAPHICS"
  printf 'Open the app on one workspace and leave it idle. Press Return to measure its memory for 60 s: '
  ck_read_line "the memory measure"
  MEMORY_STATUS=0
  "$HERE/memory.sh" --linux | tee "$CK_WORK/memory.txt" || MEMORY_STATUS=$?
  ck_context "Memory (HW-03)" "$(tail -n 1 "$CK_WORK/memory.txt") (status $MEMORY_STATUS)"
fi

items_linux_hardware() {
  ck_item HW-01 "The window draws on X11 with a hardware driver" \
    "Under X11, on Ubuntu 22.04 and on Debian 12 (two sessions, or two machines): start the app." \
    "The window draws. With the software driver of those two releases it stays empty, which is why this check needs the hardware one."
  ck_item HW-02 "The terminal under Wayland and under X11" \
    "In a terminal pane, under Wayland and then under X11: type, use a dead key, a compose sequence and the mouse (select, scroll, a click in an application that takes the mouse)." \
    "Keys, dead keys, a compose sequence and the mouse work as in a native terminal."
  ck_item "HW-03 typing" "Typing against foot" \
    "Open the terminal pane and foot side by side and compare a key's time on screen as described in this script's header." \
    "At most 10 ms later than foot at the median and the 95th percentile. The reference is foot, not Ghostty: accept that or say no."
  ck_item "HW-03 scrolling" "Scrolling a diff of 2,200 lines" \
    "Open a diff of 2,200 lines, unified and then split, and scroll it with the mouse wheel and with the keys for several seconds. Count frames with the desktop's frame counter or a capture of yours." \
    "Frames at the refresh rate of the display, and at most 1% of the frames longer than two refresh intervals."
  ck_item "HW-03 flood" "A flood of 200,000 lines" \
    "Time this writer alone, in a plain terminal with its output to /dev/null, and then in the app's terminal pane with its output on screen: perl -e '\$|=1; for \$i (1..200000) { printf(\"\\e[3%dm%06d\\e[0m the quick brown fox jumps over the lazy dog\\n\", 1+\$i%6, \$i) }'" \
    "In the pane the flood takes at most 1.5 times what the writing program needs alone."
  ck_item "HW-03 memory" "Memory" \
    "Read the memory line in this file's header (the measure this script ran)." \
    "At most 200 MB for the app's own processes."
  ck_item SYS-02 "The package installs and draws" \
    "The install above is this machine's. For the five promised releases, the jobs install-linux install the package on each; here, on the machine you have, start the app after the install." \
    "It installs from the apt repository and the window draws."
  ck_item SYS-03 "No Vulkan driver" \
    "On a system with no Vulkan driver (or with the driver taken away), start the app." \
    "The app draws and says, in a line in the window, that it runs in software."
}

printf 'The Linux machine checks. Open the app yourself, look, and answer here.\n'
ck_walk items_linux_hardware
ck_signoff "I accept these checks as passed on this Linux machine"

RESULTS=$(ck_results_dir)
if [ "$DRY" -eq 1 ]; then
  OUT="$RESULTS/linux-hardware-dry-run.md"
else
  OUT="$RESULTS/linux-hardware.md"
fi
ck_write "$OUT" "HW-01 to HW-03, SYS-02 and SYS-03: Linux on real hardware" "$CK_SIGN"
printf '\nWritten to %s\n' "$OUT"
