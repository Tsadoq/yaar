# YAAR

| Abbreviation | Meaning |
| --- | --- |
| CLI | A program run from a terminal; here an agent CLI, Claude Code or pi |

Yet another agent reviewer: a desktop app for reviewing what a coding agent changes, while
it changes it. You run Claude Code or pi in its terminal, read the diff beside it, comment
on lines, and send the comments back to the agent with your next message.

**Pre-release.** No release is published yet. Build it from source as shown below.

YAAR is two programs: the Window (`yaar-review`) draws, and the Keeper (`yaar-keeper`)
runs behind it and holds the terminals, the agents and the reviews; see
[docs/architecture.md](docs/architecture.md).

## What works today

- Opens a git repository or one of its worktrees, and lists every worktree of each
  opened repository in the sidebar.
- Shows what a worktree changed against its base, commit by commit, turn by turn or all
  at once, as a unified or a split diff.
- Runs terminals beside the diff, each in the worktree on screen, with a launcher for
  each agent CLI.
- Hears Claude Code, and pi once you allow its extension: the sidebar shows whether each
  agent works, waits for you or finished a turn, and Flow shows each step it takes.
- Records the worktree at the end of every turn, without a commit and without touching
  your index or branches.
- Keeps comments on any line of a diff, drafts included, through a crash or a restart.
- Sends a review round with your next message to the agent. The agent replies in each
  thread, and the threads follow the code it rewrites. Only you resolve a thread.
- Makes a worktree on a new branch and starts an agent in it in one step.
- Answers hover, go to definition and find references from a language server, for
  TypeScript and JavaScript, Python and Rust, on the new and on the old side of a diff.
- Has a settings screen and a light and a dark theme.
- Keeps the agents and terminals running after you close the window.

YAAR runs on macOS and on Linux. It needs git 2.34 or newer.

## Build from source

You need [rustup](https://rustup.rs) and `git`, and on macOS Xcode with its Metal
toolchain, which compiles the interface toolkit's shaders. The compiler is pinned in
`rust-toolchain.toml`, and rustup installs it on first use. A `cargo` that ignores that
file, such as Homebrew's, builds with the wrong compiler: put `~/.cargo/bin` first on your
`PATH`.

On Debian and Ubuntu the build also needs these packages, and on Ubuntu 22.04 `g++` too;
the second line is what the app needs to run:

```sh
sudo apt-get install gcc libc6-dev pkg-config libfontconfig-dev libxkbcommon-x11-dev
sudo apt-get install libwayland-client0 libvulkan1 mesa-vulkan-drivers fonts-dejavu-core ca-certificates
```

Build the whole workspace:

```sh
cargo build --locked --workspace
```

## Start

```sh
cargo run --locked -p yaar-window
```

The Window starts the Keeper by itself and looks for its program beside its own. The
Keeper looks for the programs the agent CLIs run in the same place. Build the whole
workspace first: `cargo run` alone builds only the Window.

```sh
target/debug/yaar-review --version
target/debug/yaar-review --keeper-status  # finds or starts the Keeper, opens no window
```

## Documentation

| Page | What it holds |
| --- | --- |
| [docs/guide.md](docs/guide.md) | How to use the app, and every key |
| [docs/architecture.md](docs/architecture.md) | The two programs, the crates, the Link, where state lives |
| [docs/development.md](docs/development.md) | How to build, test and check it, and the conventions |
| [docs/releasing.md](docs/releasing.md) | The steps of a release |

## Licence

Apache License, Version 2.0; see `LICENSE` and `NOTICE`. Contributions are taken under the
same licence, with no contributor agreement.
