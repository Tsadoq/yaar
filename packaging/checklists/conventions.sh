#!/bin/bash
# The platform conventions checklists, walked in the packaged app and signed: CONV-01 for
# macOS (17 items) and CONV-02 for Linux (11 items).
#
#   packaging/checklists/conventions.sh macos [--dry-run]
#   packaging/checklists/conventions.sh linux [--dry-run]
#
# What it does: prints the items one at a time, with what to do and what to expect, and asks
# "yes / no / note / skip" on the terminal (the answers can come from a file instead:
# `conventions.sh macos < answers.txt`, the format is in lib.sh). A "no" is a defect or a
# decision and asks for one line saying which; neither is left unsaid. Items 11 and 16 of
# macOS print the steps of the light theme walk that no test can make. At the end, when every
# item holds, it asks whether you sign; otherwise the sign-off names the items refused.
#
# It opens nothing and posts no input. Open the app yourself, walk the items in it, answer
# here. The programs it runs are `yaar-review --version` (it prints and ends) and `sw_vers`
# (macOS), and it reads /etc/os-release (Linux), to record the version and the system.
#
# What it needs of the owner: the packaged app installed (on macOS at /Applications/YAAR.app,
# or YAAR_APP; on Linux `yaar-review` from the package, or YAAR_REVIEW names the Window
# program), and a desktop to walk it on: macOS 26 or 27; for Linux GNOME under Wayland on the
# newest promised Ubuntu, then the differences under X11.
#
# --dry-run: runs no program of the system and none of the app (the version and the system
# are written as "not asked: dry run") and writes conventions-<list>-dry-run.md instead of
# the sign-off file, so that a rehearsal never takes the place of a real sign-off.
#
# Where it writes: ~/yaar-checklists/<YYYY-MM-DD>/conventions-macos.md or
# conventions-linux.md: date, version, system, one row per item with its answer and note, and
# the sign-off. Written whole at the end, or not at all.
#
# Exit codes: 0 the walk is written (whatever its answers); 1 it could not be walked or
# written (the answers ended, the app is missing: a sentence says which); 2 usage error.
set -euo pipefail

CK_NAME=conventions.sh
HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=lib.sh
. "$HERE/lib.sh"

LIST=""
DRY=0
for arg in "$@"; do
  case $arg in
    -h | --help)
      ck_help "$0"
      exit 0
      ;;
    --dry-run) DRY=1 ;;
    macos | linux)
      [ -z "$LIST" ] || ck_usage_error "two lists named, $LIST and $arg"
      LIST=$arg
      ;;
    *) ck_usage_error "unknown argument '$arg'" ;;
  esac
done
[ -n "$LIST" ] || ck_usage_error "name the list: macos or linux"

items_macos() {
  ck_item 1 "The app menu" \
    "Open the YAAR menu in the menu bar." \
    "About YAAR, Settings on Cmd-comma, Hide YAAR on Cmd-H, Hide Others, Quit YAAR on Cmd-Q."
  ck_item 2 "Cmd-Q" \
    "With an agent running in a terminal of the app, press Cmd-Q. Then find the menu entry that quits and stops the agents." \
    "Cmd-Q closes the window and the agents keep running. Stopping the agents is a menu entry of its own."
  ck_item 3 "The Edit menu" \
    "In a comment box and in the settings fields, use Undo, Redo, Cut, Copy, Paste and Select All, from the menu and by their keys." \
    "Each works in every text field, with the system's keys."
  ck_item 4 "The Window menu" \
    "Open the Window menu; press Cmd-M." \
    "Minimize on Cmd-M, Zoom, and the open window listed."
  ck_item 5 "The window's three buttons" \
    "Look at the close, minimize and full screen buttons. Double-click the title bar." \
    "The buttons are at the system's place. The double click does what System Settings says."
  ck_item 6 "Cmd-W" \
    "Press Cmd-W with the window in front." \
    "It closes the window as the red button does."
  ck_item 7 "Full screen" \
    "Enter full screen by the green button, leave it, enter it with Ctrl-Cmd-F, leave it." \
    "Both ways work and the layout holds."
  ck_item 8 "Size and place" \
    "Move and resize the window, quit, start the app again; once more with the window on a second screen if there is one." \
    "The window reopens where and as large as it was left, on the screen it was on."
  ck_item 9 "The Dock" \
    "While the app runs, look at the Dock. Open the icon's menu. Close the window, then click the icon." \
    "The icon shows, its menu has Quit, and a click brings the window back."
  ck_item 10 "Text fields" \
    "In a comment box: Option-arrow, Cmd-arrow, Ctrl-Cmd-Space, and a dead key (Option-E, then e)." \
    "Option-arrow moves by word, Cmd-arrow by line, the character viewer opens, the dead key composes."
  ck_item 11 "Light and dark" \
    "With no theme chosen, switch System Settings between light and dark while the app runs. Then, in the settings, choose Light while the system is light, choose \"as the system\", and switch the system to dark. Do the same with Dark chosen while the system is dark, switching to light. Do the same with a theme name that no theme has. Last, quit, set the system to dark, start the app." \
    "The window and its frame follow the system each time. Started in the dark, the first frame is already dark, with no flash of light."
  ck_item 12 "Scrolling" \
    "Scroll a diff with a trackpad and with a mouse wheel. Change the system's scroll bar and direction settings and scroll again." \
    "Scroll bars show by the system's setting. Scrolling has the system's momentum and direction."
  ck_item 13 "A first open after download" \
    "Download the .dmg from the run's page in a browser (it carries the quarantine mark), open it, drag the app to a folder, open the app." \
    "Gatekeeper opens it with the usual one question and no warning."
  ck_item 14 "Reduced motion" \
    "Turn on Reduce motion in System Settings, Accessibility, Display. Use the app: open files, switch panes." \
    "Nothing animates beyond a fade."
  ck_item 15 "Keys the system owns" \
    "With a terminal pane focused: Cmd-Tab, Cmd-Space, Mission Control, and the screenshot keys." \
    "Each does what it does in every app. The terminal sends none of them to its program."
  ck_item 16 "The light theme on a screen" \
    "With the light theme on, open a diff with an added row, a removed row and a changed word. Look at the numbers and the code on each, and at the red, green and yellow of the marks. Open a terminal pane and run a command that prints the sixteen colours. Look at the terminal's empty cells and at the space right of its last column." \
    "The line numbers and the code are readable on all three rows. The marks read clearly. The terminal's ground and its sixteen colours follow the theme, and its empty cells and the space right of the last column show the terminal's ground."
  ck_item 17 "Hide, Hide Others, Minimize" \
    "Press Cmd-H, then bring the app back; Cmd-Alt-H; Cmd-M. Then the same three by their menu items." \
    "Cmd-H hides YAAR, Cmd-Alt-H hides the other apps, Cmd-M minimises the window, and the menu items Hide YAAR, Hide Others and Minimize do the same."
}

items_linux() {
  ck_item 1 "The app menu of the desktop" \
    "Open the desktop's application menu and look for YAAR. Start it from there." \
    "YAAR is listed with its icon and starts with no terminal."
  ck_item 2 "The running icon" \
    "With the app running, look at the dock or the panel." \
    "The window groups under the YAAR icon, not under a generic one."
  ck_item 3 "The window's frame" \
    "Move, resize, maximise and tile the window, with the mouse and with the desktop's keys." \
    "The title bar and buttons are as in other apps on that desktop, and each action works."
  ck_item 4 "Quit and close" \
    "Press Ctrl-Q. Start again and close the window by its button." \
    "Ctrl-Q is the plain quit, and closing the window by its button does the same."
  ck_item 5 "Copy and paste in a terminal" \
    "In a terminal pane: select text, Ctrl-Shift-C, Ctrl-Shift-V; select text and press the middle button." \
    "The copy and the paste work, and a selection pastes with the middle button."
  ck_item 6 "Copy and paste in a comment box" \
    "In a comment box: Ctrl-C, Ctrl-V, Ctrl-X, Ctrl-A, Ctrl-Z." \
    "Each does what it does in other apps."
  ck_item 7 "Light and dark" \
    "With no theme chosen, switch the desktop between light and dark while the app runs." \
    "The app follows the desktop's setting as it changes."
  ck_item 8 "Input" \
    "In a comment box and in a terminal pane: type a compose sequence and use an input method of the desktop." \
    "Both work in both."
  ck_item 9 "Scaling" \
    "Set the display scale to 200% and to a fractional scale, and look at the whole window." \
    "Text is sharp and nothing is cut."
  ck_item 10 "Where files go" \
    "List the app's files in the home folder: settings, data, state and the socket (ls ~/.config ~/.local/share ~/.local/state, and the runtime folder)." \
    "They lie in the folders the desktop standard names. Nothing lies loose in the home folder."
  ck_item 11 "Keys the desktop owns" \
    "With a terminal pane focused: the Super key and the desktop's switcher." \
    "Each does what it does in every app."
}

if [ "$LIST" = macos ]; then
  TITLE="CONV-01: the macOS checklist"
  STATEMENT="I accept these as following the platform (CONV-01, macOS)"
else
  TITLE="CONV-02: the Linux checklist"
  STATEMENT="I accept these as following the platform (CONV-02, Linux)"
fi

ck_begin
ck_walker_context "$DRY"

printf '%s\n' "$TITLE"
printf 'Open the app yourself and walk each item in it, then answer here.\n'
ck_walk "items_$LIST"
ck_signoff "$STATEMENT"

RESULTS=$(ck_results_dir)
if [ "$DRY" -eq 1 ]; then
  OUT="$RESULTS/conventions-$LIST-dry-run.md"
else
  OUT="$RESULTS/conventions-$LIST.md"
fi
ck_write "$OUT" "$TITLE" "$CK_SIGN"
printf '\nWritten to %s\n' "$OUT"
