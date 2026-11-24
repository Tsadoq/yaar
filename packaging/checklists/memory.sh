#!/bin/bash
# The resident memory of the app's own processes, one workspace open and idle, against 200 MB.
#
# packaging/checklists/memory.sh [--idle SECONDS]            macOS: the app in /Applications
# packaging/checklists/memory.sh --linux [--idle SECONDS]    Linux: the installed package
#
# Sums `ps -o rss=` (KiB) of this user's Window and Keeper processes, matched by full path, so
# language servers, agents and other copies of the app are left out. Samples every 10 s for the
# idle time (default 60) and judges the last against 200,000,000 bytes.
# Exit: 0 within 200 MB, 1 over or an app process missing or lost, 2 usage.
set -euo pipefail

CK_NAME=memory.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

LIMIT_BYTES=200000000
IDLE=60
LINUX=0
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
    --idle)
      [ "$#" -ge 2 ] || ck_usage_error "--idle needs a number of seconds"
      IDLE=$2
      shift 2
      case $IDLE in
        '' | *[!0-9]*) ck_usage_error "--idle needs a number of seconds, not '$IDLE'" ;;
      esac
      ;;
    *) ck_usage_error "unknown argument '$1'" ;;
  esac
done

if [ "$LINUX" -eq 1 ]; then
  [ "$(ck_os)" = Linux ] || ck_die "--linux measures a Linux machine and this is $(ck_os): run it there (linux-hardware.sh does)"
  KIND=linux
else
  [ "$(ck_os)" = Darwin ] || ck_die "this is not a Mac: run it with --linux"
  KIND=macos
fi

WINDOW_PROGRAM=$(ck_window_program)
KEEPER_PROGRAM=$(ck_keeper_program)

sample() {
  local total_w=0 total_k=0 pid kb found_w=0 found_k=0
  for pid in $(ck_pids_of "$WINDOW_PROGRAM"); do
    kb=$(ck_rss_kb "$pid")
    [ -n "$kb" ] || continue
    total_w=$((total_w + kb))
    found_w=1
  done
  for pid in $(ck_pids_of "$KEEPER_PROGRAM"); do
    kb=$(ck_rss_kb "$pid")
    [ -n "$kb" ] || continue
    total_k=$((total_k + kb))
    found_k=1
  done
  [ "$found_w" -eq 1 ] || ck_die "no Window is running (a process of $WINDOW_PROGRAM): open the app on one workspace first"
  [ "$found_k" -eq 1 ] || ck_die "no Keeper is running (a process of $KEEPER_PROGRAM): the Window starts one, so the app is not in the state PERF-03 describes"
  printf '%s %s\n' "$total_w" "$total_k"
}

RESULTS=$(ck_results_dir)
OUT="$RESULTS/memory-$KIND-$(date +%H%M%S).txt"
: > "$OUT"
say() {
  printf '%s\n' "$1" | tee -a "$OUT"
}

say "date=$(date '+%Y-%m-%d %H:%M:%S') idle=${IDLE}s"
say "window=$WINDOW_PROGRAM"
say "keeper=$KEEPER_PROGRAM"
say "$(printf 'second\twindow_kib\tkeeper_kib\tsum_kib')"

take() {
  local pair w k
  pair=$(sample)
  w=${pair% *}
  k=${pair#* }
  LAST_SUM=$((w + k))
  say "$(printf '%s\t%s\t%s\t%s' "$1" "$w" "$k" "$LAST_SUM")"
}

START=$SECONDS
NEXT=0
LAST_SUM=0
while [ $((SECONDS - START)) -lt "$IDLE" ]; do
  ELAPSED=$((SECONDS - START))
  if [ "$ELAPSED" -ge "$NEXT" ]; then
    take "$ELAPSED"
    NEXT=$((NEXT + 10))
  fi
  sleep 0.2
done
take "$((SECONDS - START))"

BYTES=$((LAST_SUM * 1024))
MIB=$(awk -v kb="$LAST_SUM" 'BEGIN { printf "%.1f", kb / 1024 }')
if [ "$BYTES" -le "$LIMIT_BYTES" ]; then
  say "the last sample, $BYTES bytes ($MIB MiB), is within 200 MB"
  STATUS=0
else
  say "the last sample, $BYTES bytes ($MIB MiB), is over 200 MB"
  STATUS=1
fi
printf '%s: written to %s\n' "$CK_NAME" "$OUT"
exit "$STATUS"
