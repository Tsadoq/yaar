#!/bin/bash
# PERF-01 and PERF-02: the time from the start of the packaged app to the diff of a
# workspace on screen, against 0.3 s.
#
#   packaging/checklists/start-time.sh [--runs N]            macOS: the app in /Applications
#   packaging/checklists/start-time.sh --linux [--runs N]    Linux: the desktop entry's Exec
#   packaging/checklists/start-time.sh --read <file> <t0 in ms since 1970>
#
# What it measures: with the trace variable YAAR_TRACE_START set, the app writes one line a
# mark on its standard error, `yaar-trace <name> epoch_ms=<ms>` (crates/window/frame/src/
# trace.rs): `main` when the Window program starts, `first_frame` when the Frame draws its
# first frame, `first_diff` the frame after the review pane first draws a diff with rows.
# This script notes the clock just before it asks the system to start the app and prints each
# mark as milliseconds after that. The figure of PERF-01 and PERF-02 is `first_diff` of the
# FIRST start, against 300 ms. The time of the call itself (`open`, or the start of the
# program) is inside it, as it is for a user.
#
# macOS: `open -g -n --env YAAR_TRACE_START=1 --stderr <file> /Applications/YAAR.app` (an app
# started by `open` has its standard error where `open --stderr` sends it, and nowhere
# else). The app reopens the workspace it was last left on, so open the app once before, on a
# workspace that has a diff, and quit it. Then, for a cold start: restart the Mac, log in,
# wait two minutes, and run this once (RUNS defaults to 1; later starts of one run are warm
# and are printed for comparison only). The script refuses to start while YAAR is already
# running, ends the app it started by SIGTERM to the pid of its Window program (matched by
# full path), and waits for the Keeper to leave (it lingers 5 s) before a next start. It
# posts no input.
#
# --linux: runs the Exec line of the package's desktop entry
# (/usr/share/applications/io.github.tsadoq.yaar.desktop) with YAAR_TRACE_START=1 and its
# standard error in a file, detached, and ends it by its pid. It needs a graphical session:
# DISPLAY or WAYLAND_DISPLAY in the environment, or named in YAAR_SESSION_ENV as words
# `NAME=value` (for example "WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000"), which
# are passed to the app. Run on a Linux machine it measures that machine. Run on the Mac it
# measures the project's virtual machine: it copies this folder to the machine, runs
# `start-time.sh --linux` there through the machine's own ssh (YAAR_VM, default
# ~/ade-phase4-2026-11-12/vm/vm.sh; the machine must be running: start it yourself), and
# copies the results folder back. PERF-02 is "after the machine's own restart, a cold start,
# the .deb installed there".
#
# --read <file> <t0>: prints the marks of a file that an earlier start wrote, as
# milliseconds after <t0>, and starts nothing. This is the reading the other modes use.
#
# What it needs of the owner: the packaged app installed, a workspace with a diff left open
# in it, the screen unlocked, mains power, nobody at the keyboard. The older measure it
# follows is ~/yaar-phase5-2026-11-17/cold-start/run.sh, which timed the first frame of the
# empty window; this one times the diff.
#
# Where it writes: ~/yaar-checklists/<YYYY-MM-DD>/start-time-<macos|linux>-<HHMMSS>/ with
# context.txt (date, system, uptime, power, load, the app's sha256), start-<n>.err (the app's
# standard error), starts.tsv (run, main_ms, first_frame_ms, first_diff_ms) and verdict.txt.
#
# Exit codes: 0 the first start's first_diff is within 300 ms; 1 it is over, or a mark is
# missing, or a start failed (a sentence says which); 2 usage error.
set -euo pipefail

CK_NAME=start-time.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

LIMIT_MS=300
RUNS=1
LINUX=0
READ_FILE=""
READ_T0=""
while [ "$#" -gt 0 ]; do
  case $1 in
    -h | --help)
      ck_help "$0"
      exit 0
      ;;
    --linux)
      LINUX=1
      shift
      ;;
    --runs)
      [ "$#" -ge 2 ] || ck_usage_error "--runs needs a number"
      RUNS=$2
      shift 2
      case $RUNS in
        '' | *[!0-9]* | 0) ck_usage_error "--runs needs a number above 0, not '$RUNS'" ;;
      esac
      ;;
    --read)
      [ "$#" -ge 3 ] || ck_usage_error "--read needs a file and a time"
      READ_FILE=$2
      READ_T0=$3
      shift 3
      ;;
    *) ck_usage_error "unknown argument '$1'" ;;
  esac
done

# row <file> <t0>: "main_ms first_frame_ms first_diff_ms", "none" for a mark not there.
row() {
  local name ms out=""
  for name in main first_frame first_diff; do
    ms=$(ck_mark_ms "$1" "$name")
    if [ -n "$ms" ]; then
      out="$out${out:+ }$((ms - $2))"
    else
      out="$out${out:+ }none"
    fi
  done
  printf '%s\n' "$out"
}

if [ -n "$READ_FILE" ]; then
  [ -f "$READ_FILE" ] || ck_die "$READ_FILE is not a file"
  case $READ_T0 in
    '' | *[!0-9]*) ck_usage_error "the time after --read is milliseconds since 1970, not '$READ_T0'" ;;
  esac
  printf 'main_ms first_frame_ms first_diff_ms\n%s\n' "$(row "$READ_FILE" "$READ_T0")"
  exit 0
fi

# ---------------------------------------------------------------------------------------
# macOS, run from the Mac, into the virtual machine
# ---------------------------------------------------------------------------------------

if [ "$LINUX" -eq 1 ] && [ "$(ck_os)" = Darwin ]; then
  VM=${YAAR_VM:-$HOME/ade-phase4-2026-11-12/vm/vm.sh}
  [ -x "$VM" ] || ck_die "no virtual machine script at $VM: name it in YAAR_VM"
  VM_STATE=$("$VM" status) || ck_die "$VM status failed"
  case $VM_STATE in
    running*) ;;
    *) ck_die "the virtual machine is not running ($VM_STATE): start it with '$VM start', restart it for a cold start, and run this again" ;;
  esac
  "$VM" ssh 'rm -rf "$HOME/yaar-checklists-kit"' || ck_die "could not clear yaar-checklists-kit in the virtual machine"
  "$VM" scp-to "$HERE" yaar-checklists-kit > /dev/null || ck_die "could not copy $HERE to the virtual machine"
  REMOTE_LOG=$(mktemp "${TMPDIR:-/tmp}/yaar-start-time.XXXXXX") || ck_die "cannot make a scratch file"
  trap 'rm -f "$REMOTE_LOG"' EXIT
  # The session words are quoted for the remote shell with single quotes.
  SESSION=${YAAR_SESSION_ENV:-}
  case $SESSION in
    *"'"*) ck_die "YAAR_SESSION_ENV holds a single quote, which cannot be passed on" ;;
  esac
  if "$VM" ssh "YAAR_SESSION_ENV='$SESSION' bash \"\$HOME/yaar-checklists-kit/start-time.sh\" --linux --runs $RUNS" | tee "$REMOTE_LOG"; then
    REMOTE_STATUS=0
  else
    REMOTE_STATUS=$?
  fi
  REMOTE_DIR=$(sed -n 's/^start-time.sh: results in //p' "$REMOTE_LOG" | tail -n 1)
  [ -n "$REMOTE_DIR" ] || ck_die "the virtual machine's run said nothing of where it wrote its results (status $REMOTE_STATUS, its output is above)"
  LOCAL=$(ck_results_dir)
  "$VM" scp-from "$REMOTE_DIR" "$LOCAL/vm-${REMOTE_DIR##*/}" > /dev/null || ck_die "could not copy $REMOTE_DIR back from the virtual machine"
  printf '%s: copied to %s\n' "$CK_NAME" "$LOCAL/vm-${REMOTE_DIR##*/}"
  exit "$REMOTE_STATUS"
fi

# ---------------------------------------------------------------------------------------
# One system: start, read, end
# ---------------------------------------------------------------------------------------

if [ "$LINUX" -eq 1 ]; then
  KIND=linux
  ENTRY=${YAAR_DESKTOP_ENTRY:-/usr/share/applications/io.github.tsadoq.yaar.desktop}
  [ -r "$ENTRY" ] || ck_die "$ENTRY cannot be read: install the package, or name the desktop entry in YAAR_DESKTOP_ENTRY"
  EXEC_LINE=$(sed -n 's/^Exec=//p' "$ENTRY" | head -n 1)
  [ -n "$EXEC_LINE" ] || ck_die "$ENTRY has no Exec line"
  read -r -a EXEC_WORDS <<< "$EXEC_LINE"
  # Field codes (%U, %f, ...) stand for files the launcher passes: none here.
  EXEC_CLEAN=()
  for word in "${EXEC_WORDS[@]}"; do
    case $word in
      %*) ;;
      *) EXEC_CLEAN+=("$word") ;;
    esac
  done
  [ "${#EXEC_CLEAN[@]}" -gt 0 ] || ck_die "$ENTRY has an Exec line with no program"
  read -r -a SESSION_WORDS <<< "${YAAR_SESSION_ENV:-}"
  if [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    case " ${YAAR_SESSION_ENV:-} " in
      *" DISPLAY="* | *" WAYLAND_DISPLAY="*) ;;
      *) ck_die "there is no graphical session: DISPLAY and WAYLAND_DISPLAY are not set, and YAAR_SESSION_ENV does not name one" ;;
    esac
  fi
  WINDOW_PROGRAM=$(ck_window_program)
else
  KIND=macos
  [ "$(ck_os)" = Darwin ] || ck_die "this is not a Mac: run it with --linux"
  APP=$(ck_app_bundle)
  WINDOW_PROGRAM=$(ck_window_program)
  [ -x "$WINDOW_PROGRAM" ] || ck_die "no app at $APP: install it first, or name it in YAAR_APP"
fi
KEEPER_PROGRAM=$(ck_keeper_program)

running() {
  [ -n "$(ck_pids_of "$WINDOW_PROGRAM")$(ck_pids_of "$KEEPER_PROGRAM")" ]
}
if running; then
  ck_die "YAAR is already running (a program at $WINDOW_PROGRAM or $KEEPER_PROGRAM): quit it first, a start with it running is not a cold start"
fi

RESULTS=$(ck_results_dir)
OUT="$RESULTS/start-time-$KIND-$(date +%H%M%S)"
mkdir "$OUT" || ck_die "cannot make $OUT"

if [ "$KIND" = macos ]; then
  BOOT=$(sysctl -n kern.boottime | sed -E 's/^\{ sec = ([0-9]+),.*/\1/')
  UP=$(($(date +%s) - BOOT))
  POWER=$(pmset -g batt | head -n 1)
  LOAD=$(sysctl -n vm.loadavg)
  SYSTEM="macOS $(sw_vers -productVersion) $(sysctl -n machdep.cpu.brand_string)"
  SHA=$(shasum -a 256 "$WINDOW_PROGRAM" | awk '{ print $1 }')
else
  UP=$(awk '{ printf "%d", $1 }' /proc/uptime)
  POWER="not asked"
  LOAD=$(cut -d' ' -f1-3 /proc/loadavg)
  SYSTEM=$(sed -n 's/^PRETTY_NAME="\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' /etc/os-release | head -n 1)
  SHA=$(sha256sum "$WINDOW_PROGRAM" | awk '{ print $1 }')
fi
{
  echo "date=$(date '+%Y-%m-%d %H:%M:%S')"
  echo "system=$SYSTEM"
  echo "seconds_since_boot=$UP"
  echo "power=$POWER"
  echo "load=$LOAD"
  echo "program=$WINDOW_PROGRAM"
  echo "program_sha256=$SHA"
  echo "runs=$RUNS"
} > "$OUT/context.txt"
cat "$OUT/context.txt"
if [ "$UP" -lt 120 ]; then
  printf '%s: warning: the system started %s s ago; wait two minutes after logging in, or this is not the cold start\n' "$CK_NAME" "$UP" >&2
fi

printf 'run\tmain_ms\tfirst_frame_ms\tfirst_diff_ms\n' > "$OUT/starts.tsv"
PID=""
stop_app() {
  local pid
  if [ "$KIND" = macos ]; then
    for pid in $(ck_pids_of "$WINDOW_PROGRAM"); do
      if kill -0 "$pid" 2> /dev/null; then
        kill -TERM "$pid"
      fi
    done
  elif [ -n "$PID" ] && kill -0 "$PID" 2> /dev/null; then
    kill -TERM "$PID"
  fi
  if ! ck_wait_until 20 not_running; then
    ck_die "the app did not leave in 20 s after SIGTERM: a program at $WINDOW_PROGRAM or $KEEPER_PROGRAM is still running"
  fi
}
not_running() {
  ! running
}
trap 'stop_app' EXIT

RUN=1
FIRST_DIFF=""
while [ "$RUN" -le "$RUNS" ]; do
  ERR="$OUT/start-$RUN.err"
  : > "$ERR"
  T0=$(ck_now_ms)
  if [ "$KIND" = macos ]; then
    open -g -n --env YAAR_TRACE_START=1 --stderr "$ERR" "$APP"
  else
    if command -v setsid > /dev/null; then
      setsid env YAAR_TRACE_START=1 ${SESSION_WORDS[@]+"${SESSION_WORDS[@]}"} "${EXEC_CLEAN[@]}" > "$OUT/start-$RUN.out" 2> "$ERR" < /dev/null &
    else
      nohup env YAAR_TRACE_START=1 ${SESSION_WORDS[@]+"${SESSION_WORDS[@]}"} "${EXEC_CLEAN[@]}" > "$OUT/start-$RUN.out" 2> "$ERR" < /dev/null &
    fi
    PID=$!
  fi
  have_diff() {
    [ -n "$(ck_mark_ms "$ERR" first_diff)" ]
  }
  if ! ck_wait_until 30 have_diff; then
    printf '%s: no first_diff mark in 30 s; the marks seen: %s\n' "$CK_NAME" "$(grep -c '^yaar-trace ' "$ERR") lines, $(sed -n 's/^yaar-trace \([a-z_]*\) .*/\1/p' "$ERR" | tr '\n' ' ')" >&2
  fi
  VALUES=$(row "$ERR" "$T0")
  printf '%s\t%s\n' "$RUN" "$(printf '%s' "$VALUES" | tr ' ' '\t')" | tee -a "$OUT/starts.tsv"
  if [ "$RUN" -eq 1 ]; then
    FIRST_DIFF=${VALUES##* }
  fi
  stop_app
  RUN=$((RUN + 1))
done
trap - EXIT

if [ "$FIRST_DIFF" = none ]; then
  VERDICT="PERF-01/02: no first_diff mark in the first start: the workspace the app reopens drew no diff, or the app did not start (see $OUT/start-1.err)"
  STATUS=1
elif [ "$FIRST_DIFF" -le "$LIMIT_MS" ]; then
  VERDICT="first start: first_diff at $FIRST_DIFF ms, within $LIMIT_MS ms"
  STATUS=0
else
  VERDICT="first start: first_diff at $FIRST_DIFF ms, over $LIMIT_MS ms"
  STATUS=1
fi
printf '%s\n' "$VERDICT" | tee "$OUT/verdict.txt"
printf '%s: results in %s\n' "$CK_NAME" "$OUT"
exit "$STATUS"
