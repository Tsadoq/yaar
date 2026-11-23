#!/bin/bash
# PERF-04: a key is on screen at most 10 ms later than in Ghostty, at the median and at the
# 95th percentile, in three places: the terminal pane idle, the terminal pane while an agent
# writes, and a comment box. (HW-03 asks the same of the Linux machine against foot: see
# linux-hardware.sh.)
#
#   packaging/checklists/typing.sh
#   packaging/checklists/typing.sh --analyse <input log> <capture log>
#
# NOT RUN BY ANY AGENT. It posts real key events to the system and records the screen, so it
# runs only on the owner's word: it asks for the word "go" before it posts anything, and
# fakeinput itself refuses to post unless the app it types into is the frontmost one.
#
# What it does, for each of the three places: asks you to put the app in the state named
# (you start `cat` or the stream in the terminal pane, or open a comment box, and click in
# it), then starts adecap.app on the app's Window (it logs the display time of every frame of
# that window whose pixels changed), brings the app to the front, posts 208 or 210 letters
# with fakeinput at random gaps of 150 to 250 ms (every 30 in the idle case it also posts
# Ctrl-U to clear the line), stops the capture, and does the same on a second instance of
# Ghostty running `cat` (for "while an agent writes", `cat` with the same stream beside it;
# the comment box is compared with Ghostty idle, the only text-input surface it has). The
# posts and the frames are on one clock, the system's uptime in nanoseconds: the key to
# screen time is the first changed frame after each post (a frame sooner than 12 ms is not
# the key's; a key with none before the next key plus 20 ms, or within 400 ms, is missed).
# The figures are the median and the 95th percentile of those times, for the app and for
# Ghostty, and their difference against 10 ms.
#
# What it needs of the owner: his "go"; mains power, an unlocked screen, nobody at the
# keyboard; Accessibility for the terminal that runs this (to post events); Screen Recording
# for adecap.app (not for the terminal); the packaged app running with one workspace open;
# Ghostty installed. The measuring tools are the phase 4 kit's, read and never rebuilt:
# fakeinput, adecap.app and tools/stream.pl in YAAR_KIT (default
# ~/ade-phase4-2026-11-12/gpui-measure), or YAAR_FAKEINPUT and YAAR_ADECAP outright.
# YAAR_REFERENCE_APP names the application to compare with (default Ghostty).
#
# --analyse reads the logs of an earlier case (fakeinput's `--log` and adecap's `--log`
# files) and prints the number of keys, the median and the 95th percentile of the key to
# screen time; it posts nothing and opens nothing.
#
# Where it writes: ~/yaar-checklists/<YYYY-MM-DD>/typing-<HHMMSS>/ with, for each place, the
# input log and the capture log of the app and of the reference, and results.txt.
#
# Exit codes: 0 the difference is within 10 ms at the median and at the 95th percentile in
# all three places; 1 it is not, or the run could not be done (a sentence says which); 2
# usage error.
set -euo pipefail

CK_NAME=typing.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

LIMIT_MS=10
ANALYSE_INPUT=""
ANALYSE_CAP=""
while [ "$#" -gt 0 ]; do
  case $1 in
    -h | --help)
      ck_help "$0"
      exit 0
      ;;
    --analyse)
      [ "$#" -ge 3 ] || ck_usage_error "--analyse needs the input log and the capture log"
      ANALYSE_INPUT=$2
      ANALYSE_CAP=$3
      shift 3
      ;;
    *) ck_usage_error "unknown argument '$1'" ;;
  esac
done

# summary <input log> <capture log>: "n=<keys> median=<ms> p95=<ms>" and, apart, the counts
# of missed and early keys on standard error.
summary() {
  ck_key_latencies "$1" "$2" | ck_median_p95
}

if [ -n "$ANALYSE_INPUT" ]; then
  [ -f "$ANALYSE_INPUT" ] || ck_die "$ANALYSE_INPUT is not a file"
  [ -f "$ANALYSE_CAP" ] || ck_die "$ANALYSE_CAP is not a file"
  summary "$ANALYSE_INPUT" "$ANALYSE_CAP"
  exit 0
fi

ck_lit_preflight 1 > /dev/null
KIT=$(ck_kit_dir)
FAKEINPUT=$(ck_fakeinput)
REFERENCE=${YAAR_REFERENCE_APP:-Ghostty}
STREAM="$KIT/tools/stream.pl"
[ -f "$STREAM" ] || ck_die "no stream.pl at $STREAM: it is the phase 4 kit's output generator"
APP=$(ck_app_bundle)
WINDOW_PROGRAM=$(ck_window_program)
APP_PIDS=$(ck_pids_of "$WINDOW_PROGRAM")
case $APP_PIDS in
  '') ck_die "the app is not running (no process of $WINDOW_PROGRAM): open it on one workspace first" ;;
  *[[:space:]]*) ck_die "more than one Window of the app is running ($(echo "$APP_PIDS" | tr '\n' ' ')): quit the others" ;;
esac
APP_PID=$APP_PIDS

RESULTS=$(ck_results_dir)
OUT="$RESULTS/typing-$(date +%H%M%S)"
mkdir "$OUT" || ck_die "cannot make $OUT"
WRAPPER="$OUT/reference-stream.sh"
printf '#!/bin/sh\nperl %s &\nexec cat\n' "'$STREAM'" > "$WRAPPER"
chmod 755 "$WRAPPER"

cat << EOF
This posts real keys to the system, about 600 of them in all, to the app and to a second
$REFERENCE, and records the screen. Nobody should touch the keyboard or the mouse.
EOF
ck_go "The app is pid $APP_PID, the measuring tools are in $KIT."

is_front() {
  [ "$(ck_front_pid)" = "$1" ]
}

bring_front() {
  open -a "$2"
  ck_wait_until 10 is_front "$1" || ck_die "the application (pid $1) did not come to the front in 10 s: another window took the focus"
}

# type_into <name> <pid> <app to open> <count> <batch>: one capture and one run of keys.
type_into() {
  local name=$1 pid=$2 target=$3 count=$4 batch=$5
  bring_front "$pid" "$target"
  ck_cap_start "$OUT/$name.cap" "$pid" "$OUT/$name.capstop"
  bring_front "$pid" "$target"
  "$FAKEINPUT" type --pid "$pid" --log "$OUT/$name.input" --count "$count" --batch "$batch" \
    || ck_die "fakeinput stopped early on $name (see $OUT/$name.input)"
  ck_cap_stop "$OUT/$name.cap" "$OUT/$name.capstop"
}

# reference <name> <command>: a second instance of the reference terminal running the command,
# typed into and closed again by its pid.
reference() {
  local name=$1 command=$2 pid="" before
  before=$(ck_front_pid)
  open -n -a "$REFERENCE" --args "--command=$command" --window-save-state=never --confirm-close-surface=false --quit-after-last-window-closed=true
  newest_front() {
    local front
    front=$(ck_front_pid)
    [ "$front" != "$before" ] && [ "$front" != "$APP_PID" ] && ps -p "$front" -o comm= | grep -qi "$REFERENCE"
  }
  ck_wait_until 15 newest_front || ck_die "a second $REFERENCE did not come to the front in 15 s"
  pid=$(ck_front_pid)
  local settled=$((SECONDS + 2))
  while [ "$SECONDS" -lt "$settled" ]; do sleep 0.2; done
  type_into "$name" "$pid" "$REFERENCE" 210 30
  kill "$pid"
  gone() { ! kill -0 "$pid" 2> /dev/null; }
  ck_wait_until 10 gone || ck_die "the second $REFERENCE (pid $pid) did not leave in 10 s"
}

wait_ready() {
  printf '%s\nPress Return when it is so: ' "$1"
  ck_read_line "$2"
}

RESULTS_TXT="$OUT/results.txt"
: > "$RESULTS_TXT"
say() {
  printf '%s\n' "$1" | tee -a "$RESULTS_TXT"
}
OVER=0

# judge <case> <app summary> <reference summary>
judge() {
  local app_median app_p95 ref_median ref_p95 dm dp
  app_median=$(printf '%s\n' "$2" | sed -n 's/.*median=\([0-9.]*\).*/\1/p')
  app_p95=$(printf '%s\n' "$2" | sed -n 's/.*p95=\([0-9.]*\).*/\1/p')
  ref_median=$(printf '%s\n' "$3" | sed -n 's/.*median=\([0-9.]*\).*/\1/p')
  ref_p95=$(printf '%s\n' "$3" | sed -n 's/.*p95=\([0-9.]*\).*/\1/p')
  if [ -z "$app_median" ] || [ -z "$ref_median" ]; then
    say "$1: no key was matched to a frame (app: $2; $REFERENCE: $3)"
    OVER=1
    return 0
  fi
  dm=$(awk -v a="$app_median" -v b="$ref_median" 'BEGIN { printf "%.2f", a - b }')
  dp=$(awk -v a="$app_p95" -v b="$ref_p95" 'BEGIN { printf "%.2f", a - b }')
  say "$1: app median $app_median ms, p95 $app_p95 ms; $REFERENCE median $ref_median ms, p95 $ref_p95 ms; later than $REFERENCE by $dm ms at the median and $dp ms at the p95"
  if awk -v m="$dm" -v p="$dp" -v l="$LIMIT_MS" 'BEGIN { exit !(m <= l && p <= l) }'; then
    say "$1: within $LIMIT_MS ms"
  else
    say "$1: OVER $LIMIT_MS ms"
    OVER=1
  fi
}

# --- the terminal pane, idle
wait_ready "Place 1 of 3. In the app, click into a terminal pane and run: cat" "place 1"
type_into app-terminal-idle "$APP_PID" "$APP" 210 30
reference ref-terminal-idle cat
APP_IDLE=$(summary "$OUT/app-terminal-idle.input" "$OUT/app-terminal-idle.cap" 2> /dev/null)
REF_IDLE=$(summary "$OUT/ref-terminal-idle.input" "$OUT/ref-terminal-idle.cap" 2> /dev/null)
judge "terminal idle" "$APP_IDLE" "$REF_IDLE"

# --- the terminal pane while output streams
wait_ready "Place 2 of 3. In the app, stop what runs in the terminal pane (Ctrl-C), then run: perl '$STREAM' & cat" "place 2"
type_into app-terminal-stream "$APP_PID" "$APP" 208 0
reference ref-terminal-stream "$WRAPPER"
APP_STREAM=$(summary "$OUT/app-terminal-stream.input" "$OUT/app-terminal-stream.cap" 2> /dev/null)
REF_STREAM=$(summary "$OUT/ref-terminal-stream.input" "$OUT/ref-terminal-stream.cap" 2> /dev/null)
judge "terminal while output streams" "$APP_STREAM" "$REF_STREAM"

# --- a comment box
wait_ready "Place 3 of 3. In the app, stop what runs in the terminal pane, open a comment box on a diff line, and click into it" "place 3"
type_into app-comment-box "$APP_PID" "$APP" 208 0
APP_COMMENT=$(summary "$OUT/app-comment-box.input" "$OUT/app-comment-box.cap" 2> /dev/null)
judge "comment box (against $REFERENCE idle)" "$APP_COMMENT" "$REF_IDLE"

printf '%s: written to %s\n' "$CK_NAME" "$OUT"
exit "$OVER"
