# YAAR

Yet another agent reviewer: a desktop app for reviewing what a coding agent changes, while
it changes it. You run Claude Code or pi in its terminal, read the diff beside it, comment
on lines, and send the comments back to the agent.

**Pre-release.** What exists today is a walking skeleton: an empty window, the background
program it starts, and the link between them. Nothing here reviews anything yet.

## Build

You need [rustup](https://rustup.rs). The compiler is pinned in `rust-toolchain.toml` and
rustup installs it on first use. A `cargo` that ignores that file, such as Homebrew's,
builds with the wrong compiler: put `~/.cargo/bin` first on your `PATH`.

On Debian and Ubuntu the build also needs
`gcc libc6-dev pkg-config libfontconfig-dev libxkbcommon-x11-dev`.

```sh
cargo build --locked --workspace
cargo run --locked -p yaar-window         # opens the window
```

## Check

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace           # no window opens
cargo xtask layout                        # the rules between the crates
```

CI also runs `cargo nextest`, `cargo deny check` and `cargo machete`;
`.github/scripts/install-tools.sh <folder>` fetches those tools at the pinned versions.

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
| `xtask` | `cargo xtask layout`, which checks the rules between the crates |
| `patches`, `vendor` | The one patched file of GPUI and the script that reapplies the patch |
| `packaging` | The Mac app, the `.deb`, the archive, the apt repository, the install script |

Most crates are still shells: each says in its `lib.rs` which part of the app it will hold.

Tests come at three levels. Unit tests sit in a crate's source. Component tests sit in its
`tests/` folder and use only what other crates may use. System tests sit in `system-tests/`.

## Licence

Apache License, Version 2.0; see `LICENSE` and `NOTICE`. Contributions are taken under the
same licence, with no contributor agreement.
