#!/bin/bash
# shellcheck disable=SC2016
# App started as a desktop's menu starts it: a worktree's post-checkout hook runs a program found
# only on the user's login path. .github/scripts/linux-login-path.sh <user> <test-programs-dir>,
# as root after linux-launch.sh. Everything as <user> runs under `env -i`: no job variable leaks.
set -euo pipefail
set -E

HERE=$(cd "$(dirname "$0")" && pwd)
PROGRAM=linux-login-path.sh
SYSTEM_PATH=/usr/local/bin:/usr/bin:/bin
ENTRY=/usr/share/applications/io.github.tsadoq.yaar.desktop
WINDOW=/usr/bin/yaar-review
KEEPER=/usr/libexec/yaar-review/yaar-keeper
DISPLAY_NAME=:99
BRANCH=from-the-menu

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
    echo "usage: .github/scripts/linux-login-path.sh <user> <test-programs-dir>" >&2
    exit 2
}

user=""
home=""
programs=""
only=""
repository=""
worktrees=""
window_pid=""
keeper_pid=""
xvfb_pid=""
standin_pids=""
startup=""
startup_changed=no
startup_existed=no
# Which folders this run made: a folder a former run left is not this run's to remove.
made_only=no
made_repository=no
made_worktrees=no

as_user() {
    sudo -u "$user" env -C "$home" -i HOME="$home" USER="$user" PATH="$SYSTEM_PATH" "$@"
}

as_user_on_display() {
    as_user DISPLAY="$DISPLAY_NAME" "$@"
}

as_user_git() {
    as_user GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git "$@"
}

# The pids of the processes of <user> whose program is $1. Runs as <user>: a container's root has
# no CAP_SYS_PTRACE and cannot read another user's /proc/<pid>/exe.
pids_of() {
    as_user sh -c '
        for process in /proc/[0-9]*; do
            [ "$(readlink "$process/exe" 2> /dev/null || true)" = "$1" ] && echo "${process#/proc/}"
        done
        exit 0' sh "$1"
}

# Whether the process $1 is gone: not there, or ended and waiting to be collected.
is_gone() {
    local stat
    stat=$(cat "/proc/$1/stat" 2> /dev/null) || return 0
    stat=${stat##*) }
    [ "${stat%% *}" = Z ]
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

diagnose() {
    {
        echo "---- the processes of ${user:-nobody yet}"
        if [ -n "$user" ]; then ps -u "$user" -o pid,ppid,stat,etime,args || true; fi
        if [ -n "$scratch" ]; then
            echo "---- the app's standard output and error, $scratch/app.log"
            cat "$scratch/app.log" 2> /dev/null || true
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
    end_process "$xvfb_pid"
    if [ "$startup_changed" = yes ]; then
        if [ "$startup_existed" = yes ]; then
            as_user sh -c 'cat > "$1"' sh "$startup" < "$scratch/startup.original" \
                || echo "$PROGRAM: $startup could not be put back as it was" >&2
        else
            as_user rm -f "$startup" || echo "$PROGRAM: $startup could not be removed" >&2
        fi
    fi
    [ "$made_only" = no ] || rm -rf "$only"
    [ "$made_repository" = no ] || rm -rf "$repository"
    [ "$made_worktrees" = no ] || rm -rf "$worktrees"
    rm -rf "$scratch"
    exit "$status"
}
trap cleanup EXIT

[ "$#" -eq 2 ] || usage "two arguments are needed, a user and the folder of the test programs"
user=$1
[ "$(id -u)" -eq 0 ] || usage "it runs as root, and this is user $(id -un)"
[ -n "${RUNNER_TEMP:-}" ] || usage "RUNNER_TEMP is not set; the start-up file is copied there"
[ -d "$RUNNER_TEMP" ] || usage "RUNNER_TEMP is $RUNNER_TEMP, which is not a folder"
[ -d "$2" ] || usage "$2 is not a folder"
programs=$(cd "$2" && pwd -P) || fail "$2 could not be entered"
for tool in sudo getent git gio Xvfb ps; do
    command -v "$tool" > /dev/null || fail "$tool is not installed (linux-launch.sh installs xvfb, xauth, x11-utils and libglib2.0-bin before this step)"
done
for program in standin-agent yaar-script; do
    [ -f "$programs/$program" ] || fail "$programs/$program is missing"
    chmod +x "$programs/$program" || fail "$programs/$program could not be made executable"
done
entry=$(getent passwd "$user") || fail "there is no user $user"
home=$(printf '%s\n' "$entry" | cut -d: -f6)
shell=$(printf '%s\n' "$entry" | cut -d: -f7)
[ -d "$home" ] || fail "the home of $user, $home, is not a folder"
[ -n "$shell" ] || shell=/bin/sh

only="$home/yaar-login-only"
repository="$home/yaar-login-repo"
worktrees="$home/yaar-login-repo.worktrees"
scratch=$(mktemp -d "$RUNNER_TEMP/login-path.XXXXXX") || fail "no scratch folder could be made under $RUNNER_TEMP"

[ ! -e "$only" ] || fail "$only is there already: a former run did not end"
[ ! -e "$repository" ] || fail "$repository is there already: a former run did not end"
[ ! -e "$worktrees" ] || fail "$worktrees is there already: a former run did not end"
[ ! -e "/tmp/.X11-unix/X${DISPLAY_NAME#:}" ] || fail "the display $DISPLAY_NAME is there already"

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
chown -R "$user": "$only"
as_user test -x "$programs/standin-agent" || fail "$user cannot run $programs/standin-agent: a folder on its way is closed to $user"
as_user test -x "$programs/yaar-script" || fail "$user cannot run $programs/yaar-script: a folder on its way is closed to $user"

case ":$SYSTEM_PATH:" in
    *":$only/bin:"*) fail "the search path of the app, $SYSTEM_PATH, holds $only/bin" ;;
esac
plain=$(sudo -u "$user" env -C "$home" -i HOME="$home" sh -c 'echo "$PATH"') || fail "a plain shell of $user could not be asked for its search path"
case ":$plain:" in
    *":$only/bin:"*) fail "the search path of a plain shell of $user holds $only/bin: $plain" ;;
esac

case "${shell##*/}" in
    sh | dash) startup="$home/.profile" ;;
    bash)
        startup="$home/.profile"
        for candidate in .bash_profile .bash_login .profile; do
            if [ -e "$home/$candidate" ]; then
                startup="$home/$candidate"
                break
            fi
        done
        ;;
    *) fail "the login shell of $user is ${shell##*/}, and only sh, dash and bash are known here" ;;
esac
if [ -e "$startup" ]; then
    startup_existed=yes
    cp "$startup" "$scratch/startup.original"
fi
as_user sh -c 'printf "\nexport PATH=\"\$HOME/yaar-login-only/bin:\$PATH\"\n" >> "$1"' sh "$startup" \
    || fail "the line that puts $only/bin on the login path could not be appended to $startup, which stays as it was"
startup_changed=yes
# Asked as the Keeper asks its login shell (crates/shared/link/src/login.rs): -l -i -c, stdin from
# /dev/null. -i matters to start-up files that act only when interactive (a ~/.profile that sources
# ~/.bashrc); a PATH set there would hide the line.
found=$(as_user "$shell" -l -i -c 'command -v yaar-hook-witness' < /dev/null 2> /dev/null || true)
[ "$found" = "$only/bin/yaar-hook-witness" ] || fail "the login shell of $user, $shell, asked as the Keeper asks it (-l -i -c), finds yaar-hook-witness at '$found' and not at $only/bin/yaar-hook-witness: the line in $startup does not work, or a start-up file the shell reads after it sets PATH again"
echo "step 1: $only/bin holds yaar-hook-witness and claude; only the login shell $shell of $user finds it, through $startup"

as_user_git init --quiet --initial-branch=main "$repository"
as_user_git -C "$repository" config user.name "A Test"
as_user_git -C "$repository" config user.email "test@example.invalid"
as_user sh -c 'echo one > "$1"' sh "$repository/a.txt"
as_user_git -C "$repository" add a.txt
as_user_git -C "$repository" commit --quiet -m "first"
printf '#!/bin/sh\nexec yaar-hook-witness\n' | as_user sh -c 'cat > "$1" && chmod +x "$1"' sh "$repository/.git/hooks/post-checkout"
echo "step 2: $repository on main with a post-checkout hook that runs yaar-hook-witness"

# A Keeper of an earlier step that has not left yet would answer with the environment it took then.
no_keeper_runs() {
    local listed
    listed=$(pids_of "$KEEPER") || fail "the processes of $user could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ' ')
    seen="a Keeper runs as $user: ${keepers:-none}"
    [ -z "$keepers" ]
}
keepers=""
wait_until 20 "a Keeper of an earlier step did not leave before the app was started" no_keeper_runs
# -ac: the app runs as <user> and the server as root.
setsid Xvfb "$DISPLAY_NAME" -screen 0 1280x800x24 -ac -nolisten tcp \
    < /dev/null > "$scratch/xvfb.log" 2>&1 &
xvfb_pid=$!
x_is_up() {
    if [ -S "/tmp/.X11-unix/X${DISPLAY_NAME#:}" ]; then
        return 0
    fi
    seen="no socket yet; Xvfb says: $(cat "$scratch/xvfb.log" 2> /dev/null)"
    if is_gone "$xvfb_pid"; then
        fail "Xvfb ended at once: $(cat "$scratch/xvfb.log")"
    fi
    return 1
}
wait_until 30 "no display came up" x_is_up

launch=(gio launch "$ENTRY")
# linux-launch.sh installed dbus only when GLib refused to launch without a session bus.
if command -v dbus-run-session > /dev/null; then
    launch=(dbus-run-session -- "${launch[@]}")
fi
as_user_on_display "${launch[@]}" < /dev/null > "$scratch/app.log" 2>&1 \
    || fail "gio launch of $ENTRY failed: $(cat "$scratch/app.log")"

windows=""
both_run() {
    local listed
    listed=$(pids_of "$WINDOW") || fail "the processes of $user could not be listed"
    windows=$(printf '%s' "$listed" | tr '\n' ' ')
    listed=$(pids_of "$KEEPER") || fail "the processes of $user could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ' ')
    seen="windows: ${windows:-none}; keepers: ${keepers:-none}"
    [ -n "$windows" ] && [ -n "$keepers" ]
}
wait_until 30 "the Window $WINDOW and its Keeper $KEEPER did not both run" both_run
# shellcheck disable=SC2086 # pids, words by construction
set -- $windows
[ "$#" -eq 1 ] || fail "more than one Window runs as $user: $windows"
window_pid=$1
# shellcheck disable=SC2086
set -- $keepers
[ "$#" -eq 1 ] || fail "$# Keepers run as $user and not one: $keepers"
keeper_pid=$1
# Neither program may have been handed the login-only folder: /proc/<pid>/environ holds the
# environment each started with, which a later setenv does not change.
for started in "$window_pid" "$keeper_pid"; do
    environment=$(as_user sh -c 'tr "\\0" "\\n" < "/proc/$1/environ"' sh "$started") \
        || fail "the environment of the process $started, which the system started, could not be read"
    started_path=$(printf '%s\n' "$environment" | sed -n 's/^PATH=//p')
    [ -n "$started_path" ] || fail "the environment of the process $started holds no PATH: $environment"
    case ":$started_path:" in
        *":$only/bin:"*) fail "the process $started was started with the path $started_path, which holds $only/bin: the app was not started by the system alone but through a login shell" ;;
    esac
done
echo "step 3: the Window $window_pid and the Keeper $keeper_pid run, started by gio launch of $ENTRY, and neither was handed $only/bin"

if ! as_user "$programs/yaar-script" worktree-with-agent --repository "$repository" \
    --branch "$BRANCH" --agent claude --deadline 60 > "$scratch/script.log" 2>&1; then
    case "$(cat "$scratch/script.log")" in
        *"yaar-hook-witness: not found"*)
            fail "the hook of $repository found no yaar-hook-witness on the path of the Keeper's git, so the Keeper did not take the login shell's environment: $(cat "$scratch/script.log")"
            ;;
    esac
    fail "yaar-script worktree-with-agent failed: $(cat "$scratch/script.log")"
fi
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

keepers_now=$(pids_of "$KEEPER" | tr '\n' ' ')
# shellcheck disable=SC2086
set -- $keepers_now
if [ "$#" -ne 1 ] || [ "$1" != "$keeper_pid" ]; then
    fail "the Keeper that made the worktree is not the one the app started: pid $keeper_pid then, $KEEPER runs as ${keepers_now:-none} now"
fi
standin_pids=$(pids_of "$programs/standin-agent" | tr '\n' ' ')
[ -n "$standin_pids" ] || fail "no stand-in agent, $programs/standin-agent, runs as $user"
witness="$only/witness"
[ -f "$witness" ] || fail "the hook of $repository left no witness line: $witness is not there, so post-checkout did not find yaar-hook-witness on the path of the Keeper's git"
lines=$(wc -l < "$witness")
[ "$lines" -eq 1 ] || fail "$witness holds $lines lines and not one: $(cat "$witness")"
witness_line=$(cat "$witness")
witness_folder=${witness_line%% *}
witness_path=${witness_line#* }
[ "$(readlink -f "$witness_folder")" = "$(readlink -f "$worktree")" ] || fail "the hook ran in $witness_folder and the worktree is $worktree"
case ":$witness_path:" in
    *":$only/bin:"*) ;;
    *) fail "the hook ran with the path $witness_path, which lacks $only/bin" ;;
esac
listed=$(as_user_git -C "$repository" worktree list)
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

kill "$window_pid" || fail "the Window $window_pid was gone before the step ended it"
as_user "$programs/yaar-script" stop --deadline 20 > "$scratch/script.log" 2>&1 \
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
end_process "$xvfb_pid"
xvfb_pid=""
if [ "$startup_existed" = yes ]; then
    as_user sh -c 'cat > "$1"' sh "$startup" < "$scratch/startup.original"
    cmp -s "$startup" "$scratch/startup.original" || fail "$startup is not as it was before"
else
    as_user rm -f "$startup"
fi
startup_changed=no
rm -rf "$only" "$repository" "$worktrees"
made_only=no
made_repository=no
made_worktrees=no
echo "step 6: the app, its Keeper and the agent are gone, $startup is as it was, and $only, $repository and $worktrees are removed"
