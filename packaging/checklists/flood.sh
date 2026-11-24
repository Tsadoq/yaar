#!/bin/bash
# Checks that the terminal pane takes a flood of 200,000 lines in at most 1.5 times the time the
# writing program needs alone.
#
# packaging/checklists/flood.sh
#
# RUN BY HAND ONLY: it types into the real window; it asks for "go" and types only while the app
# is frontmost. Needs Accessibility for the running terminal, an unlocked screen, the app at a
# shell prompt with the pane focused, perl, and the fakeinput program that YAAR_FAKEINPUT names (or
# YAAR_KIT, a folder that holds it as tools/fakeinput). Exit: 0 ratio <= 1.5, 1 over or not done,
# 2 usage.
set -euo pipefail

CK_NAME=flood.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

LIMIT_RATIO=1.5
while [ "$#" -gt 0 ]; do
  case $1 in
    -h | --help)
      ck_help "$0"
      exit 0
      ;;
    *) ck_usage_error "unknown argument '$1'" ;;
  esac
done

ck_lit_preflight 0 > /dev/null
command -v perl > /dev/null || ck_die "perl is missing, which the generator needs"
command -v osascript > /dev/null || ck_die "osascript is missing, which types into the pane"
WINDOW_PROGRAM=$(ck_window_program)
APP_PIDS=$(ck_pids_of "$WINDOW_PROGRAM")
case $APP_PIDS in
  '') ck_die "the app is not running (no process of $WINDOW_PROGRAM): open it with a terminal pane first" ;;
  *[[:space:]]*) ck_die "more than one Window of the app is running ($(echo "$APP_PIDS" | tr '\n' ' ')): quit the others" ;;
esac
APP_PID=$APP_PIDS
APP=$(ck_app_bundle)

RESULTS=$(ck_results_dir)
OUT="$RESULTS/flood-$(date +%H%M%S)"
mkdir "$OUT" || ck_die "cannot make $OUT"
case $OUT in
  *[\"\\\'\$\`]*) ck_die "$OUT holds a quote, a backslash, a dollar or a backquote, which cannot be typed safely" ;;
esac
GEN="$OUT/flood-generator.pl"
cat > "$GEN" << 'EOF'
use strict;
use warnings;
use Time::HiRes qw(time);
$| = 1;
my $t0 = time;
for my $i (1 .. 200000) {
    printf("\e[3%dm%06d\e[0m %s\n", 1 + $i % 6, $i, "the quick brown fox jumps over the lazy dog 0123456789");
}
my $secs = sprintf("%.2f", time - $t0);
print "FLOOD_DONE ${secs}s\n";
if (@ARGV) {
    open(my $fh, ">>", $ARGV[0]) or die "cannot append to $ARGV[0]: $!";
    print $fh "$secs\n";
    close($fh);
}
EOF
RESULTS_TXT="$OUT/results.txt"
: > "$RESULTS_TXT"
say() {
  printf '%s\n' "$1" | tee -a "$RESULTS_TXT"
}

ck_go "This types a command into the terminal pane of the app (pid $APP_PID) three times, and runs the generator alone three times."

: > "$OUT/alone.txt"
: > "$OUT/in-app.txt"
RUN=1
while [ "$RUN" -le 3 ]; do
  perl "$GEN" "$OUT/alone.txt" | tail -n 1 > /dev/null
  RUN=$((RUN + 1))
done
say "alone: $(tr '\n' ' ' < "$OUT/alone.txt")seconds"

is_front() {
  [ "$(ck_front_pid)" = "$APP_PID" ]
}

RUN=1
while [ "$RUN" -le 3 ]; do
  open -a "$APP"
  ck_wait_until 10 is_front || ck_die "the app (pid $APP_PID) did not come to the front in 10 s: another window took the focus"
  BEFORE=$(wc -l < "$OUT/in-app.txt" | tr -d ' ')
  osascript \
    -e 'tell application "System Events" to keystroke "l" using control down' \
    -e "tell application \"System Events\" to keystroke \"perl '$GEN' '$OUT/in-app.txt'\"" \
    -e 'tell application "System Events" to key code 36' \
    || ck_die "System Events could not type into the app (Accessibility may be missing for this terminal)"
  grew() {
    [ "$(wc -l < "$OUT/in-app.txt" | tr -d ' ')" -gt "$BEFORE" ]
  }
  if ! ck_wait_until 120 grew; then
    ck_die "the generator wrote no time in 120 s: the command did not run in the pane (is a shell prompt in front, and the pane focused?)"
  fi
  RUN=$((RUN + 1))
done
say "in the app: $(tr '\n' ' ' < "$OUT/in-app.txt")seconds"

ALONE=$(ck_median_p95 < "$OUT/alone.txt" | sed -n 's/.*median=\([0-9.]*\).*/\1/p')
INAPP=$(ck_median_p95 < "$OUT/in-app.txt" | sed -n 's/.*median=\([0-9.]*\).*/\1/p')
if [ -z "$ALONE" ] || [ -z "$INAPP" ]; then
  ck_die "no time was recorded ($OUT/alone.txt, $OUT/in-app.txt)"
fi
RATIO=$(awk -v a="$ALONE" -v b="$INAPP" 'BEGIN { printf "%.2f", b / a }')
say "median alone $ALONE s, median in the app $INAPP s: $RATIO times"
STATUS=0
if awk -v r="$RATIO" -v l="$LIMIT_RATIO" 'BEGIN { exit !(r <= l) }'; then
  say "within $LIMIT_RATIO times"
else
  say "OVER $LIMIT_RATIO times"
  STATUS=1
fi
printf '%s: written to %s\n' "$CK_NAME" "$OUT"
exit "$STATUS"
