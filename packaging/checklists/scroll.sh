#!/bin/bash
# PERF-05: scrolling a diff of 2,200 lines, unified and split: the app draws at the refresh
# rate of the display, and at most 1% of the frames take longer than two refresh intervals.
#
#   packaging/checklists/scroll.sh
#   packaging/checklists/scroll.sh --analyse <input log> <capture log> <refresh Hz>
#
# NOT RUN BY ANY AGENT. It posts real scroll-wheel events to the system and records the
# screen, so it runs only on the owner's word: it asks for the word "go" before it posts
# anything, and fakeinput refuses to post unless the app is the frontmost one.
#
# What it does, for the unified view and then for the split view: asks you to put the app on
# a diff of 2,200 lines in that view with the review pane in front, starts adecap.app on the
# app's Window (it logs the display time of every frame of that window whose pixels changed),
# moves the pointer to the pane (75% across the window, 55% down), and posts pixel scroll
# events with fakeinput at the display's refresh rate: 8 s down at 30 pixels an event, 8 s
# up, 6 s down at 6 pixels. From the capture it counts, for each of the three phases and
# in total, the frames, the frames a second, and the gaps between consecutive frames longer
# than two refresh intervals. The verdict of a view: frames a second at least 95% of the
# refresh rate, and at most 1% of the gaps long.
#
# What it needs of the owner: his "go"; mains power, an unlocked screen, nobody at the
# keyboard; Accessibility for the terminal that runs this; Screen Recording for adecap.app;
# the packaged app running on a workspace with a diff of 2,200 lines (the review pane's
# unified and split views are the app's own setting); the window not covered. The
# measuring tools are the phase 4 kit's, read and never rebuilt: fakeinput and adecap.app in
# YAAR_KIT (default ~/ade-phase4-2026-11-12/gpui-measure), or YAAR_FAKEINPUT and YAAR_ADECAP
# outright. The window's place is read through System Events (Accessibility).
#
# --analyse reads the logs of an earlier run and the refresh rate in Hz and prints the
# figures above; it posts nothing and opens nothing.
#
# Where it writes: ~/yaar-checklists/<YYYY-MM-DD>/scroll-<HHMMSS>/ with the input log and
# the capture log of each view, and results.txt.
#
# Exit codes: 0 both views are within the limits; 1 one is not, or the run could not be done
# (a sentence says which); 2 usage error.
set -euo pipefail

CK_NAME=scroll.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

ANALYSE_INPUT=""
ANALYSE_CAP=""
ANALYSE_HZ=""
while [ "$#" -gt 0 ]; do
  case $1 in
    -h | --help)
      ck_help "$0"
      exit 0
      ;;
    --analyse)
      [ "$#" -ge 4 ] || ck_usage_error "--analyse needs the input log, the capture log and the refresh rate"
      ANALYSE_INPUT=$2
      ANALYSE_CAP=$3
      ANALYSE_HZ=$4
      shift 4
      ;;
    *) ck_usage_error "unknown argument '$1'" ;;
  esac
done

if [ -n "$ANALYSE_INPUT" ]; then
  [ -f "$ANALYSE_INPUT" ] || ck_die "$ANALYSE_INPUT is not a file"
  [ -f "$ANALYSE_CAP" ] || ck_die "$ANALYSE_CAP is not a file"
  case $ANALYSE_HZ in
    '' | *[!0-9]*) ck_usage_error "the refresh rate is a whole number of Hz, not '$ANALYSE_HZ'" ;;
  esac
  ck_frame_stats "$ANALYSE_INPUT" "$ANALYSE_CAP" "$ANALYSE_HZ"
  exit 0
fi

INFO=$(ck_lit_preflight 1)
FAKEINPUT=$(ck_fakeinput)
HZ=$(printf '%s\n' "$INFO" | sed -n 's/^display_mode_hz=\([0-9][0-9]*\).*/\1/p')
if [ -z "$HZ" ] || [ "$HZ" -le 0 ]; then
  ck_die "fakeinput does not report the display's refresh rate ($(printf '%s' "$INFO" | grep display_mode_hz))"
fi
APP=$(ck_app_bundle)
WINDOW_PROGRAM=$(ck_window_program)
APP_PIDS=$(ck_pids_of "$WINDOW_PROGRAM")
case $APP_PIDS in
  '') ck_die "the app is not running (no process of $WINDOW_PROGRAM): open it on the workspace with the 2,200-line diff first" ;;
  *[[:space:]]*) ck_die "more than one Window of the app is running ($(echo "$APP_PIDS" | tr '\n' ' ')): quit the others" ;;
esac
APP_PID=$APP_PIDS

RESULTS=$(ck_results_dir)
OUT="$RESULTS/scroll-$(date +%H%M%S)"
mkdir "$OUT" || ck_die "cannot make $OUT"
RESULTS_TXT="$OUT/results.txt"
: > "$RESULTS_TXT"
say() {
  printf '%s\n' "$1" | tee -a "$RESULTS_TXT"
}

ck_go "This posts scroll events to the app (pid $APP_PID) for about 25 s a view, at $HZ Hz, and records the screen. Nobody should touch the keyboard or the mouse."

is_front() {
  [ "$(ck_front_pid)" = "$APP_PID" ]
}

# window_box: "x y w h" of the app's first window, in points.
window_box() {
  local raw
  raw=$(osascript -e "tell application \"System Events\" to tell (first process whose unix id is $APP_PID) to get {position, size} of window 1") \
    || ck_die "System Events cannot read the window of pid $APP_PID (Accessibility may be missing for this terminal)"
  printf '%s\n' "$raw" | tr -d ',' | awk '{ print $1, $2, $3, $4 }'
}

OVER=0
# view <name> <what to set>
view() {
  local name=$1
  printf '%s\nPress Return when it is so: ' "$2"
  ck_read_line "the $name view"
  open -a "$APP"
  ck_wait_until 10 is_front || ck_die "the app (pid $APP_PID) did not come to the front in 10 s: another window took the focus"
  local box x y w h px py
  box=$(window_box)
  read -r x y w h <<< "$box"
  px=$(awk -v x="$x" -v w="$w" 'BEGIN { printf "%.0f", x + w * 0.75 }')
  py=$(awk -v y="$y" -v h="$h" 'BEGIN { printf "%.0f", y + h * 0.55 }')
  ck_cap_start "$OUT/$name.cap" "$APP_PID" "$OUT/$name.capstop"
  "$FAKEINPUT" scroll --pid "$APP_PID" --log "$OUT/$name.input" --x "$px" --y "$py" --dy 30 --hz "$HZ" --seconds 8 \
    || ck_die "fakeinput stopped early in $name (see $OUT/$name.input)"
  "$FAKEINPUT" scroll --pid "$APP_PID" --log "$OUT/$name.input" --x "$px" --y "$py" --dy -30 --hz "$HZ" --seconds 8 \
    || ck_die "fakeinput stopped early in $name (see $OUT/$name.input)"
  "$FAKEINPUT" scroll --pid "$APP_PID" --log "$OUT/$name.input" --x "$px" --y "$py" --dy 6 --hz "$HZ" --seconds 6 \
    || ck_die "fakeinput stopped early in $name (see $OUT/$name.input)"
  ck_cap_stop "$OUT/$name.cap" "$OUT/$name.capstop"
  local stats total fps pct
  stats=$(ck_frame_stats "$OUT/$name.input" "$OUT/$name.cap" "$HZ")
  say "$name view:"
  say "$stats"
  total=$(printf '%s\n' "$stats" | grep '^total:')
  fps=$(printf '%s\n' "$total" | sed -n 's/.* fps=\([0-9.]*\) .*/\1/p')
  pct=$(printf '%s\n' "$total" | sed -n 's/.*long_pct=\([0-9.]*\).*/\1/p')
  if awk -v fps="$fps" -v pct="$pct" -v hz="$HZ" 'BEGIN { exit !(fps >= 0.95 * hz && pct <= 1.0) }'; then
    say "$name view: within the limits ($fps frames a second of $HZ, $pct% of the gaps long)"
  else
    say "$name view: OVER the limits ($fps frames a second of $HZ, $pct% of the gaps long; at least $(awk -v hz="$HZ" 'BEGIN { printf "%.1f", 0.95 * hz }') and at most 1% are asked)"
    OVER=1
  fi
}

view unified "Place 1 of 2. In the app, open the diff of 2,200 lines in the UNIFIED view and click into the review pane so that it takes the wheel."
view split "Place 2 of 2. Switch the same diff to the SPLIT view."

printf '%s: written to %s\n' "$CK_NAME" "$OUT"
exit "$OVER"
