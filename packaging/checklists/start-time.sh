#!/bin/bash
# Time from starting the packaged app to the diff of a workspace on screen, against 0.3 s.
#
# packaging/checklists/start-time.sh [--runs N]            macOS: the app in /Applications
# packaging/checklists/start-time.sh --linux [--runs N]    Linux: the desktop entry's Exec
# packaging/checklists/start-time.sh --read <file> <t0 in ms since 1970>
#
# The figure is `first_diff` of the FIRST start, from the marks the app writes on stderr when
# YAAR_TRACE_START is set. The app reopens its last workspace: open it once on a diff and quit.
# For a cold start restart the Mac, log in, wait two minutes, run once. --linux needs DISPLAY or
# WAYLAND_DISPLAY, or YAAR_SESSION_ENV as `NAME=value` words; on the Mac it runs in the virtual
# machine that the program YAAR_VM names drives. Exit: 0 within 300 ms, 1 over, a mark missing or
# a start failed, 2 usage.
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

if [ "$LINUX" -eq 1 ] && [ "$(ck_os)" = Darwin ]; then
  VM=${YAAR_VM:-}
  [ -n "$VM" ] || ck_die "set YAAR_VM to the program that drives the Linux virtual machine: it takes status, start, ssh, scp-to and scp-from"
  [ -x "$VM" ] || ck_die "no virtual machine program at $VM: name it in YAAR_VM"
  VM_STATE=$("$VM" status) || ck_die "$VM status failed"
  case $VM_STATE in
    running*) ;;
    *) ck_die "the virtual machine is not running ($VM_STATE): start it with '$VM start', restart it for a cold start, and run this again" ;;
  esac
  # shellcheck disable=SC2016 # the remote shell expands $HOME
  "$VM" ssh 'rm -rf "$HOME/yaar-checklists-scripts"' || ck_die "could not clear yaar-checklists-scripts in the virtual machine"
  "$VM" scp-to "$HERE" yaar-checklists-scripts > /dev/null || ck_die "could not copy $HERE to the virtual machine"
  ck_tmpdir
  REMOTE_LOG="$CK_TMPDIR/remote.log"
  SESSION=${YAAR_SESSION_ENV:-}
  case $SESSION in
    *"'"*) ck_die "YAAR_SESSION_ENV holds a single quote, which cannot be passed on" ;;
  esac
  if "$VM" ssh "YAAR_SESSION_ENV='$SESSION' bash \"\$HOME/yaar-checklists-scripts/start-time.sh\" --linux --runs $RUNS" | tee "$REMOTE_LOG"; then
    REMOTE_STATUS=0
  else
    REMOTE_STATUS=$?
  fi
  REMOTE_DIR=$(sed -n 's/^start-time.sh: results in //p' "$REMOTE_LOG" | tail -n 1)
  case $REMOTE_STATUS in
    0 | 1) ;;
    *) printf '%s: the run in the virtual machine ended with status %s, which this script does not use (255 is ssh failing): its output is above\n' "$CK_NAME" "$REMOTE_STATUS" >&2
       REMOTE_STATUS=1 ;;
  esac
  [ -n "$REMOTE_DIR" ] || ck_die "the virtual machine's run said nothing of where it wrote its results (status $REMOTE_STATUS, its output is above)"
  LOCAL=$(ck_results_dir)
  "$VM" scp-from "$REMOTE_DIR" "$LOCAL/vm-${REMOTE_DIR##*/}" > /dev/null || ck_die "could not copy $REMOTE_DIR back from the virtual machine"
  printf '%s: copied to %s\n' "$CK_NAME" "$LOCAL/vm-${REMOTE_DIR##*/}"
  exit "$REMOTE_STATUS"
fi

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
  # The Keeper lingers 5 s after the Window is gone.
  if ! ck_wait_until 20 not_running; then
    ck_die "the app did not leave in 20 s after SIGTERM: a program at $WINDOW_PROGRAM or $KEEPER_PROGRAM is still running"
  fi
}
not_running() {
  ! running
}
CK_EXTRA_CLEANUP=stop_app

RUN=1
FIRST_DIFF=""
while [ "$RUN" -le "$RUNS" ]; do
  ERR="$OUT/start-$RUN.err"
  : > "$ERR"
  T0=$(ck_now_ms)
  if [ "$KIND" = macos ]; then
    # An app started by `open` has its stderr only where --stderr sends it.
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
CK_EXTRA_CLEANUP=""

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
