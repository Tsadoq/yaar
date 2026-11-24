#!/bin/bash
# A key is on screen at most 10 ms later than in Ghostty, at the median and p95, in three places:
# the terminal pane idle, the pane while an agent writes, and a comment box.
#
# packaging/checklists/typing.sh
# packaging/checklists/typing.sh --analyse <input log> <capture log>
#
# RUN BY HAND ONLY: it posts real key events and records the screen, so it asks for "go" first.
# Per terminal place: 208 or 210 letters at random 150-250 ms gaps, then the same on a second
# Ghostty running `cat` (the stream beside it while an agent writes); the comment box is compared
# with Ghostty idle. YAAR_REFERENCE_APP and YAAR_REFERENCE_PROGRAM name another reference.
# Needs Accessibility and Screen Recording, mains power, the app with one workspace, Ghostty, and
# the tools that YAAR_FAKEINPUT, YAAR_ADECAP and YAAR_STREAM name (or YAAR_KIT, a folder that holds
# them under tools/). --analyse posts nothing. Exit: 0 within 10 ms in all three, 1 over or not
# done, 2 usage.
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
FAKEINPUT=$(ck_fakeinput)
REFERENCE=${YAAR_REFERENCE_APP:-Ghostty}
STREAM=$(ck_stream_script)
[ -f "$STREAM" ] || ck_die "no stream.pl at $STREAM: name it in YAAR_STREAM (or its folder in YAAR_KIT)"
WINDOW_PROGRAM=$(ck_window_program)
APP_PIDS=$(ck_pids_of "$WINDOW_PROGRAM")
case $APP_PIDS in
  '') ck_die "the app is not running (no process of $WINDOW_PROGRAM): open it on one workspace first" ;;
  *[[:space:]]*) ck_die "more than one Window of the app is running ($(echo "$APP_PIDS" | tr '\n' ' ')): quit the others" ;;
esac
APP_PID=$APP_PIDS
if [ -n "${YAAR_REFERENCE_PROGRAM:-}" ]; then
  REF_PROGRAM=$YAAR_REFERENCE_PROGRAM
elif [ -z "${YAAR_REFERENCE_APP:-}" ]; then
  REF_PROGRAM=/Applications/Ghostty.app/Contents/MacOS/ghostty
else
  ck_die "YAAR_REFERENCE_APP names $REFERENCE, so the full path of its program is needed too: name it in YAAR_REFERENCE_PROGRAM"
fi
[ -x "$REF_PROGRAM" ] || ck_die "no reference program at $REF_PROGRAM: install $REFERENCE, or name its program in YAAR_REFERENCE_PROGRAM"

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
ck_go "The app is pid $APP_PID."

is_front() {
  [ "$(ck_front_pid)" = "$1" ]
}

# Front by pid: asking for an application by name brings whichever instance LaunchServices likes.
bring_front() {
  case $1 in
    '' | *[!0-9]*) ck_die "the pid to bring to the front is '$1', not a number" ;;
  esac
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $1) to true" > /dev/null \
    || ck_die "could not bring the process $1 to the front (the terminal that runs this needs Accessibility)"
  ck_wait_until 10 is_front "$1" || ck_die "the application (pid $1) did not come to the front in 10 s: another window took the focus"
}

type_into() {
  local name=$1 pid=$2 count=$3 batch=$4
  bring_front "$pid"
  ck_cap_start "$OUT/$name.cap" "$pid" "$OUT/$name.capstop"
  bring_front "$pid"
  "$FAKEINPUT" type --pid "$pid" --log "$OUT/$name.input" --count "$count" --batch "$batch" \
    || ck_die "fakeinput stopped early on $name (see $OUT/$name.input)"
  ck_cap_stop "$OUT/$name.cap" "$OUT/$name.capstop"
}

STARTED_REFERENCE=""
end_started_reference() {
  if [ -n "$STARTED_REFERENCE" ] && kill -0 "$STARTED_REFERENCE" 2> /dev/null; then
    kill "$STARTED_REFERENCE" 2> /dev/null || true
  fi
}
CK_EXTRA_CLEANUP=end_started_reference

# The reference's pid is the one new process of REF_PROGRAM, by full path, after the start:
# the frontmost application says nothing of which process is ours.
reference() {
  local name=$1 command=$2 pid="" known
  known=$(ck_pids_of "$REF_PROGRAM")
  open -n -a "$REFERENCE" --args "--command=$command" --window-save-state=never --confirm-close-surface=false --quit-after-last-window-closed=true
  new_reference() {
    local found
    found=$(ck_new_pids_of "$REF_PROGRAM" "$known")
    [ -n "$found" ] || return 1
    pid=$found
  }
  ck_wait_until 15 new_reference || ck_die "no new process of $REF_PROGRAM appeared in 15 s after the start of a second $REFERENCE"
  case $pid in
    *[[:space:]]*) ck_die "more than one new process of $REF_PROGRAM appeared ($(echo "$pid" | tr '\n' ' ')): none was ended, close the one you do not know yourself" ;;
  esac
  STARTED_REFERENCE=$pid
  has_child() {
    ps -axo pid=,ppid= | awk -v p="$pid" '$2 == p { found = 1 } END { exit !found }'
  }
  ck_wait_until 15 has_child || ck_die "the second $REFERENCE (pid $pid) started no command in 15 s"
  type_into "$name" "$pid" 210 30
  kill "$pid"
  gone() { ! kill -0 "$pid" 2> /dev/null; }
  ck_wait_until 10 gone || ck_die "the second $REFERENCE (pid $pid) did not leave in 10 s"
  STARTED_REFERENCE=""
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

wait_ready "Place 1 of 3. In the app, click into a terminal pane and run: cat" "place 1"
type_into app-terminal-idle "$APP_PID" 210 30
reference ref-terminal-idle cat
APP_IDLE=$(summary "$OUT/app-terminal-idle.input" "$OUT/app-terminal-idle.cap" 2> /dev/null)
REF_IDLE=$(summary "$OUT/ref-terminal-idle.input" "$OUT/ref-terminal-idle.cap" 2> /dev/null)
judge "terminal idle" "$APP_IDLE" "$REF_IDLE"

wait_ready "Place 2 of 3. In the app, stop what runs in the terminal pane (Ctrl-C), then run: perl '$STREAM' & cat" "place 2"
type_into app-terminal-stream "$APP_PID" 208 0
reference ref-terminal-stream "$WRAPPER"
APP_STREAM=$(summary "$OUT/app-terminal-stream.input" "$OUT/app-terminal-stream.cap" 2> /dev/null)
REF_STREAM=$(summary "$OUT/ref-terminal-stream.input" "$OUT/ref-terminal-stream.cap" 2> /dev/null)
judge "terminal while output streams" "$APP_STREAM" "$REF_STREAM"

wait_ready "Place 3 of 3. In the app, stop what runs in the terminal pane, open a comment box on a diff line, and click into it" "place 3"
type_into app-comment-box "$APP_PID" 208 0
APP_COMMENT=$(summary "$OUT/app-comment-box.input" "$OUT/app-comment-box.cap" 2> /dev/null)
judge "comment box (against $REFERENCE idle)" "$APP_COMMENT" "$REF_IDLE"

printf '%s: written to %s\n' "$CK_NAME" "$OUT"
exit "$OVER"
