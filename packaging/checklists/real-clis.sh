#!/bin/bash
# Walks the checks of the real Claude Code and pi in terminals of the packaged app.
#
# packaging/checklists/real-clis.sh [--dry-run]
#
# Starts nothing of the CLIs: they are the owner's logins and money, each check is one run of his.
# Note which CLI dialogs the app is not told of, and whether pi can resume one of two sessions of
# a worktree. Needs both CLIs logged in, the app on a Mac, a repository, and a folder
# neither CLI has run in. Exit: 0 written, 1 could not be walked or written, 2 usage.
set -euo pipefail

CK_NAME=real-clis.sh
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

items_clis() {
  ck_item LOAD-01 "claude in a shell of the app" \
    "In a terminal pane of the app, type: claude" \
    "The app's hooks run and its tool is listed."
  ck_item LOAD-02 "After /clear, --resume and a sub-agent" \
    "In claude: type /clear. Quit and run: claude --resume. Give it a task that starts a sub-agent that edits a file." \
    "After each, the app's hooks still run and the earlier review is taken over. The sub-agent's edits are credited to its parent."
  ck_item LOAD-03 "pi in a shell of the app" \
    "In a terminal pane of the app, type each: pi, pi --continue, pi list, pi mcp list. Answer yes when the app asks about its extension." \
    "They behave as without the app, and after the yes the first two load the extension."
  ck_item LOAD-04 "No change by the app after a session" \
    "After a session of each CLI: run git status --ignored in the repository, and look at each CLI's settings file. Compare with what they were before." \
    "The app changed nothing but, after your yes, the one entry it put into pi's settings. macOS asked for no keychain access on the app's behalf."
  ck_item LOAD-05 "The trust dialog" \
    "In a folder that makes the CLI show its trust dialog, start the CLI in a terminal pane of the app. Wait before accepting." \
    "No event reaches the app before you accept, and the app sends no key to the terminal."
  ck_item LOAD-07 "pi in another terminal program" \
    "After the yes, start pi in a terminal program that is not the app." \
    "pi loads the app's extension, which registers no tool, prints nothing and reports nothing."
  ck_item LOAD-08 "A subagent of pi" \
    "After the yes, in a terminal pane of the app, give pi a task that starts a subagent that writes a file." \
    "The subagent shows as a session of its own, and its writes are credited to it."
  ck_item CLI-01 "One live turn each" \
    "In each CLI, one turn that reads a file, edits a file, makes a new file and writes by a shell command. Look at Flow." \
    "Flow shows the same steps as the recorded ones, and the shape of the events equals the recorded ones the tests replay."
  ck_item CLI-02 "A permission dialog" \
    "In each CLI, make it ask for a permission. Answer it." \
    "The waiting state turns on within 100 ms of the dialog and off when you answer."
  ck_item CLI-03 "Esc during a turn" \
    "In each CLI, press Esc while a turn runs." \
    "The turn ends in the app, in both CLIs. (Never run with pi: its login had expired.)"
  ck_item CLI-04 "A review attached while the agent works" \
    "In each CLI, attach a review while the agent works on a turn." \
    "The review goes with the queued message, in both CLIs. (Never run with pi: its login had expired.)"
  ck_item CLI-05 "Resume after a restart" \
    "With two sessions of each CLI in one worktree, restart the Mac, open the app, and use the resume control of each session. For pi the command for one of two sessions is: pi --session-dir <its folder> --session <id>" \
    "Each control brings back its own session, in both CLIs."
  ck_item DELIVER-02 "A review of 200,000 characters" \
    "Make a review of 200,000 characters with a fact planted at each of five places, send it, and ask the agent for the five facts. Do it in both CLIs." \
    "All five come back, with no permission dialog, in both CLIs."
  ck_item REPLY-01 "Five threads, 9,000 characters" \
    "Make a review of 9,000 characters with 5 threads, send it with the message: Address each comment. Then once more with a one-word message. Do it in both CLIs." \
    "5 replies arrive, each on its thread, with no permission dialog, in both CLIs. After the one-word message the instruction on how to answer is still followed."
  ck_item TERM-05 "The CLIs in the terminal pane" \
    "In each CLI, in the app's terminal pane: a new line without submit, submit, interrupt, history, paste, the mouse, resize, quit." \
    "Each works as in Ghostty, in both CLIs."
}

ck_begin
ck_walker_context "$DRY"
printf 'The real CLIs. Run each check once, in terminals of the app, and answer here.\n'
ck_walk items_clis
ck_signoff "I accept these checks as passed with the real Claude Code and pi"

RESULTS=$(ck_results_dir)
if [ "$DRY" -eq 1 ]; then
  OUT="$RESULTS/real-clis-dry-run.md"
else
  OUT="$RESULTS/real-clis.md"
fi
ck_write "$OUT" "The real Claude Code and pi" "$CK_SIGN"
printf '\nWritten to %s\n' "$OUT"
