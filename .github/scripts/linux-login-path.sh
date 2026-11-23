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
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "linux-login-path.sh: not built yet" >&2
exit 99
