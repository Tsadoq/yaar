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
#      name). The folder is on no other path: the step first asserts that `launchctl getenv
#      PATH` and the runner's own PATH do not hold it.
#   2. A repository $HOME/yaar-login-repo made with git, one commit on main, a git identity
#      of its own in .git/config, and .git/hooks/post-checkout: `#!/bin/sh` then
#      `exec yaar-hook-witness`.
#   3. Starts the app with `open -a /Applications/YAAR.app`, waits (30 s) for the Window and
#      its Keeper; asserts the Window's parent is launchd.
#   4. `yaar-script worktree-with-agent --repository $HOME/yaar-login-repo --branch
#      from-the-dock --agent claude --deadline 60`; exit 0, and it printed the worktree's
#      folder and a session.
#   5. Asserts: $HOME/yaar-login-only/witness has a line whose folder is that worktree's,
#      with a PATH that holds $HOME/yaar-login-only/bin (the Keeper's git ran the hook with
#      the login path); `git -C $HOME/yaar-login-repo worktree list` lists the worktree on
#      branch from-the-dock.
#   6. Ends the app as macos-launch.sh does, waits for the Keeper and the stand-in to end,
#      takes the line back out of the start-up file, and removes $HOME/yaar-login-only,
#      $HOME/yaar-login-repo and $HOME/yaar-login-repo.worktrees.
# The break that must turn it red: a Keeper that does not take the login shell's environment
# (the hook then finds no yaar-hook-witness, and no line is written).
#
# Exit codes: 0 holds; 1 does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-login-path.sh: not built yet" >&2
exit 99
