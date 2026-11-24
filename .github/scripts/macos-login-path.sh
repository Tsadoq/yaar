#!/bin/bash
# App started by the system: a worktree's post-checkout hook runs a program found only on the user's
# login path. GitHub's Mac only (opens a window, writes the runner's start-up files).
# .github/scripts/macos-login-path.sh <test-programs-dir> (holds standin-agent and yaar-script)
set -euo pipefail
set -E

HERE=$(cd "$(dirname "$0")" && pwd)
PROGRAM=macos-login-path.sh
APP=/Applications/YAAR.app
WINDOW="$APP/Contents/MacOS/yaar-review"
KEEPER="$APP/Contents/Helpers/yaar-keeper"
BRANCH=from-the-dock

scratch=""
# usage_said: set by usage alone, the one place that may leave with status 2.
said=no
usage_said=no
last_command=""
last_line=""
trap 'last_command=$BASH_COMMAND
    if [ "${#BASH_LINENO[@]}" -gt 1 ]; then last_line=${BASH_LINENO[${#BASH_LINENO[@]}-2]}; else last_line=$LINENO; fi' ERR

fail() {
    said=yes
    echo "$PROGRAM: $*" >&2
    if [ -n "$scratch" ]; then
        : > "$scratch/said"
    fi
    exit 1
}

usage() {
    usage_said=yes
    said=yes
    echo "$PROGRAM: $*" >&2
    echo "usage: .github/scripts/macos-login-path.sh <test-programs-dir>" >&2
    exit 2
}

user=""
uid=""
home=""
programs=""
only=""
repository=""
worktrees=""
window_pid=""
keeper_pid=""
standin_pids=""
startup=""
startup_changed=no
startup_existed=no
# Which folders this run made: a folder a former run left is not this run's to remove.
made_only=no
made_repository=no
made_worktrees=no

plain_git() {
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git "$@"
}

# Matched by name, not by full path: the program the shell starts for claude is a link to the
# stand-in.
claude_below() {
    ps -axo uid=,pid=,ppid=,command= | awk -v uid="$uid" -v root="$1" '
        $1 == uid { parent[$2] = $3; name[$2] = $4 }
        END {
            for (pid in parent) {
                n = split(name[pid], part, "/")
                if (part[n] != "claude") continue
                for (up = parent[pid]; up > 1; up = parent[up]) {
                    if (up == root) { print pid; break }
                }
            }
        }'
}

end_process() {
    local pid=$1 limit=$((SECONDS + 10))
    [ -n "$pid" ] || return 0
    is_gone "$pid" && return 0
    kill "$pid" 2> /dev/null || true
    while [ "$SECONDS" -lt "$limit" ]; do
        is_gone "$pid" && return 0
        sleep 0.2
    done
    kill -KILL "$pid" 2> /dev/null || true
    return 0
}

seen=""
# shellcheck source=wait-until.sh
. "$HERE/wait-until.sh"
# shellcheck source=macos-processes.sh
. "$HERE/macos-processes.sh"

# Asks the login shell as the Keeper does (crates/shared/link/src/login.rs): -l -i -c, stdin from
# /dev/null, launchd's search path in an otherwise bare environment, a 10 s deadline. -i makes zsh
# read ~/.zshrc after ~/.zprofile, so a ~/.zshrc that sets PATH outright shows here.
found=""
ask_the_login_shell() {
    local out="$scratch/probe.out" pid limit=$((SECONDS + 10)) raw
    : > "$out"
    (
        cd "$home" || exit 1
        # The Keeper hands the shell the environment launchd gave it: HOME, PATH, the user's name,
        # the shell and the temporary folder, which a start-up file may read.
        me=$(id -un)
        exec env -i HOME="$home" PATH="${launchd_path:-/usr/bin:/bin:/usr/sbin:/sbin}" \
            USER="$me" LOGNAME="$me" SHELL="$shell" TMPDIR="$(getconf DARWIN_USER_TEMP_DIR)" "$shell" -l -i -c \
            'printf "yaar-probe-follows:"; command -v yaar-hook-witness' < /dev/null > "$out" 2> /dev/null
    ) &
    pid=$!
    while ! is_gone "$pid"; do
        if [ "$SECONDS" -ge "$limit" ]; then
            # What a start-up file started and waits for goes with the shell.
            pkill -KILL -P "$pid" 2> /dev/null || true
            kill -KILL "$pid" 2> /dev/null || true
            wait "$pid" 2> /dev/null || true
            fail "the login shell of $user, $shell, asked as the Keeper asks it (-l -i -c, standard input from /dev/null), did not answer within 10 s, the deadline the Keeper gives it: a start-up file waits for something"
        fi
        sleep 0.2
    done
    wait "$pid" 2> /dev/null || true
    raw=$(cat "$out")
    case $raw in
        *yaar-probe-follows:*) found=${raw##*yaar-probe-follows:}; found=${found%%$'\n'*} ;;
        *) found="" ;;
    esac
}

diagnose() {
    {
        echo "---- the processes of ${user:-nobody yet}"
        if [ -n "$uid" ]; then ps -U "$uid" -o pid,ppid,stat,etime,command || true; fi
        if [ -n "$scratch" ]; then
            echo "---- the output of yaar-script, $scratch/script.log"
            cat "$scratch/script.log" 2> /dev/null || true
            echo "---- the witness file, $only/witness"
            cat "$only/witness" 2> /dev/null || echo "(there is none)"
        fi
    } >&2
}

cleanup() {
    local status=$? pid
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$usage_said" = no ]; then
        if [ "$said" = no ] && [ ! -e "$scratch/said" ]; then
            failed_text=""
            if [ -n "$last_line" ]; then
                failed_text=$(sed -n "${last_line}s/^[[:space:]]*//p" "$0" 2> /dev/null)
            fi
            echo "$PROGRAM: ${failed_text:-${last_command:-the step}} failed with status $status${last_line:+ (line $last_line)}" >&2
            status=1
        fi
        diagnose
    fi
    end_process "$window_pid"
    end_process "$keeper_pid"
    for pid in $standin_pids; do end_process "$pid"; done
    if [ "$startup_changed" = yes ]; then
        if [ "$startup_existed" = yes ]; then
            cat "$scratch/startup.original" > "$startup" \
                || echo "$PROGRAM: $startup could not be put back as it was" >&2
        else
            rm -f "$startup" || echo "$PROGRAM: $startup could not be removed" >&2
        fi
    fi
    [ "$made_only" = no ] || rm -rf "$only"
    [ "$made_repository" = no ] || rm -rf "$repository"
    [ "$made_worktrees" = no ] || rm -rf "$worktrees"
    rm -rf "$scratch"
    exit "$status"
}
trap cleanup EXIT

[ "$#" -eq 1 ] || usage "one argument is needed, the folder of the test programs"
[ "$(uname -s)" = Darwin ] || usage "it runs on macOS, and this is $(uname -s)"
[ -n "${RUNNER_TEMP:-}" ] || usage "RUNNER_TEMP is not set; the start-up file is copied there"
[ -d "$RUNNER_TEMP" ] || usage "RUNNER_TEMP is $RUNNER_TEMP, which is not a folder"
[ -d "$1" ] || usage "$1 is not a folder"
programs=$(cd "$1" && pwd -P) || fail "$1 could not be entered"
for tool in dscl launchctl open git ps awk; do
    command -v "$tool" > /dev/null || fail "$tool is not there"
done
for program in standin-agent yaar-script; do
    [ -f "$programs/$program" ] || fail "$programs/$program is missing"
    chmod +x "$programs/$program" || fail "$programs/$program could not be made executable"
done
[ -x "$WINDOW" ] || fail "$WINDOW is not there: the install step did not put the app in /Applications"
[ -x "$KEEPER" ] || fail "$KEEPER is not there: the app holds no Keeper"
user=$(id -un) || fail "the name of the user could not be read (id -un failed)"
uid=$(id -u) || fail "the number of the user could not be read (id -u failed)"
[ -d "${HOME:-}" ] || fail "HOME is not a folder"
home=$(cd "$HOME" && pwd -P) || fail "HOME, $HOME, could not be entered"
shell=$(dscl . -read "/Users/$user" UserShell) || fail "dscl could not read the login shell of $user"
shell=$(printf '%s\n' "$shell" | sed -n 's/^UserShell: //p')
[ -n "$shell" ] || fail "dscl gave no login shell for $user"

only="$home/yaar-login-only"
repository="$home/yaar-login-repo"
worktrees="$home/yaar-login-repo.worktrees"
scratch=$(mktemp -d "$RUNNER_TEMP/login-path.XXXXXX") || fail "no scratch folder could be made under $RUNNER_TEMP"

[ ! -e "$only" ] || fail "$only is there already: a former run did not end"
[ ! -e "$repository" ] || fail "$repository is there already: a former run did not end"
[ ! -e "$worktrees" ] || fail "$worktrees is there already: a former run did not end"

made_only=yes
made_repository=yes
made_worktrees=yes
mkdir -p "$only/bin"
cat > "$only/bin/yaar-hook-witness" << EOF
#!/bin/sh
printf '%s %s\n' "\$PWD" "\$PATH" >> '$only/witness'
EOF
chmod +x "$only/bin/yaar-hook-witness"
ln -s "$programs/standin-agent" "$only/bin/claude"

launchd_path=$(launchctl getenv PATH) || fail "launchctl getenv PATH failed, so launchd's search path cannot be looked at"
for path in "$launchd_path" "$PATH"; do
    case ":$path:" in
        *":$only/bin:"*) fail "a search path of the runner holds $only/bin: $path" ;;
    esac
done

case "${shell##*/}" in
    zsh) startup="$home/.zprofile" ;;
    bash) startup="$home/.bash_profile" ;;
    *) fail "the login shell of $user is ${shell##*/}, and only zsh and bash are known here" ;;
esac
if [ -e "$startup" ]; then
    startup_existed=yes
    cp "$startup" "$scratch/startup.original"
fi
# shellcheck disable=SC2016 # the line is written as it is: $HOME and $PATH are the login shell's
printf '\nexport PATH="$HOME/yaar-login-only/bin:$PATH"\n' >> "$startup" \
    || fail "the line that puts $only/bin on the login path could not be appended to $startup, which stays as it was"
startup_changed=yes
ask_the_login_shell
[ "$found" = "$only/bin/yaar-hook-witness" ] || fail "the login shell of $user, $shell, asked as the Keeper asks it (-l -i -c), finds yaar-hook-witness at '$found' and not at $only/bin/yaar-hook-witness: the line in $startup does not work, or a start-up file the shell reads after it sets PATH again (for zsh, -i adds ~/.zshrc after ~/.zprofile), and then the Keeper takes no login path either"
echo "step 1: $only/bin holds yaar-hook-witness and claude; neither launchd's path nor the runner's holds it; the login shell $shell finds it through $startup"

plain_git init --quiet --initial-branch=main "$repository"
plain_git -C "$repository" config user.name "A Test"
plain_git -C "$repository" config user.email "test@example.invalid"
echo one > "$repository/a.txt"
plain_git -C "$repository" add a.txt
plain_git -C "$repository" commit --quiet -m "first"
printf '#!/bin/sh\nexec yaar-hook-witness\n' > "$repository/.git/hooks/post-checkout"
chmod +x "$repository/.git/hooks/post-checkout"
echo "step 2: $repository on main with a post-checkout hook that runs yaar-hook-witness"

# A Keeper an earlier step left would answer with the environment it took then: wait until none
# runs. These checks run from `if`, where set -e is off, so a failed listing fails the step by hand.
no_keeper_runs() {
    local listed
    listed=$(processes_of "$KEEPER") || fail "the processes of $user could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ';')
    seen="a Keeper runs as $user (pid, parent): ${keepers:-none}"
    [ -z "$keepers" ]
}
keepers=""
wait_until 20 "a Keeper of an earlier step did not leave before the app was started" no_keeper_runs
open -a "$APP" || fail "open -a $APP failed: the system did not start the app"
windows=""
both_run() {
    local listed
    listed=$(processes_of "$WINDOW") || fail "the processes of $user could not be listed"
    windows=$(printf '%s' "$listed" | tr '\n' ';')
    listed=$(processes_of "$KEEPER") || fail "the processes of $user could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ';')
    seen="windows (pid, parent): ${windows:-none}; keepers: ${keepers:-none}"
    [ -n "$windows" ] && [ -n "$keepers" ]
}
wait_until 30 "the Window $WINDOW and its Keeper $KEEPER did not both run" both_run
[ "$(processes_of "$WINDOW" | wc -l | tr -d ' ')" -eq 1 ] || fail "more than one Window of the bundle runs: $windows"
[ "$(processes_of "$KEEPER" | wc -l | tr -d ' ')" -eq 1 ] || fail "more than one Keeper of the bundle runs: $keepers"
window_pid=$(processes_of "$WINDOW" | cut -d' ' -f1)
window_parent=$(processes_of "$WINDOW" | cut -d' ' -f2)
keeper_pid=$(processes_of "$KEEPER" | cut -d' ' -f1)
# ps -E shows no environment on a recent macOS: that no shell started the app rests on the parent
# being launchd and neither program being a script; a compiled wrapper is not caught.
[ "$window_parent" = 1 ] || fail "the Window $window_pid was started by process $window_parent and not by launchd (1)"
for program in "$WINDOW" "$KEEPER"; do
    magic=$(head -c 2 "$program" | od -An -tx1 | tr -d ' \n') || fail "the first bytes of $program could not be read"
    [ "$magic" != 2321 ] || fail "$program is a script and not a program: it could start the app through the login shell, and the hook would find its program without the Keeper's take"
done
echo "step 3: the Window $window_pid (parent launchd) and the Keeper $keeper_pid run, started by open -a $APP; neither program of the bundle is a script"

"$programs/yaar-script" worktree-with-agent --repository "$repository" \
    --branch "$BRANCH" --agent claude --deadline 60 > "$scratch/script.log" 2>&1 \
    || fail "yaar-script worktree-with-agent failed: $(cat "$scratch/script.log")"
line=$(cat "$scratch/script.log")
case "$line" in
    worktree=/*" workspace="*" session="*) ;;
    *) fail "yaar-script printed '$line' and not 'worktree=<folder> workspace=<id> session=<id>'" ;;
esac
worktree=${line#worktree=}
worktree=${worktree%% workspace=*}
session=${line##* session=}
[ -n "$session" ] || fail "yaar-script named no session: $line"
echo "step 4: yaar-script made the worktree $worktree with the agent's session $session"

keepers_now=$(processes_of "$KEEPER" | cut -d' ' -f1 | tr '\n' ' ')
if [ "$keepers_now" != "$keeper_pid " ]; then
    fail "the Keeper that made the worktree is not the one the app started: pid $keeper_pid then, $KEEPER runs as ${keepers_now:-none} now"
fi
standin_pids=$(claude_below "$keeper_pid" | tr '\n' ' ')
[ -n "$standin_pids" ] || fail "no stand-in agent runs below the Keeper $keeper_pid"
witness="$only/witness"
[ -f "$witness" ] || fail "the hook of $repository left no witness line: $witness is not there, so post-checkout did not find yaar-hook-witness on the path of the Keeper's git"
lines=$(wc -l < "$witness" | tr -d ' ')
[ "$lines" -eq 1 ] || fail "$witness holds $lines lines and not one: $(cat "$witness")"
witness_line=$(cat "$witness")
witness_folder=${witness_line%% *}
witness_path=${witness_line#* }
[ "$(cd -P "$witness_folder" && pwd)" = "$(cd -P "$worktree" && pwd)" ] || fail "the hook ran in $witness_folder and the worktree is $worktree"
case ":$witness_path:" in
    *":$only/bin:"*) ;;
    *) fail "the hook ran with the path $witness_path, which lacks $only/bin" ;;
esac
listed=$(plain_git -C "$repository" worktree list)
# One line of the listing, so a pattern cannot join the main line's folder to the next line's
# branch.
on_branch=no
while IFS= read -r entry_line; do
    case "$entry_line" in
        "$worktree "*"[$BRANCH]") on_branch=yes ;;
    esac
done <<< "$listed"
[ "$on_branch" = yes ] || fail "git does not list $worktree on branch $BRANCH: $listed"
echo "step 5: the Keeper is still $keeper_pid; the hook ran once in $witness_folder with the login path; git lists the worktree on $BRANCH"

kill -TERM "$window_pid" || fail "the Window $window_pid was gone before the step ended it"
"$programs/yaar-script" stop --deadline 20 > "$scratch/script.log" 2>&1 \
    || fail "yaar-script stop failed: $(cat "$scratch/script.log")"
all_gone() {
    local pid
    seen=""
    for pid in $window_pid $keeper_pid $standin_pids; do
        is_gone "$pid" || seen="$seen $pid"
    done
    [ -z "$seen" ] || seen="still running:$seen"
    [ -z "$seen" ]
}
wait_until 20 "the Window, the Keeper and the stand-in did not all end" all_gone
if [ "$startup_existed" = yes ]; then
    cat "$scratch/startup.original" > "$startup"
    cmp -s "$startup" "$scratch/startup.original" || fail "$startup is not as it was before"
else
    rm -f "$startup"
fi
startup_changed=no
rm -rf "$only" "$repository" "$worktrees"
made_only=no
made_repository=no
made_worktrees=no
echo "step 6: the app, its Keeper and the agent are gone, $startup is as it was, and $only, $repository and $worktrees are removed"
