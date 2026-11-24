#!/bin/bash
# Row PKG-12 on Linux: from the installed app started as a desktop's menu starts it, not from
# a shell, a worktree is made with an agent in a repository whose post-checkout hook runs a
# program found only on the user's login path, and the hook runs.
#
#   .github/scripts/linux-login-path.sh <user> <test-programs-dir>
#
# Run as root after linux-launch.sh (which installed xvfb, xauth, x11-utils and
# libglib2.0-bin, and ended the app). <test-programs-dir> holds standin-agent and yaar-script
# (the job package-linux built them on Ubuntu 22.04; an artifact keeps no execute bit, so
# this sets it). Everything done as <user> goes through `sudo -u <user> env -i HOME=<home>
# USER=<user> PATH=/usr/local/bin:/usr/bin:/bin ...` (DISPLAY=:99 too where the app is
# started), so that no variable of the job or of sudo reaches the app.
#   1. A folder of the login path only: <home>/yaar-login-only/bin, owned by <user>, holding
#      - yaar-hook-witness, a script that appends "<PWD> <PATH>" to
#        <home>/yaar-login-only/witness;
#      - claude, a link to <test-programs-dir>/standin-agent (the stand-in of the system
#        tests, speaking as Claude Code; never the real claude).
#      <user>'s login shell is the shell of its entry (`getent passwd <user>`, field 7);
#      useradd gives /bin/sh. Its login start-up file gets the line
#      `export PATH="$HOME/yaar-login-only/bin:$PATH"` appended: ~/.profile for sh and dash;
#      for bash the first that exists of ~/.bash_profile, ~/.bash_login, ~/.profile, else
#      ~/.profile; another shell fails the step with its name. Its bytes (or its absence) are
#      first copied to $RUNNER_TEMP. The folder is on no other path: the step first asserts
#      that the PATH above and `sudo -u <user> env -i HOME=<home> sh -c 'echo $PATH'` do not
#      hold it, and that `sudo -u <user> env -i HOME=<home> <shell> -l -c 'command -v
#      yaar-hook-witness'` finds it (the line works).
#   2. A repository <home>/yaar-login-repo made with git as <user> (GIT_CONFIG_GLOBAL=/dev/null,
#      GIT_CONFIG_NOSYSTEM=1), one commit on main, an identity of its own in .git/config, and
#      .git/hooks/post-checkout: `#!/bin/sh` then `exec yaar-hook-witness`.
#   3. Starts `Xvfb :99` as linux-launch.sh does; as <user>, in the env -i environment with
#      DISPLAY=:99, runs `gio launch /usr/share/applications/io.github.tsadoq.yaar.desktop`
#      (under `dbus-run-session` when linux-launch.sh needed it); waits (30 s) for a process
#      of <user> whose /proc/<pid>/exe is /usr/bin/yaar-review and exactly one whose exe is
#      /usr/libexec/yaar-review/yaar-keeper; records the Keeper's pid.
#   4. As <user> in the env -i environment: `<test-programs-dir>/yaar-script
#      worktree-with-agent --repository <home>/yaar-login-repo --branch from-the-menu --agent
#      claude --deadline 60`; exit 0, and it printed the worktree's folder and a session.
#   5. Asserts: one Keeper of /usr/libexec/yaar-review/yaar-keeper runs as <user> and its pid
#      is the pid recorded in step 3; <home>/yaar-login-only/witness holds exactly one line,
#      whose folder is the worktree's folder of step 4 and whose PATH holds
#      <home>/yaar-login-only/bin; `git -C <home>/yaar-login-repo worktree list` lists the
#      worktree on branch from-the-menu.
#   6. Ends the app: SIGTERM to the Window's pid; as <user>, `yaar-script stop --deadline 20`;
#      waits (20 s) for the Keeper's pid and the stand-in to be gone; stops Xvfb by its pid;
#      puts the start-up file back byte for byte (or removes it if it did not exist), and
#      removes <home>/yaar-login-only, <home>/yaar-login-repo and
#      <home>/yaar-login-repo.worktrees.
# The clause that carries the break is the hook's, as on macOS (macos-login-path.sh).
# Prints what it saw at each step; on failure, the processes of <user>, the Keeper's standard
# error if the app wrote one, and the witness file.
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# What the header leaves to the body:
# - The dbus session. `gio launch` runs under `dbus-run-session` when that program is
#   installed (linux-launch.sh installs the package `dbus` only when GLib refused without a
#   bus) and plainly when it is not.
# - The start-up file, the folders and every process this step made are put back or ended
#   by a trap, so that a step that failed in the middle leaves the user's home as it found
#   it. A folder or a screen a former run left (<home>/yaar-login-only,
#   <home>/yaar-login-repo, the display :99) is not removed: the step fails and says so.
# - What "started by the system and not through a shell" rests on: after step 3, the PATH in
#   /proc/<pid>/environ of the Window and of the Keeper, read as <user>, does not hold
#   <home>/yaar-login-only/bin. That is the environment each began with, which a later
#   setenv does not change; an entry that started the app through the login shell would
#   put the folder there, and the step fails and says so.
# - Before the app is started the step waits (20 s) until no Keeper of <user> runs: one that
#   an earlier step left would answer with the environment it took then.
# - The folders are removed by the trap only when this run made them.
# - Xvfb is started with access control off (-ac) and no TCP socket: the app runs as <user>
#   and the server as root.
# The scripts handed to `sh -c` are single-quoted on purpose: the user's shell expands them.
# shellcheck disable=SC2016
set -euo pipefail
# -E: the ERR trap below also runs inside functions and command substitutions.
set -E

PROGRAM=linux-login-path.sh
SYSTEM_PATH=/usr/local/bin:/usr/bin:/bin
ENTRY=/usr/share/applications/io.github.tsadoq.yaar.desktop
WINDOW=/usr/bin/yaar-review
KEEPER=/usr/libexec/yaar-review/yaar-keeper
DISPLAY_NAME=:99
BRANCH=from-the-menu

# Set to the scratch folder once it is made; fail() leaves a marker file in it.
scratch=""
# What the step was doing when a command that nobody guarded failed: set -e ends the step
# with that command's own status and words, and the trap below puts one sentence of the
# step's own in front of them. Inside a function the line is the one of the outermost call,
# which is a line of the step and not of a helper.
last_command=""
last_line=""
trap 'last_command=$BASH_COMMAND
    if [ "${#BASH_LINENO[@]}" -gt 1 ]; then last_line=${BASH_LINENO[${#BASH_LINENO[@]}-2]}; else last_line=$LINENO; fi' ERR

# One sentence on standard error, and exit 1. The marker file tells cleanup that a sentence
# was said, also when fail ran in a subshell and could set no variable of the step's.
fail() {
    echo "$PROGRAM: $*" >&2
    if [ -n "$scratch" ]; then
        : > "$scratch/said"
    fi
    exit 1
}

usage() {
    echo "$PROGRAM: $*" >&2
    echo "usage: .github/scripts/linux-login-path.sh <user> <test-programs-dir>" >&2
    exit 2
}

[ "$#" -eq 2 ] || usage "two arguments are needed, a user and the folder of the test programs"
user=$1
[ "$(id -u)" -eq 0 ] || usage "it runs as root, and this is user $(id -un)"
[ -n "${RUNNER_TEMP:-}" ] || usage "RUNNER_TEMP is not set; the start-up file is copied there"
[ -d "$RUNNER_TEMP" ] || usage "RUNNER_TEMP is $RUNNER_TEMP, which is not a folder"
[ -d "$2" ] || usage "$2 is not a folder"
programs=$(cd "$2" && pwd -P)
for tool in sudo getent git gio Xvfb ps; do
    command -v "$tool" > /dev/null || fail "$tool is not installed (linux-launch.sh installs xvfb, xauth, x11-utils and libglib2.0-bin before this step)"
done
for program in standin-agent yaar-script; do
    [ -f "$programs/$program" ] || fail "$programs/$program is missing"
    chmod +x "$programs/$program"
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

# The processes the step made, which the trap ends by their pids; the start-up file and
# whether the step changed it.
window_pid=""
keeper_pid=""
xvfb_pid=""
standin_pids=""
startup=""
startup_changed=no
startup_existed=no
# Which of the three folders this run made: a folder a former run left is not this run's to
# remove, so the trap takes out only what is flagged.
made_only=no
made_repository=no
made_worktrees=no

# As <user>, in the environment of a program that no shell started, and in the user's home.
as_user() {
    sudo -u "$user" env -C "$home" -i HOME="$home" USER="$user" PATH="$SYSTEM_PATH" "$@"
}

as_user_on_display() {
    as_user DISPLAY="$DISPLAY_NAME" "$@"
}

as_user_git() {
    as_user GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git "$@"
}

# The pids of the processes of <user> whose program is the file $1. The loop runs as <user>:
# root of a container has no CAP_SYS_PTRACE, and without it the link /proc/<pid>/exe of
# another user's process cannot be read.
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

# Ends a process this step started, if it is still there, and waits for it.
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

# Waits up to $1 seconds until the function $3 succeeds. The function leaves what it saw in
# $seen; without success the step fails with the sentence $2 and that.
seen=""
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

diagnose() {
    {
        echo "---- the processes of $user"
        ps -u "$user" -o pid,ppid,stat,etime,args || true
        echo "---- the app's standard output and error, $scratch/app.log"
        cat "$scratch/app.log" 2> /dev/null || true
        echo "---- the output of yaar-script, $scratch/script.log"
        cat "$scratch/script.log" 2> /dev/null || true
        echo "---- the witness file, $only/witness"
        cat "$only/witness" 2> /dev/null || echo "(there is none)"
    } >&2
}

# Puts every file and process of the step back, on success and on failure alike.
cleanup() {
    local status=$? pid
    trap - EXIT ERR
    set +e
    if [ "$status" -ne 0 ] && [ "$status" -ne 2 ]; then
        # A step that fails without a sentence of its own (a command nobody guarded, a
        # signal) exits with that command's status, which the header does not name: say what
        # failed, and exit 1.
        if [ ! -e "$scratch/said" ]; then
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

# --- 1. The folder of the login path only, and the line that puts it there.
[ ! -e "$only" ] || fail "$only is there already: a former run did not end"
[ ! -e "$repository" ] || fail "$repository is there already: a former run did not end"
[ ! -e "$worktrees" ] || fail "$worktrees is there already: a former run did not end"
[ ! -e "/tmp/.X11-unix/X${DISPLAY_NAME#:}" ] || fail "the display $DISPLAY_NAME is there already"

# Each folder was absent a line ago, so whatever is there from now on is this run's.
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
plain=$(as_user sh -c 'echo "$PATH"')
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
startup_changed=yes
as_user sh -c 'printf "\nexport PATH=\"\$HOME/yaar-login-only/bin:\$PATH\"\n" >> "$1"' sh "$startup"
found=$(as_user "$shell" -l -c 'command -v yaar-hook-witness' 2> /dev/null || true)
[ "$found" = "$only/bin/yaar-hook-witness" ] || fail "the login shell of $user, $shell, finds yaar-hook-witness at '$found' and not at $only/bin/yaar-hook-witness: the line in $startup does not work"
echo "step 1: $only/bin holds yaar-hook-witness and claude; only the login shell $shell of $user finds it, through $startup"

# --- 2. The repository whose hook needs the login path.
as_user_git init --quiet --initial-branch=main "$repository"
as_user_git -C "$repository" config user.name "A Test"
as_user_git -C "$repository" config user.email "test@example.invalid"
as_user sh -c 'echo one > "$1"' sh "$repository/a.txt"
as_user_git -C "$repository" add a.txt
as_user_git -C "$repository" commit --quiet -m "first"
printf '#!/bin/sh\nexec yaar-hook-witness\n' | as_user sh -c 'cat > "$1" && chmod +x "$1"' sh "$repository/.git/hooks/post-checkout"
echo "step 2: $repository on main with a post-checkout hook that runs yaar-hook-witness"

# --- 3. The app, started as a desktop's menu starts it.
# A Keeper of an earlier step that has not left yet would answer with the environment it took
# then, and the Window would not start another: it must be gone first.
no_keeper_runs() {
    # Called from `if`, where set -e is off: a listing that failed must fail the step, and
    # not read as "no Keeper runs".
    local listed
    listed=$(pids_of "$KEEPER") || fail "the processes of $user could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ' ')
    seen="a Keeper runs as $user: ${keepers:-none}"
    [ -z "$keepers" ]
}
keepers=""
wait_until 20 "a Keeper of an earlier step did not leave before the app was started" no_keeper_runs
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
# The app was started by the system and not through a shell if neither program was handed
# the login-only folder: the environment a process starts with is in /proc/<pid>/environ,
# which a later setenv does not change. A start through the user's login shell would put
# the folder there, and the hook would find its program with no take by the Keeper.
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

# --- 4. A worktree with an agent, asked of that Keeper.
if ! as_user "$programs/yaar-script" worktree-with-agent --repository "$repository" \
    --branch "$BRANCH" --agent claude --deadline 60 > "$scratch/script.log" 2>&1; then
    # A hook that cannot start its program makes git refuse the worktree, so the missing
    # login path shows here and not at step 5.
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

# --- 5. The Keeper is the one the app started, and its git ran the hook with the login path.
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
# One line of the listing: the folder, a commit, and the branch in brackets at its end. A
# pattern over the whole text could join the main line's folder to the next line's branch.
on_branch=no
while IFS= read -r entry_line; do
    case "$entry_line" in
        "$worktree "*"[$BRANCH]") on_branch=yes ;;
    esac
done <<< "$listed"
[ "$on_branch" = yes ] || fail "git does not list $worktree on branch $BRANCH: $listed"
echo "step 5: the Keeper is still $keeper_pid; the hook ran once in $witness_folder with the login path; git lists the worktree on $BRANCH"

# --- 6. The end: the Window, then the Keeper and its agent, then the screen.
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
