#!/bin/bash
# The installed app starts from the Dock with no terminal; GitHub's Mac only (exit 2 otherwise).
# .github/scripts/macos-launch.sh  Refuses to start when a Window or Keeper of the bundle runs.
# --keeper-status runs in the background so a silent Window fails in a sentence, not at timeout.
set -euo pipefail
set -E

PROGRAM=macos-launch.sh
APP=/Applications/YAAR.app
WINDOW="$APP/Contents/MacOS/yaar-review"
KEEPER="$APP/Contents/Helpers/yaar-keeper"
BUNDLE_ID=io.github.tsadoq.yaar

said=no
last_command=""
trap 'last_command=$BASH_COMMAND' ERR

fail() {
    said=yes
    echo "$PROGRAM: $*" >&2
    exit 1
}

usage_said=no
usage() {
    usage_said=yes
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

window_pid=""
keeper_pid=""
status_dir=$(mktemp -d) || fail "no scratch folder could be made"
status_pid_of_query=""
seen=""

processes_of() {
    ps -axo uid=,pid=,ppid=,comm= | awk -v uid="$uid" -v want="$1" '
        $1 == uid {
            program = $4
            for (i = 5; i <= NF; i++) program = program " " $i
            if (program == want) print $2, $3
        }'
}

is_gone() {
    ! kill -0 "$1" 2> /dev/null
}

# wait_until <seconds> <sentence> <function>: runs the function every 0.2 s until it succeeds,
# else fails with the sentence and what the function left in $seen.
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

# lsappinfo type of bundle id $1: Foreground (Dock icon), UIElement or BackgroundOnly, or nothing.
app_type() {
    local list
    list=$(lsappinfo list) || return 1
    # awk reads the whole list: stopping at the first match would leave the writer a closed pipe and
    # SIGPIPE, which pipefail turns into a failure.
    printf '%s\n' "$list" | awk -v id="$1" '
        /^ *[0-9]+\) / { hit = 0 }
        index($0, "bundleID=\"" id "\"") { hit = 1 }
        hit && found == "" && match($0, /type="[A-Za-z]+"/) {
            found = substr($0, RSTART + 6, RLENGTH - 7)
        }
        END { if (found != "") print found }'
}

diagnose() {
    {
        echo "---- the processes of uid $uid: pid, parent, terminal, command"
        ps -U "$uid" -o pid=,ppid=,tty=,command= || true
        echo "---- lsappinfo, the entry of $BUNDLE_ID: type '$(app_type "$BUNDLE_ID" || true)'"
        echo "---- the last 50 lines of: log show --last 2m --predicate 'process == \"yaar-review\"'"
        log show --last 2m --predicate 'process == "yaar-review"' 2>&1 | tail -n 50 || true
    } >&2
}

cleanup() {
    local status=$? pid limit
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$usage_said" = no ]; then
        if [ "$said" = no ]; then
            echo "$PROGRAM: ${last_command:-the step} failed with status $status" >&2
            status=1
        fi
        diagnose
    fi
    for pid in "$window_pid" "$keeper_pid" "$status_pid_of_query"; do
        [ -n "$pid" ] || continue
        if ! is_gone "$pid"; then
            kill "$pid" 2> /dev/null
            limit=$((SECONDS + 10))
            while ! is_gone "$pid" && [ "$SECONDS" -lt "$limit" ]; do sleep 0.2; done
            is_gone "$pid" || kill -KILL "$pid" 2> /dev/null
        fi
    done
    rm -rf "$status_dir"
    exit "$status"
}
trap cleanup EXIT

# A Window or Keeper already running would be the one asserted about, not the one `open` starts.
for program in "$WINDOW" "$KEEPER"; do
    before=$(processes_of "$program") || fail "the processes of uid $uid could not be listed"
    [ -z "$before" ] || fail "$program runs before the step opened the app (pid, parent: $(printf '%s' "$before" | tr '\n' ';')): an earlier step left it"
done

open -a "$APP" || fail "open -a $APP failed: the system did not start the app"
echo "step 1: open -a $APP"

windows=""
keepers=""
both_run() {
    local listed
    listed=$(processes_of "$WINDOW") || fail "the processes of uid $uid could not be listed"
    windows=$(printf '%s' "$listed" | tr '\n' ';')
    # The pid is kept as soon as seen so the trap ends a Window that came up alone.
    [ -z "$listed" ] || window_pid=${listed%% *}
    listed=$(processes_of "$KEEPER") || fail "the processes of uid $uid could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ';')
    [ -z "$listed" ] || keeper_pid=${listed%% *}
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
# Detached with nohup (macOS has no setsid; it execs the program, so $! is the query's own pid).
nohup "$WINDOW" --keeper-status > "$status_dir/out" 2>&1 < /dev/null &
status_pid_of_query=$!
query_ended() {
    seen="$WINDOW --keeper-status has printed: $(tr '\n' ' ' < "$status_dir/out")"
    is_gone "$status_pid_of_query"
}
wait_until 30 "$WINDOW --keeper-status did not end" query_ended
query_status=0
wait "$status_pid_of_query" || query_status=$?
status_pid_of_query=""
status_line=$(cat "$status_dir/out")
[ "$query_status" -eq 0 ] || fail "$WINDOW --keeper-status failed with status $query_status: $status_line"
case "$status_line" in
    *started=false*) ;;
    *) fail "--keeper-status does not say started=false: the app's own start did not start the Keeper: $status_line" ;;
esac
status_pid=$(printf '%s\n' "$status_line" | tr ' ' '\n' | sed -n 's/^pid=//p')
[ "$status_pid" = "$keeper_pid" ] || fail "--keeper-status names the Keeper pid '$status_pid', not the pid $keeper_pid of the process the system started"
codesign --verify --verbose=1 "$keeper_pid" || fail "codesign --verify does not pass on the running Keeper, pid $keeper_pid"
echo "step 3: the Window's parent is launchd, its terminal is none (??), $BUNDLE_ID is a $type_seen application, --keeper-status names the Keeper $keeper_pid with started=false, the running Keeper's signature verifies; the Keeper's terminal is '${keeper_tty:-unknown}'"

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
