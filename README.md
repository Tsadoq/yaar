# YAAR

Yet another agent reviewer: a desktop app for reviewing what a coding agent changes, while
it changes it. You run Claude Code or pi in its terminal, read the diff beside it, comment
on lines, and send the comments back to the agent.

**Pre-release.** Today the app opens a git repository, shows what a branch changed, lets
you comment on any line of the diff and keeps those comments, copies the review as text to
paste to an agent, and runs terminals beside the diff. It hears Claude Code, which you run in
one of those terminals as always: the sidebar shows whether it works, waits for you or has
finished a turn, Flow shows what it does as it works, and Changes shows what the turn
changed. It does not deliver comments to the agent by itself yet: you hand it the copied
review by hand. pi runs in a terminal as any program does, and the app does not hear it yet.

## What it does now

- **Opens a folder** that holds a git repository or one of its worktrees. The sidebar lists
  each opened repository with all its worktrees under it, each named by its branch.
- **Shows the changes of the worktree on screen**: what they are compared with, the
  commits, and each changed file as a diff, committed or not. A branch is compared with
  the default branch of its repository, from the point where it left it. On the default
  branch itself the app lists the commits that are not on its upstream and shows the
  uncommitted changes.
- **Lets you set another base**, a branch or a commit, and keeps it.
- **Comments on any line of a diff**, removed lines included, in the unified and in the
  split view. A comment belongs to a path, a side of the diff, its lines and their text.
- **Keeps a draft from the first key.** What you type in a comment box is saved at every
  key, and a draft survives closing the window, a crash of the app or of the machine, and
  a restart: at most the last key is lost. You can edit and delete a draft, and reply in
  the thread of a comment that was sent.
- **Copies the review as text** to the clipboard: every unsent comment on an unresolved
  thread, each with its path, its lines and the code it sits on, under an instruction for
  the agent. The copied comments then count as sent in a new round, cannot be edited, and
  the state of the worktree at that moment is recorded.
- **Marks a file viewed**, which the app forgets when the file changes, and **marks the
  change reviewed**, which records the worktree as it is and starts the next review from
  it. Neither makes a commit.
- **Commits on request.** The commit control asks for a message and makes an ordinary git
  commit of every uncommitted change of the worktree, with your own git identity and
  hooks. It marks nothing reviewed. Nothing commits unless you press it.
- **Keeps a review for each agent session.** Each worktree has one review that belongs to
  no agent. The first Claude Code session started in the worktree takes it over. When the
  session ends its review stays open, and the next Claude Code session in that worktree
  takes it over. Two Claude Code sessions in one worktree have a review each; with several,
  the line above Changes names them and Shift-A, or a click on a name, shows the other's.
  The app lists a worktree that git removed, marked "worktree gone", for as long as its
  review holds a comment.
- **Hears Claude Code and shows what it is doing.** The sidebar marks each worktree whose
  agent waits for your permission, has finished a turn you have not reviewed yet, works, or
  is not running; with several sessions in a worktree it shows the first of these in that
  order. An agent that waits at a permission prompt carries the same mark on its terminal.
  The app never answers a prompt or any other dialog of the agent: you answer in the
  terminal. A program that is not Claude Code, a CLI that was not started with the app's
  pieces, and an agent the app does not support are not heard: their changes show as changes
  no agent made.
- **Tells you when a turn ends elsewhere.** When a turn ends in a worktree that is not on
  screen, its mark changes in the sidebar and a notice offers to switch to it
  (Cmd-Alt-N).
- **Shows the history of the worktree in Changes.** Above the files, a tree lists the
  commits of the branch and the agent turns that ended on them, in the order they
  happened, and last the uncommitted changes. Click an entry, or Tab into the tree and use
  `j`, `k`, Space and Enter as tuicr's commit selector does, to review one commit, one turn
  or a range of them. The uncommitted changes listed are those of the review's own session
  and those no agent made, which are marked.
- **Selects what a turn changed.** When a turn ends, Changes selects what changed since you
  last reviewed. Shift-T sets the app to keep the selection instead: Changes stays as it
  is and offers what changed, which Shift-O, or a click, takes. The setting lasts until the
  window closes.
- **Shows what the agent does, in Flow.** The review pane has two views, Changes and Flow;
  the bar at its top names both, and `f` switches between them. Flow has two forms, which
  Shift-L switches between. The *timeline* lists each read, edit, new file and shell command
  of the turn in order, as the agent does it. The *live diff* shows the files the running
  turn has changed so far, from the state the worktree was in when the turn started up to
  the files on disk (once the turn ended, up to the state recorded at its end), and grows as
  the agent writes; a file a shell command writes shows once the watcher sees it. After the
  files it changed come the files the turn read and left as they were, each marked as read,
  with its path and no lines; a file leaves that group when the turn changes it. Flow opens
  as the timeline the first time, and afterwards in the form you chose last in that
  worktree, which the Keeper keeps.
- **Follows the agent.** Follow is on in every new window and stays on until you turn it off
  with its control in the bar or with Shift-F. While it is on and the agent works, Flow
  moves to each step as it happens, to a file the agent reads as to a file it writes. When
  you send the agent a message while follow is on, the review pane switches to Flow.
  Scrolling, clicking or typing in the pane while a turn runs pauses follow: the line under
  the bar counts the steps you miss, and Shift-G, or its button, jumps to the newest step and
  follows again. The bar and the keys `f`, Shift-L, Shift-F and Shift-G pause nothing, and
  nothing pauses while no turn runs. Flow and the agent's steps never scroll Changes.
- **Offers what a turn changed, in Flow.** When a turn ends with Flow on screen, Flow stays
  and a line offers what changed since you last reviewed; Shift-O, or its button, opens that
  change in Changes.
- **Drafts in Flow.** In the live diff, go to a line with `j`, `k` or a click and press `c`:
  the comment is a draft as in Changes, and it shows on that line in Changes.
- **Records the worktree at the end of every turn** and at each send of a review, as a
  state in the app's own folder, without a commit and without touching your index or
  branch.
- **Quits two ways.** Cmd-Q quits the window and the agents and terminals go on. Cmd-Alt-Q
  quits and stops every agent and program the app holds. Plain quit is the default.
- **Shows it all again when a window opens.** Start the app again, and every terminal the
  Keeper still holds comes back with its screen, each review with its drafts, and the
  turns that ended meanwhile in the tree.
- **Runs terminals** in a pane to the right of the diff. Each terminal starts your login
  shell (`SHELL`, else the shell of your user, else `/bin/sh`) in the folder of the
  worktree on screen, and anything you start in it, an agent CLI included, runs as in any
  terminal: the keys you type reach the program, and so do the mouse and the paste. The
  pane shows the terminals of the worktree on screen, one, or two side by side in the
  split view; each worktree keeps its own terminals, which run on while another worktree
  is on screen, and they come back with it, with the focus and the split they had. The
  pane moves to the left of the diff, or below it. The window's title is the one the
  program of the focused terminal sets.
- **Writes to your repository only when you commit.** The diff, the comments and the
  states the app records are read from git or kept in the app's own folder. What you run
  in a terminal is your own doing.
- **Adds its pieces to Claude Code only through its own extension points.** Every shell of
  the app sets `CLAUDE_CODE_PLUGIN_DIRS`, after any value you have, to a plugin folder in
  the app's own data folder, and sets `YAAR_SOCKET` and `YAAR_TERMINAL`. The plugin, named
  `yaar-review`, runs the hook program `yaar-claude-hook`, which passes what Claude Code
  reports to the Keeper and decides nothing: `/hooks` lists them as the plugin's. Nothing
  is written into your Claude Code settings, your `~/.claude` or your worktree, the app
  reads no credential of any CLI, and it does not use Claude Code's editor connection:
  variables that name one are left out of the shell. A `claude` started outside the app
  loads nothing of it.

### Not there yet

- A settings screen: the form of Flow is the one setting kept, and Shift-L sets it.
- Follow is not kept: every new window starts with it on, however you left the last one.
- Claude Code's own turns: a turn Claude Code starts by itself, as at the end of a
  background task, switches the pane to Flow as a message of yours does, since the app
  cannot tell the two apart yet.
- Older turns in the timeline: it shows the running turns, or the newest turn of the
  worktree. Earlier turns are kept, and cannot be picked.
- A file the agent reads through an `@` mention or a shell command is in neither the
  timeline nor the files read of the live diff: the app hears reads by Claude Code's read
  tool only.
- A file you change by hand between two turns shows in the next turn's live diff, which
  counts from the state the worktree was in when that turn started.
- The live diff finds its files with one `git status` for each question, one question at a
  time, so on a large repository it can lag behind the writes by about a second.
- Delivering a review to an agent: the app cannot send a comment by itself, and no tool of
  the agent hands it one. Copy the review and paste it.
- pi: it runs in a terminal, and the app hears no session, turn or permission prompt of it
  yet, nor does it give pi any piece of the app.
- A control that starts an agent for you: type `claude` in a shell of the app.
- The hook program is not packaged yet: the Mac app and the `.deb` hold the Window and the
  Keeper only. Build the workspace, which puts `yaar-claude-hook` beside `yaar-keeper`.
- The state of a session that waits for permission stays on until the tool ends: after you
  approve a long command at its dialog, the mark stays until the command finishes.
- A workspace with one idle session and one that ended shows "not running" while the
  ended session still holds its review, and no mark once another session has taken that
  review over, since the ended session is then no longer listed. A session that is idle,
  with nothing to review, draws no mark.
- Resolving a thread, and a list of the threads with their states.
- Comments on a range of lines, and on a line reached through a definition.
- Two worktrees that share one id at different paths: the app refuses the second.

| On macOS | On Linux | Does | Works |
| --- | --- | --- | --- |
| Cmd-O | Ctrl-O | Opens a folder | anywhere |
| Cmd-Shift-B | Ctrl-Shift-B | Asks for a branch or a commit and makes it the base | anywhere |
| Cmd-B | Ctrl-B | Pins the sidebar as a rail, or as the full list | anywhere |
| Cmd-Alt-B | Ctrl-Alt-B | Lets the width of the window choose the form of the sidebar again | anywhere |
| Cmd-Alt-Down, Cmd-Alt-Up | Ctrl-Alt-Down, Ctrl-Alt-Up | Next and previous worktree | anywhere, but on macOS not in the field for the base |
| Cmd-T | Ctrl-T | Opens a terminal for the worktree on screen | anywhere |
| Cmd-], Cmd-[ | Ctrl-], Ctrl-[ | Next and previous terminal of the worktree on screen | anywhere |
| Cmd-D | Ctrl-D | Split view of two terminals of the worktree on screen, on and off | anywhere |
| Cmd-Alt-P | Ctrl-Alt-P | Moves the terminal pane to the left of the diff, below it, and back to the right | anywhere |
| Ctrl-Tab, Ctrl-Shift-Tab | the same | Moves the focus to the next and the previous pane | anywhere, a terminal included |
| Cmd-C, Cmd-V | Ctrl-Shift-C, Ctrl-Shift-V | Copies the selected text of a terminal, pastes into it | in a terminal |
| Shift-PageUp, Shift-PageDown | the same | Scrolls the history of a terminal | in a terminal |
| Cmd-Q | Ctrl-Q | Quits; the agents and terminals go on | anywhere |
| Cmd-Alt-Q | Ctrl-Alt-Q | Quits and stops every agent and program the app holds | anywhere |
| Cmd-Alt-N | Ctrl-Alt-N | Shows the worktree where an agent just finished a turn | anywhere |
| `j` or Down, `k` or Up | the same | One line down, one line up | in the diff |
| `s` | the same | Switches between the unified and the split diff, on the same line | in the diff |

### Keys in the review pane

They work in the diff and in Flow, as plain keys. The review keys follow vim and tuicr (0.29.0,
`docs/KEYBINDINGS.md`) wherever either has a key for the action; where neither has, the key
is the app's own choice.

| Key | Does | From |
| --- | --- | --- |
| `j`, `k`, Down, Up | One line down, one line up | vim and tuicr |
| `s` | Switches between the unified and the split diff | the app; tuicr has the command `:diff` |
| `c` | Opens a comment box on the line | tuicr, add a line comment |
| `i` | Edits the draft on the line | tuicr, edit the comment; vim, insert |
| `o` | Writes a reply in the thread of a sent comment | the app; vim's `o` opens a line below |
| `d d` | Deletes the draft on the line | tuicr and vim |
| `m`, Shift-M | Next and previous thread | tuicr, next and previous comment |
| `y` | Copies the review as text, sending its comments | tuicr, copy the review; vim, yank |
| `r` | Marks the file viewed, or takes the mark away | tuicr, toggle a file reviewed |
| Shift-R | Marks the change reviewed | the app; tuicr's `R` marks a hunk |
| Shift-C | Asks for a message and commits every uncommitted change | the app; tig and lazygit commit on `C` |
| Shift-T | Keeps the selection at a turn end, and offers what changed, or goes back to selecting it | the app |
| Shift-O | Takes the offer: shows what changed since the last review | the app |
| Shift-A | Shows the review of the next agent session of the worktree | the app |
| `tab` | Moves the focus from the diff to the tree of Changes, and back | tuicr cycles the focus with Tab |
| `f` | Switches between Changes and Flow, in both views | the app; neither vim nor tuicr has the action |
| Shift-F | Turns follow on and off, in both views | the app; neither has the action |
| Shift-L | Switches Flow between the timeline and the live diff, and keeps the form for the worktree; in Flow only, in Changes it does nothing | the app; neither has the action |
| Shift-G | Resumes follow at the newest step; in Flow only, in Changes it does nothing | vim, last line; tuicr, last file |

In the tree of Changes, as plain keys:

| Key | Does | From |
| --- | --- | --- |
| `j`, `k`, Down, Up | Moves the cursor one entry down, one up | tuicr, commit selector |
| Space | Marks the entry under the cursor | tuicr, commit selector |
| Enter | Shows the change of the marked entries, first to last, or of the one under the cursor | tuicr, commit selector |

In the comment box the keys type text. Enter closes the box, Shift-Enter starts a new line,
and Escape closes it too: the draft stays either way, and `d d` deletes it.

On macOS the field where you type a base takes Cmd-Alt-Down and Cmd-Alt-Up for itself while
it has the focus, so those two keys do not change the worktree there; they do again once
the focus is elsewhere.

On Linux the keys of the app are those of the Mac with Ctrl in place of Cmd. With nothing
pinned, the sidebar is a rail in a window narrower than 1000 px.

### Keys in a terminal

Every key typed in a focused terminal reaches its program, but the kept keys. These never
reach the program, and each runs its action in the app. Their names are those the app binds,
as GPUI writes them.

| On macOS | On Linux |
| --- | --- |
| `ctrl-tab`, `ctrl-shift-tab`: the next and the previous pane | the same |
| `shift-pageup`, `shift-pagedown`: scroll the history | the same |
| `cmd-c`, `cmd-v`: copy and paste | `ctrl-shift-c`, `ctrl-shift-v` |
| every Cmd key: `cmd-o`, `cmd-shift-b`, `cmd-b`, `cmd-alt-b`, `cmd-alt-down`, `cmd-alt-up`, `cmd-alt-p`, `cmd-alt-n`, `cmd-t`, `cmd-]`, `cmd-[`, `cmd-d`, `cmd-q`, `cmd-alt-q` run their action, and a Cmd key that has none sends nothing, but Cmd-Left, Cmd-Right and Cmd-Backspace edit the line as in other Mac terminals and send `^A`, `^E` and `^U` | none: the keys of the app are Ctrl keys |

On Linux the keys of the app that are not kept, New Terminal, Split View, Quit and the rest,
work only outside a terminal: in a terminal `ctrl-t`, `ctrl-d`, `ctrl-q` and the others
belong to the program. Leave the terminal with `ctrl-tab` first, then press them. On macOS
the Option key types characters, as the keyboard layout says; on the U.S. layout it acts as
Meta, so that Option-B is `ESC b`. Text typed through an input method or a dead key reaches
the program once it is composed.

### A Keeper left running

Cmd-Q quits the Window and leaves the Keeper, which owns the terminals and hears the agents.
The Keeper stays for as long as a program runs in one of them, however long that is, and
leaves five seconds after the last window closed when none does. Agents go on working while
the window is closed: a turn that ends meanwhile is recorded, and the next window shows it
with every terminal's screen and every review as it was.

Cmd-Alt-Q is the other quit: it tells the Keeper to stop everything. Each terminal's programs
get the hang-up, and what ignores it is killed within a few seconds, background jobs of a
shell included; a process that left its session by `setsid`, as a daemon does, is not
reached. The Keeper then ends. An older Keeper, left from before an update, offers the same
choice in a note at the top of the window. To end the Keeper by hand:

```sh
pkill -x yaar-keeper
```

The Keeper ends at once. Each program in a terminal then gets the hang-up of its closing
terminal, and a shell passes it on to the jobs it started: on macOS that ends them all. On a
system where `/bin/sh` is dash, as on Debian and Ubuntu, a job started with `&` can
outlive its shell; `pgrep -l` and `pkill` find it by its own name. A Keeper that finds its
socket left behind by one that ended this way starts afresh.

The Keeper watches the files of the worktree on screen, and of a worktree in which an agent
runs, to tell you of a change and to tell a change an agent made from one made by you or by
a program the app does not hear. Who made an uncommitted change is remembered until the
Keeper ends; a change made before it started again shows as one no agent made.

It keeps the opened repositories, the bases you set, and every review with its comments,
drafts and rounds in one database, in `~/Library/Application Support/yaar-review/` on macOS
and in `$XDG_DATA_HOME/yaar-review/` or `~/.local/share/yaar-review/` on Linux, which also
holds the plugin folder for Claude Code. The states of
a worktree that the app records for its reviews are kept beside it, in a git repository of
the app's own under `states/`, never in yours. In the git folder of every worktree of an
opened repository it writes one small file, `ade-id`, by which it knows the worktree again
after its folder moved. It writes nothing else into a repository, but the commit you ask for
with Shift-C.

## Build

You need [rustup](https://rustup.rs) and `git`. The compiler is pinned in
`rust-toolchain.toml` and rustup installs it on first use. A `cargo` that ignores that
file, such as Homebrew's, builds with the wrong compiler: put `~/.cargo/bin` first on your
`PATH`.

On Debian and Ubuntu the build also needs
`gcc libc6-dev pkg-config libfontconfig-dev libxkbcommon-x11-dev`, and on Ubuntu 22.04 `g++`.

```sh
cargo build --locked --workspace
```

## Start

```sh
cargo run --locked -p yaar-window         # opens the window
```

The window starts the Keeper by itself and looks for that program beside its own, and the
Keeper looks for the hook program there too, so build the workspace first: `cargo run` alone
builds only the Window. The Keeper leaves five
seconds after the last window closed, unless a program still runs in one of its terminals
(see "A Keeper left running"). Opened again, the app lists the same repositories.

```sh
target/debug/yaar-review --version
target/debug/yaar-review --keeper-status  # finds or starts the Keeper, says who answered, opens no window
```

## Check

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo nextest run --locked --workspace    # no window opens
cargo xtask layout                        # the rules between the crates
cargo xtask coverage                      # every requirement has a test in the plan
```

The tests run with `cargo nextest`, which gives each test a process of its own: the tests
that drive git set the environment of their process. CI also runs `cargo deny check` and
`cargo machete`; `.github/scripts/install-tools.sh <folder>` fetches these tools at the
pinned versions.

## Layout

Two programs. The **Keeper** (`yaar-keeper`) has no interface and owns what must survive a
closed window. The **Window** (`yaar-review`) draws. They talk over the **Link**.

| Folder | Holds |
| --- | --- |
| `crates/shared` | What both programs use: the Link, the diff, the contract an agent adapter fulfils |
| `crates/keeper` | The Keeper program and its parts. No interface code |
| `crates/window` | The Window program and its parts. No database, no git |
| `companions` | The pieces the agent CLIs load |
| `system-tests` | Tests that start the built programs |
| `test-plan` | The requirements, one sentence each, and the test that proves each one |
| `xtask` | `cargo xtask layout`, which checks the rules between the crates, and `cargo xtask coverage`, which checks that every requirement has a test in the plan |
| `patches`, `vendor` | The one patched file of GPUI and the script that reapplies the patch |
| `packaging` | The Mac app, the `.deb`, the archive, the apt repository, the install script |

Built so far: the Link and the diff; in the Keeper, the reading of git, the recording of
states and the commit, the database, the workspaces with their watcher, the review engine,
the terminals (the Dock), the Switchboard that follows each agent session with the steps it
takes, and the adapter and the companion of Claude Code; in the Window, the frame with its
sidebar and its settings, the review pane with Changes and Flow, and the terminal pane.
Every other crate is still a shell that says in its `lib.rs` which part of the app it will
hold.

Tests come at three levels. Unit tests sit in a crate's source. Component tests sit in its
`tests/` folder and use only what other crates may use; those of the Window run on GPUI's
test platform, which draws nothing. System tests sit in `system-tests/` and start the built
programs.

## Licence

Apache License, Version 2.0; see `LICENSE` and `NOTICE`. Contributions are taken under the
same licence, with no contributor agreement.
