# YAAR

Yet another agent reviewer: a desktop app for reviewing what a coding agent changes, while
it changes it. You run Claude Code or pi in its terminal, read the diff beside it, comment
on lines, and send the comments back to the agent.

**Pre-release.** Today the app opens a git repository, shows what a branch changed, and
runs terminals beside the diff. It knows no agent yet: Claude Code, pi or any other
program runs in a terminal as any program does, and the app takes no comments.

## What it does now

- **Opens a folder** that holds a git repository or one of its worktrees. The sidebar lists
  each opened repository with all its worktrees under it, each named by its branch.
- **Shows the changes of the worktree on screen**: what they are compared with, the
  commits, and each changed file as a diff, committed or not. A branch is compared with
  the default branch of its repository, from the point where it left it. On the default
  branch itself the app lists the commits that are not on its upstream and shows the
  uncommitted changes.
- **Lets you set another base**, a branch or a commit, and keeps it.
- **Runs terminals** in a pane to the right of the diff. Each terminal starts your login
  shell (`SHELL`, else the shell of your user, else `/bin/sh`) in the folder of the
  worktree on screen, and anything you start in it, an agent CLI included, runs as in any
  terminal: the keys you type reach the program, and so do the mouse and the paste. The
  pane holds several terminals and shows one, or two side by side in the split view. It
  moves to the left of the diff, or below it. The window's title is the one the program of
  the focused terminal sets.
- **Reads only the repository.** Nothing in the app edits a file, and it runs git only to
  read. What you run in a terminal is your own doing.

| On macOS | On Linux | Does | Works |
| --- | --- | --- | --- |
| Cmd-O | Ctrl-O | Opens a folder | anywhere |
| Cmd-Shift-B | Ctrl-Shift-B | Asks for a branch or a commit and makes it the base | anywhere |
| Cmd-B | Ctrl-B | Pins the sidebar as a rail, or as the full list | anywhere |
| Cmd-Alt-B | Ctrl-Alt-B | Lets the width of the window choose the form of the sidebar again | anywhere |
| Cmd-Alt-Down, Cmd-Alt-Up | Ctrl-Alt-Down, Ctrl-Alt-Up | Next and previous worktree | anywhere, but on macOS not in the field for the base |
| Cmd-T | Ctrl-T | Opens a terminal for the worktree on screen | anywhere |
| Cmd-], Cmd-[ | Ctrl-], Ctrl-[ | Next and previous terminal | anywhere |
| Cmd-D | Ctrl-D | Split view of two terminals, on and off | anywhere |
| Cmd-Alt-P | Ctrl-Alt-P | Moves the terminal pane to the left of the diff, below it, and back to the right | anywhere |
| Ctrl-Tab, Ctrl-Shift-Tab | the same | Moves the focus to the next and the previous pane | anywhere, a terminal included |
| Cmd-C, Cmd-V | Ctrl-Shift-C, Ctrl-Shift-V | Copies the selected text of a terminal, pastes into it | in a terminal |
| Shift-PageUp, Shift-PageDown | the same | Scrolls the history of a terminal | in a terminal |
| Cmd-Q | Ctrl-Q | Quits | anywhere |
| `j` or Down, `k` or Up | the same | One line down, one line up | in the diff |
| `s` | the same | Switches between the unified and the split diff, on the same line | in the diff |

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
| every Cmd key: `cmd-o`, `cmd-shift-b`, `cmd-b`, `cmd-alt-b`, `cmd-alt-down`, `cmd-alt-up`, `cmd-alt-p`, `cmd-t`, `cmd-]`, `cmd-[`, `cmd-d`, `cmd-q` run their action, and a Cmd key that has none sends nothing, but Cmd-Left, Cmd-Right and Cmd-Backspace edit the line as in other Mac terminals and send `^A`, `^E` and `^U` | none: the keys of the app are Ctrl keys |

On Linux the keys of the app that are not kept, New Terminal, Split View, Quit and the rest,
work only outside a terminal: in a terminal `ctrl-t`, `ctrl-d`, `ctrl-q` and the others
belong to the program. Leave the terminal with `ctrl-tab` first, then press them. On macOS
the Option key types characters, as the keyboard layout says; on the U.S. layout it acts as
Meta, so that Option-B is `ESC b`. Text typed through an input method or a dead key reaches
the program once it is composed.

### A Keeper left running

Closing the window quits the Window and leaves the Keeper, which owns the terminals. The
Keeper stays for as long as a program runs in one of them, however long that is. A window
opened again does not show those terminals yet, so a shell left in one cannot be reached
from the app until that comes. To end the Keeper and every program in it, hang up on it by
its process name:

```sh
pkill -x yaar-keeper
```

The Keeper ends at once. Each program in a terminal then gets the hang-up of its closing
terminal, and a shell passes it on to the jobs it started: on macOS that ends them all. On a
system where `/bin/sh` is dash, as on Debian and Ubuntu, a job started with `&` can
outlive its shell; `pgrep -l` and `pkill` find it by its own name. A Keeper that finds its
socket left behind by one that ended this way starts afresh.

The app does not watch the files yet. What changed outside it, a file, a branch or a new
worktree, shows when the window comes to the front again.

It keeps the opened repositories and the bases you set in one database, in
`~/Library/Application Support/yaar-review/` on macOS and in `$XDG_DATA_HOME/yaar-review/`
or `~/.local/share/yaar-review/` on Linux. In the git folder of every worktree of an opened
repository it writes one small file, `ade-id`, by which it knows the worktree again after
its folder moved. It writes nothing else into a repository.

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

The window starts the Keeper by itself and looks for that program beside its own, so build
the workspace first: `cargo run` alone builds only the Window. The Keeper leaves five
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

Built so far: the Link and the diff; in the Keeper, the reading of git, the database, the
workspaces and the terminals (the Dock); in the Window, the frame with its sidebar, the
review pane and the terminal pane. Every other crate is still a shell that says in its
`lib.rs` which part of the app it will hold.

Tests come at three levels. Unit tests sit in a crate's source. Component tests sit in its
`tests/` folder and use only what other crates may use; those of the Window run on GPUI's
test platform, which draws nothing. System tests sit in `system-tests/` and start the built
programs.

## Licence

Apache License, Version 2.0; see `LICENSE` and `NOTICE`. Contributions are taken under the
same licence, with no contributor agreement.
