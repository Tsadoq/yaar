#!/bin/bash
# Row PKG-12 on macOS: from the installed app started by the system, not from a shell, a
# worktree is made with an agent in a repository whose post-checkout hook runs a program found
# only on the user's login path, and the hook runs. Runs on GitHub's Mac only: it opens a
# window and writes the runner user's start-up files.
#
#   .github/scripts/macos-login-path.sh <test-programs-dir>
#
# <test-programs-dir> holds standin-agent and yaar-script (the job macos built them; an
# artifact keeps no execute bit, so this sets it).
#   1. A folder of the login path only: $HOME/yaar-login-only/bin, holding
#      - yaar-hook-witness, a script that appends "<PWD> <PATH>" to
#        $HOME/yaar-login-only/witness;
#      - claude, a link to <test-programs-dir>/standin-agent (the stand-in of the system
#        tests, speaking as Claude Code; never the real claude).
#      The user's login shell (from `dscl . -read /Users/$USER UserShell`) gets the line
#      `export PATH="$HOME/yaar-login-only/bin:$PATH"` appended to its login start-up file
#      (~/.zprofile for zsh, ~/.bash_profile for bash; another shell fails the step with its
#      name), whose bytes (or its absence) are first copied to $RUNNER_TEMP. The folder is on
#      no other path: the step first asserts that `launchctl getenv PATH` and the runner's own
#      PATH do not hold it.
#   2. A repository $HOME/yaar-login-repo made with git, one commit on main, a git identity
#      of its own in .git/config, and .git/hooks/post-checkout: `#!/bin/sh` then
#      `exec yaar-hook-witness`.
#   3. Starts the app with `open -a /Applications/YAAR.app`, waits (30 s) for the Window and
#      its Keeper, by their programs' full paths
#      (/Applications/YAAR.app/Contents/MacOS/yaar-review and
#      /Applications/YAAR.app/Contents/Helpers/yaar-keeper, `ps -axo pid=,ppid=,comm=`);
#      asserts the Window's parent is launchd (1) and that exactly one such Keeper runs;
#      records the Keeper's pid.
#   4. `yaar-script worktree-with-agent --repository $HOME/yaar-login-repo --branch
#      from-the-dock --agent claude --deadline 60`; exit 0, and it printed the worktree's
#      folder and a session.
#   5. Asserts: the Keeper of step 3 is the one that made it: one Keeper of the bundle
#      runs, its pid is the pid recorded in step 3, and its program is
#      /Applications/YAAR.app/Contents/Helpers/yaar-keeper (a Keeper started from a shell
#      would also take the login path, so the clause rests on the Keeper being the one the
#      launched Window started); $HOME/yaar-login-only/witness holds exactly one line, whose
#      folder is the worktree's folder of step 4 and whose PATH holds
#      $HOME/yaar-login-only/bin (the Keeper's git ran the hook with the login path);
#      `git -C $HOME/yaar-login-repo worktree list` lists the worktree on branch
#      from-the-dock.
#   6. Ends the app: SIGTERM to the Window's pid; `yaar-script stop --deadline 20` (the
#      Link's stop, the quit that also stops the agents); waits (20 s) for the Keeper's pid
#      and the stand-in to be gone; puts the start-up file back byte for byte (or removes it
#      if it did not exist), and removes $HOME/yaar-login-only, $HOME/yaar-login-repo and
#      $HOME/yaar-login-repo.worktrees.
# The clause that carries the break is the hook's: a Keeper that does not take the login
# shell's environment runs git with the system's short path, the hook finds no
# yaar-hook-witness, and no line is written. The agent's clause passes either way (the app's
# terminals already run the login shell).
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# What the header leaves to the body:
# - The start-up file, the folders and every process this step made are put back or ended
#   by a trap, so that a step that failed in the middle leaves the runner's home as it found
#   it. A folder a former run left ($HOME/yaar-login-only, $HOME/yaar-login-repo) is not
#   removed: the step fails and says so.
# - Before the app is started the step waits (20 s) until no Keeper of the bundle runs: one
#   that an earlier step left would answer with the environment it took then.
# - The folders are removed by the trap only when this run made them.
# - The environment of the started Keeper is not read: `ps -E` shows none on a recent macOS,
#   so what shows that no shell started the app is the Window's parent, launchd (step 3).
# - The stand-in is found as the process named claude among the descendants of the Keeper
#   of step 3, since the program the shell starts for that name is a link.
# - The login shell of step 1 is also asked, with nothing else in its environment, to find
#   yaar-hook-witness, as the Linux step does: the line in the start-up file works.
set -euo pipefail

PROGRAM=macos-login-path.sh
APP=/Applications/YAAR.app
WINDOW="$APP/Contents/MacOS/yaar-review"
KEEPER="$APP/Contents/Helpers/yaar-keeper"
BRANCH=from-the-dock

fail() {
    echo "$PROGRAM: $*" >&2
    exit 1
}

usage() {
    echo "$PROGRAM: $*" >&2
    echo "usage: .github/scripts/macos-login-path.sh <test-programs-dir>" >&2
    exit 2
}

[ "$#" -eq 1 ] || usage "one argument is needed, the folder of the test programs"
[ "$(uname -s)" = Darwin ] || usage "it runs on macOS, and this is $(uname -s)"
[ -n "${RUNNER_TEMP:-}" ] || usage "RUNNER_TEMP is not set; the start-up file is copied there"
[ -d "$RUNNER_TEMP" ] || usage "RUNNER_TEMP is $RUNNER_TEMP, which is not a folder"
[ -d "$1" ] || usage "$1 is not a folder"
programs=$(cd "$1" && pwd -P)
for tool in dscl launchctl open git ps awk; do
    command -v "$tool" > /dev/null || fail "$tool is not there"
done
for program in standin-agent yaar-script; do
    [ -f "$programs/$program" ] || fail "$programs/$program is missing"
    chmod +x "$programs/$program"
done
[ -x "$WINDOW" ] || fail "$WINDOW is not there: the install step did not put the app in /Applications"
[ -x "$KEEPER" ] || fail "$KEEPER is not there: the app holds no Keeper"
user=$(id -un)
uid=$(id -u)
[ -d "${HOME:-}" ] || fail "HOME is not a folder"
home=$(cd "$HOME" && pwd -P)
shell=$(dscl . -read "/Users/$user" UserShell | sed -n 's/^UserShell: //p')
[ -n "$shell" ] || fail "dscl gave no login shell for $user"

only="$home/yaar-login-only"
repository="$home/yaar-login-repo"
worktrees="$home/yaar-login-repo.worktrees"
scratch=$(mktemp -d "$RUNNER_TEMP/login-path.XXXXXX")

# The processes the step made, which the trap ends by their pids; the start-up file and
# whether the step changed it.
window_pid=""
keeper_pid=""
standin_pids=""
startup=""
startup_changed=no
startup_existed=no
# Which of the three folders this run made: a folder a former run left is not this run's to
# remove, so the trap takes out only what is flagged.
made_only=no
made_repository=no
made_worktrees=no

# git as the runner's own, with none of its configuration.
plain_git() {
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git "$@"
}

# The processes of this user whose program is the file $1, as "<pid> <parent pid>" lines.
processes_of() {
    ps -axo uid=,pid=,ppid=,comm= | awk -v uid="$uid" -v want="$1" '
        $1 == uid {
            program = $4
            for (i = 5; i <= NF; i++) program = program " " $i
            if (program == want) print $2, $3
        }'
}

# The pids of the processes of this user that descend from the process $1 and are named
# claude.
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

# Whether the process $1 is gone.
is_gone() {
    ! kill -0 "$1" 2> /dev/null
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
        ps -U "$uid" -o pid,ppid,stat,etime,command || true
        echo "---- the output of yaar-script, $scratch/script.log"
        cat "$scratch/script.log" 2> /dev/null || true
        echo "---- the witness file, $only/witness"
        cat "$only/witness" 2> /dev/null || echo "(there is none)"
    } >&2
}

# Puts every file and process of the step back, on success and on failure alike.
cleanup() {
    local status=$? pid
    trap - EXIT
    set +e
    if [ "$status" -ne 0 ] && [ "$status" -ne 2 ]; then
        diagnose
    fi
    end_process "$window_pid"
    end_process "$keeper_pid"
    for pid in $standin_pids; do end_process "$pid"; done
    if [ "$startup_changed" = yes ]; then
        if [ "$startup_existed" = yes ]; then
            cat "$scratch/startup.original" > "$startup"
        else
            rm -f "$startup"
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

launchd_path=$(launchctl getenv PATH || true)
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
startup_changed=yes
# shellcheck disable=SC2016 # the line is written as it is: $HOME and $PATH are the login shell's
printf '\nexport PATH="$HOME/yaar-login-only/bin:$PATH"\n' >> "$startup"
found=$(env -i HOME="$home" "$shell" -l -c 'command -v yaar-hook-witness' 2> /dev/null || true)
[ "$found" = "$only/bin/yaar-hook-witness" ] || fail "the login shell of $user, $shell, finds yaar-hook-witness at '$found' and not at $only/bin/yaar-hook-witness: the line in $startup does not work"
echo "step 1: $only/bin holds yaar-hook-witness and claude; neither launchd's path nor the runner's holds it; the login shell $shell finds it through $startup"

# --- 2. The repository whose hook needs the login path.
plain_git init --quiet --initial-branch=main "$repository"
plain_git -C "$repository" config user.name "A Test"
plain_git -C "$repository" config user.email "test@example.invalid"
echo one > "$repository/a.txt"
plain_git -C "$repository" add a.txt
plain_git -C "$repository" commit --quiet -m "first"
printf '#!/bin/sh\nexec yaar-hook-witness\n' > "$repository/.git/hooks/post-checkout"
chmod +x "$repository/.git/hooks/post-checkout"
echo "step 2: $repository on main with a post-checkout hook that runs yaar-hook-witness"

# --- 3. The app, started by the system.
# A Keeper of an earlier step that has not left yet would answer with the environment it took
# then, and the Window would not start another: it must be gone first. The functions below
# run from `if`, where set -e is off, so a listing that failed fails the step by hand and
# does not read as "nothing runs".
no_keeper_runs() {
    local listed
    listed=$(processes_of "$KEEPER") || fail "the processes of $user could not be listed"
    keepers=$(printf '%s' "$listed" | tr '\n' ';')
    seen="a Keeper runs as $user (pid, parent): ${keepers:-none}"
    [ -z "$keepers" ]
}
keepers=""
wait_until 20 "a Keeper of an earlier step did not leave before the app was started" no_keeper_runs
open -a "$APP"
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
[ "$window_parent" = 1 ] || fail "the Window $window_pid was started by process $window_parent and not by launchd (1)"
echo "step 3: the Window $window_pid (parent launchd) and the Keeper $keeper_pid run, started by open -a $APP"

# --- 4. A worktree with an agent, asked of that Keeper.
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

# --- 5. The Keeper is the one the app started, and its git ran the hook with the login path.
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

# --- 6. The end: the Window, then the Keeper and its agent.
kill -TERM "$window_pid"
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
