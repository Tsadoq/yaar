#!/bin/bash
# The checks that need eyes on a lit screen, and the whole test suite, on macOS 27.
#
# packaging/checklists/screen-walk.sh [--dry-run]
#
# Runs `cargo nextest run --locked --workspace` first, then prints each check and asks "yes / no /
# note / skip". Opens no window and posts no input. --dry-run runs nothing.
# Needs the packaged app, cargo-nextest, YAAR_LANGUAGE_SERVERS naming a folder of real language
# servers (the suite fails without it), Ghostty, and `standin-agent` of the system tests linked as
# `claude`.
# Writes ~/yaar-checklists/<YYYY-MM-DD>/screen-walk.md. Exit: 0 written, 1 not walked or not
# written, 2 usage.
set -euo pipefail

CK_NAME=screen-walk.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

DRY=0
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
    *) ck_usage_error "unknown argument '$1'" ;;
  esac
done

RESULTS=$(ck_results_dir)
TERM_FILE="$RESULTS/term-06.txt"
{
  printf 'plain text\n'
  printf '\033[1mbold\033[0m \033[3mitalic\033[0m \033[1;3mbold italic\033[0m \033[4munderline\033[0m \033[9mstrike\033[0m\n'
  printf '\033[1;31mbold red\033[0m \033[3;32mitalic green\033[0m \033[1;3;34mbold italic blue\033[0m \033[7mreverse\033[0m\n'
  printf '\342\224\214\342\224\200\342\224\200\342\224\200\342\224\200\342\224\200\342\224\220\n'
  printf '\342\224\202 box \342\224\202\n'
  printf '\342\224\234\342\224\200\342\224\200\342\224\200\342\224\200\342\224\200\342\224\244\n'
  printf '\342\224\224\342\224\200\342\224\200\342\224\200\342\224\200\342\224\200\342\224\230\n'
  printf '\342\225\224\342\225\220\342\225\220\342\225\227 \342\226\210\342\226\223\342\226\222\342\226\221 \342\225\261\342\225\262\342\225\263\n'
  printf '\342\225\232\342\225\220\342\225\220\342\225\235\n'
} > "$TERM_FILE" || ck_die "cannot write $TERM_FILE"

ck_begin
ck_walker_context "$DRY"

if [ "$DRY" -eq 1 ]; then
  ck_context "Suite (SYS-04)" "not run: dry run"
else
  command -v cargo > /dev/null || ck_die "cargo is not on the path, which SYS-04's suite needs"
  cargo nextest --version > /dev/null 2>&1 || ck_die "cargo-nextest is not installed, which SYS-04's suite needs"
  [ -n "${YAAR_LANGUAGE_SERVERS:-}" ] || ck_die "YAAR_LANGUAGE_SERVERS names no folder: the tests of the real language servers fail without it, so the suite would not pass"
  [ -d "$YAAR_LANGUAGE_SERVERS" ] || ck_die "YAAR_LANGUAGE_SERVERS=$YAAR_LANGUAGE_SERVERS is not a folder"
  LOG="$RESULTS/screen-walk-nextest.log"
  REPO=$(cd "$HERE/../.." && pwd)
  printf '%s: running the suite in %s (log: %s)\n' "$CK_NAME" "$REPO" "$LOG"
  if (cd "$REPO" && cargo nextest run --locked --workspace) 2>&1 | tee "$LOG"; then
    SUITE="passed"
  else
    SUITE="FAILED"
  fi
  ck_context "Suite (SYS-04)" "$SUITE: $(tail -n 3 "$LOG" | tr '\n' ' ')"
  printf '\n%s: the suite %s\n' "$CK_NAME" "$SUITE"
fi

items_screen() {
  ck_item STACK-02 "The newer frame source" \
    "Quit the app. In a terminal: open -n --env YAAR_FRAME_SOURCE=metal /Applications/YAAR.app . Open a workspace, scroll a diff, type in a comment box." \
    "The app runs and draws, as with the default frame source."
  ck_item TERM-06 "Drawing of text" \
    "In a terminal pane of the app and in Ghostty, side by side, run: cat $TERM_FILE" \
    "Bold, italic, combined and box-drawing text draw as in Ghostty."
  ck_item SCREEN-01 "One review round, dark and light" \
    "With the stand-in agent in a terminal of the app: comment on a line, attach the review, send it, let the agent reply, make a second round. Do it in the dark theme, then in the light theme." \
    "Each step looks as its pane test says, and a review shows no vote and no label."
  ck_item SCREEN-02 "Dead keys and Pinyin" \
    "On the real keyboard, in a terminal pane and in a comment box: a dead key (Option-E, then e), then the Pinyin input method (type a syllable, choose a candidate). Do the same in Ghostty." \
    "Each gives in the terminal pane what it gives in Ghostty."
  ck_item SYS-04 "The suite and this list on macOS 27" \
    "Look at the suite's result in the file's header. This list is the four checks above (and the walks of conventions.sh)." \
    "The suite passed and every check of this list holds."
}

printf 'The checks of the lit screen. Open the app yourself, look, and answer here.\n'
ck_walk items_screen
ck_signoff "I accept these checks as passed on macOS 27"

if [ "$DRY" -eq 1 ]; then
  OUT="$RESULTS/screen-walk-dry-run.md"
else
  OUT="$RESULTS/screen-walk.md"
fi
ck_write "$OUT" "STACK-02, TERM-06, SCREEN-01, SCREEN-02 and SYS-04: the lit screen" "$CK_SIGN"
printf '\nWritten to %s\n' "$OUT"
