#!/bin/bash
# Row PKG-08 on macOS: the installed app starts from the Dock with no terminal, as an
# application. Runs on GitHub's Mac only: it opens a window, which no agent may do on the
# owner's Mac.
#
#   .github/scripts/macos-launch.sh
#
# After the install line put the app in /Applications:
#   1. `open -a /Applications/YAAR.app`, which asks LaunchServices to start it, as a click in
#      the Dock or the Finder does; `open` itself returns at once.
#   2. Waits up to 30 s for a process whose command is
#      /Applications/YAAR.app/Contents/MacOS/yaar-review, and one whose command is
#      /Applications/YAAR.app/Contents/Helpers/yaar-keeper.
#   3. Asserts: the Window's parent is launchd (`ps -o ppid=` is 1); it has no controlling
#      terminal (`ps -o tty=` is "??"); the entry of `lsappinfo list` for the bundle
#      io.github.tsadoq.yaar says type="Foreground" (an app with a Dock icon, not a
#      background agent);
#      `/Applications/YAAR.app/Contents/MacOS/yaar-review --keeper-status` says
#      started=false and names the Keeper's pid above; `codesign --verify` of the running
#      Keeper's pid passes.
#   4. Ends the app with SIGTERM to the Window and waits for both processes to be gone (the
#      Keeper lingers 5 s; waits 20 s).
# On failure prints the processes of the user, their parents and terminals, and the last 50
# lines of `log show --last 2m --predicate 'process == "yaar-review"'`.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# What the header leaves, decided here: (1) the step refuses (exit 2) unless GITHUB_ACTIONS
# is "true", because it opens a window, which no agent may do on the owner's Mac; (2) it
# fails, before it opens anything, when a Window or a Keeper of the bundle runs already, so
# that the process it asserts about is the one `open` started; (3) the entry of
# `lsappinfo list` is read as the block that starts at a line "<n>) "<name>" ASN:..." and
# holds bundleID="io.github.tsadoq.yaar" (the quotes keep the Keeper's own identifier from
# matching), and LaunchServices registers an app a moment after its process starts, so the
# step polls the entry's type="..." up to 30 s; (4) the Window's and the Keeper's processes
# are the user's processes whose full program path is the bundle's, from
# `ps -axo uid=,pid=,ppid=,comm=`; (5) the Keeper's own terminal is recorded, not asserted;
# (6) the app is ended by the trap too, by the pids found, when a step fails half way; (7) a
# command that stops the script without a sentence of its own is named in one, and the exit
# code is 1.
set -euo pipefail
# -E: the ERR trap below also runs inside functions and command substitutions.
set -E

PROGRAM=macos-launch.sh
APP=/Applications/YAAR.app
WINDOW="$APP/Contents/MacOS/yaar-review"
KEEPER="$APP/Contents/Helpers/yaar-keeper"
BUNDLE_ID=io.github.tsadoq.yaar

# What the step was doing when a command that nobody guarded failed: set -e ends the step
# with that command's own status and words, and the exit trap puts one sentence of the
# step's own in front of them and exits 1, the code the header names.
said=no
last_command=""
trap 'last_command=$BASH_COMMAND' ERR

fail() {
    said=yes
    echo "$PROGRAM: $*" >&2
    exit 1
}

usage() {
    said=yes
    echo "$PROGRAM: $*" >&2
    echo "usage: .github/scripts/macos-launch.sh" >&2
    exit 2
}

[ "$#" -eq 0 ] || usage "no argument is taken"
[ "$(uname -s)" = Darwin ] || usage "it runs on macOS, and this is $(uname -s)"
[ "${GITHUB_ACTIONS:-}" = true ] || usage "it runs on GitHub's Mac only: it opens a window, and GITHUB_ACTIONS is not true"
for tool in open ps awk lsappinfo codesign log kill; do
    command -v "$tool" > /dev/null || fail "$tool is not there"
done
[ -x "$WINDOW" ] || fail "$WINDOW is not there: the install step did not put the app in /Applications"
[ -x "$KEEPER" ] || fail "$KEEPER is not there: the app holds no Keeper"
uid=$(id -u)

# The pids of the Window and of the Keeper this step found, which the trap ends.
window_pid=""
keeper_pid=""
# What the last look at the system saw, for the sentence of a wait that ran out.
seen=""

# The processes of this user whose program is the file $1, as "<pid> <parent pid>" lines.
processes_of() {
    ps -axo uid=,pid=,ppid=,comm= | awk -v uid="$uid" -v want="$1" '
        $1 == uid {
            program = $4
            for (i = 5; i <= NF; i++) program = program " " $i
            if (program == want) print $2, $3
        }'
}

# Whether the process $1 is gone.
is_gone() {
    ! kill -0 "$1" 2> /dev/null
}

# Waits up to $1 seconds until the function $3 succeeds. The function leaves what it saw in
# $seen; without success the step fails with the sentence $2 and that.
wait_until() {
    local limit=$((SECONDS + $1))
    while :; do
        if "$3"; then
            return 0
        fi
        [ "$SECONDS" -lt "$limit" ] || fail "$2 within $1 s: $seen"
        sleep 0.2
    done
}

# The type of the application whose bundle identifier is $1, as `lsappinfo list` prints it
# (Foreground for an app with a Dock icon, UIElement or BackgroundOnly for an agent), or
# nothing when no entry of that bundle is listed. An entry starts at a line `<n>) "<name>"
# ASN:<asn>:`, names its bundle on a line bundleID="<id>" and its type on the line of its
# pid, type="<type>".
app_type() {
    lsappinfo list | awk -v id="$1" '
        /^ *[0-9]+\) / { hit = 0 }
        index($0, "bundleID=\"" id "\"") { hit = 1 }
        hit && match($0, /type="[A-Za-z]+"/) {
            print substr($0, RSTART + 6, RLENGTH - 7)
            exit
        }'
}

# What explains a failure: the processes of the user with their parents and terminals, and
# the last lines the Window logged.
diagnose() {
    {
        echo "---- the processes of uid $uid: pid, parent, terminal, command"
        ps -U "$uid" -o pid=,ppid=,tty=,command= || true
        echo "---- lsappinfo, the entry of $BUNDLE_ID: type '$(app_type "$BUNDLE_ID" || true)'"
        echo "---- the last 50 lines of: log show --last 2m --predicate 'process == \"yaar-review\"'"
        log show --last 2m --predicate 'process == "yaar-review"' 2>&1 | tail -n 50 || true
    } >&2
}

# Ends what the step started, on success and on failure alike, and says what failed when no
# sentence was said.
cleanup() {
    local status=$? pid limit
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$status" -ne 2 ]; then
        if [ "$said" = no ]; then
            echo "$PROGRAM: ${last_command:-the step} failed with status $status" >&2
            status=1
        fi
        diagnose
    fi
    for pid in $window_pid $keeper_pid; do
        if ! is_gone "$pid"; then
            kill "$pid" 2> /dev/null
            limit=$((SECONDS + 10))
            while ! is_gone "$pid" && [ "$SECONDS" -lt "$limit" ]; do sleep 0.2; done
            is_gone "$pid" || kill -KILL "$pid" 2> /dev/null
        fi
    done
    exit "$status"
}
trap cleanup EXIT

# A Window or a Keeper that runs already would be the one asserted about, and not the one
# `open` starts.
for program in "$WINDOW" "$KEEPER"; do
    before=$(processes_of "$program") || fail "the processes of uid $uid could not be listed"
    [ -z "$before" ] || fail "$program runs before the step opened the app (pid, parent: $(printf '%s' "$before" | tr '\n' ';')): an earlier step left it"
done

# --- 1. LaunchServices starts the app, as a click in the Dock or the Finder does.
open -a "$APP" || fail "open -a $APP failed: the system did not start the app"
echo "step 1: open -a $APP"

# --- 2. The Window and its Keeper run.
windows=""
keepers=""
both_run() {
    local listed
    listed=$(processes_of "$WINDOW") || fail "the processes of uid $uid could not be listed"
    windows=$(printf '%s' "$listed" | tr '\n' ';')
    listed=$(processes_of "$KEEPER") || fail "the processes of uid $uid could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ';')
    seen="windows (pid, parent): ${windows:-none}; keepers: ${keepers:-none}"
    [ -n "$windows" ] && [ -n "$keepers" ]
}
wait_until 30 "the Window $WINDOW and the Keeper $KEEPER did not both run" both_run
[ "$(processes_of "$WINDOW" | wc -l | tr -d ' ')" -eq 1 ] || fail "more than one Window of the bundle runs: $windows"
[ "$(processes_of "$KEEPER" | wc -l | tr -d ' ')" -eq 1 ] || fail "more than one Keeper of the bundle runs: $keepers"
window_pid=$(processes_of "$WINDOW" | cut -d' ' -f1)
window_parent=$(processes_of "$WINDOW" | cut -d' ' -f2)
keeper_pid=$(processes_of "$KEEPER" | cut -d' ' -f1)
echo "step 2: the Window $window_pid and the Keeper $keeper_pid run"

# --- 3. An application, started by the system, with no terminal.
[ "$window_parent" = 1 ] || fail "the Window $window_pid was started by process $window_parent and not by launchd (1)"
window_tty=$(ps -o tty= -p "$window_pid") || fail "ps could not read the terminal of the Window $window_pid"
window_tty=$(printf '%s' "$window_tty" | tr -d ' ')
[ "$window_tty" = '??' ] || fail "the Window $window_pid has a controlling terminal, $window_tty, and not none (??)"
keeper_tty=$(ps -o tty= -p "$keeper_pid" | tr -d ' ')
type_seen=""
foreground() {
    type_seen=$(app_type "$BUNDLE_ID") || fail "lsappinfo list failed"
    seen="the entry of $BUNDLE_ID in lsappinfo list has type '${type_seen:-none: no such entry}'"
    [ "$type_seen" = Foreground ]
}
wait_until 30 "LaunchServices did not list $BUNDLE_ID as a Foreground application" foreground
status_line=$("$WINDOW" --keeper-status 2>&1) || fail "$WINDOW --keeper-status failed: $status_line"
case "$status_line" in
    *started=false*) ;;
    *) fail "--keeper-status does not say started=false: the app's own start did not start the Keeper: $status_line" ;;
esac
status_pid=$(printf '%s\n' "$status_line" | tr ' ' '\n' | sed -n 's/^pid=//p')
[ "$status_pid" = "$keeper_pid" ] || fail "--keeper-status names the Keeper pid '$status_pid', not the pid $keeper_pid of the process the system started"
codesign --verify --verbose=1 "$keeper_pid" || fail "codesign --verify does not pass on the running Keeper, pid $keeper_pid"
echo "step 3: the Window's parent is launchd, its terminal is none (??), $BUNDLE_ID is a $type_seen application, --keeper-status names the Keeper $keeper_pid with started=false, the running Keeper's signature verifies; the Keeper's terminal is '${keeper_tty:-unknown}'"

# --- 4. The end: SIGTERM to the Window, then both are gone.
kill -TERM "$window_pid" || fail "the Window $window_pid was gone before the step ended it"
both_gone() {
    seen=""
    is_gone "$window_pid" || seen="the Window $window_pid"
    is_gone "$keeper_pid" || seen="${seen:+$seen and }the Keeper $keeper_pid"
    [ -z "$seen" ] || seen="still running: $seen"
    [ -z "$seen" ]
}
wait_until 20 "the Window and the Keeper did not both end after SIGTERM to the Window" both_gone
echo "step 4: the Window and the Keeper are gone"
