# Developing YAAR

| Abbreviation | Meaning |
| --- | --- |
| CI | The workflow `.github/workflows/ci.yml`, run on GitHub for every change |
| CLI | A program run from a terminal; here an agent CLI, Claude Code or pi |
| GPUI | The interface toolkit the Window draws with |

How the parts fit together is in [architecture.md](architecture.md).

## What to install

- What [Build from source](../README.md#build-from-source) in the README names: rustup,
  git and, on Debian and Ubuntu, the packages. The pinned toolchain brings `rustfmt` and
  `clippy`. Build and start the app as the README says.
- `cargo-nextest`, `cargo-deny` and `cargo-machete` for the tests and checks, and
  `cargo-about` for [Packages by hand](#packages-by-hand). This script fetches them at the
  versions CI pins, into the folder you name:

  ```sh
  .github/scripts/install-tools.sh ~/.cargo/bin nextest deny machete about
  ```

- Node 22.18 or later, for one test only: the test of pi's extension
  (`crates/keeper/adapter-pi/tests/extension.rs`). Nothing else uses Node.
- The real language servers of the tests, fetched once into a folder of your choice:

  ```sh
  cargo xtask language-servers ~/yaar-language-servers
  export YAAR_LANGUAGE_SERVERS=~/yaar-language-servers
  ```

## Test

```sh
cargo nextest run --locked --workspace
```

No window opens. Run the tests with `cargo nextest`, which gives each test a process of its
own: the tests that drive git set the environment of their process.

- **Unit tests** sit in a crate's source.
- **Component tests** sit in a crate's `tests/` folder and use only what other crates may
  use. Those of the Window run on GPUI's test platform, which draws nothing.
- **System tests** sit in `system-tests/` and start the built programs, with a stand-in
  agent (`standin-agent`) and a scripted client in place of the Window.

What some tests need:

| Need | Tests |
| --- | --- |
| `YAAR_LANGUAGE_SERVERS`, filled by `cargo xtask language-servers` | The tests that run a real tsgo, ty or rust-analyzer. They fail, and never skip, when it is unset or lacks a pinned version |
| Access to the publishers' addresses | `crates/window/navigator/tests/downloaded.rs`, which downloads the three servers into its own scratch folder |
| Node 22.18 or later | `crates/keeper/adapter-pi/tests/extension.rs` |
| `strace`, on Linux | One system test of `system-tests/tests/files.rs` |

Every other test of the downloader is served on the loopback address, and every other test
of the Navigator runs a stand-in language server (`standin-lsp`). `.config/nextest.toml`
runs the system tests that hold the Keeper to a time one at a time, and gives each test
that runs rust-analyzer every test thread.

## The checks

CI runs each of these. Run them locally before a pull request:

| Check | Command |
| --- | --- |
| Format | `cargo fmt --all --check` |
| Lints | `cargo clippy --locked --workspace --all-targets -- -D warnings` |
| Tests | `cargo nextest run --locked --workspace` |
| Licences, advisories, bans, sources | `cargo deny check` |
| Unused dependencies | `cargo machete` |
| The rules between the crates | `cargo xtask layout` |

`deny.toml` holds the licences the dependencies may have and the crates they may not be.
On Linux CI also checks the app's own licence (`.github/scripts/own-licence.sh`) and
plants a refused dependency for each kind of refusal, to see `cargo deny` fail on it
(`.github/scripts/plant.sh`).

## Commit messages

Every commit of a pull request, a merge commit apart, has a subject of the form
`<type>[(<scope>)][!]: <description>`. The type is one of `feat`, `fix`, `perf`, `refactor`,
`docs`, `test`, `build`, `ci`, `chore` and `revert`; the scope is lower-case letters, digits and
hyphens; the description follows a colon and one space. A `!` before the colon, or a footer
`BREAKING CHANGE:` or `BREAKING-CHANGE:`, marks a breaking change.

The subjects decide the next version and make the notes of a release, so each says what the
change does for a reader of the notes. A `feat` raises the minor number, a `fix`, `perf` or
`revert` the patch number, a breaking change the minor number while the major number is 0 and
the major number after; the other types release nothing. [releasing.md](releasing.md) has the
rest.

To try it on your branch:

```sh
.github/scripts/commit-lint.sh origin/main HEAD
```

## What CI does

The job `version` reads the one version number and decides whether the run publishes. On a
pull request, the job `commits` holds the commit messages to the form above. `checks-linux`
runs the checks above on x86-64 and on arm64, then the checks of the scripts that decide and
make a release, each on scratch repositories:

| Script | Shows |
| --- | --- |
| `.github/scripts/commit-lint-check.sh` | The commit check passes the right forms and names each wrong subject |
| `packaging/release/next-version-check.sh` | The next version and the notes of a tag, for every rule |
| `packaging/release/bump-check.sh` | A bump changes `Cargo.toml` and `Cargo.lock` alone and `cargo metadata --locked` still holds; it needs `cargo` and the fetched crates |
| `packaging/release/release-check.sh` | The job `release` pushes once, starts the run, and refuses a moved `main` or an existing tag |
| `.github/scripts/publish-notes-check.sh` | The notes of a release are the changes of its tag, then the fixed text |
| `.github/scripts/publish-latest-check.sh` | A release is the latest, and the site is deployed, only when no published release has a greater version |

`macos` runs the lints, the tests and the layout rules, then builds the release for both Mac
processors and signs the app ad hoc. `package-linux` builds on Ubuntu 22.04 and makes the
`.deb` and the archive. `assemble` makes the release files, the site and an apt repository
signed with a key it makes and throws away. Then the rehearsal: `install-linux` serves that
site on 127.0.0.1 and installs the app with the install line on Ubuntu 22.04, 24.04 and 26.04
and Debian 12 and 13, on both processors, and `install-macos` does the same on macOS; each
starts the app, makes a worktree with an agent, and uninstalls it. A rehearsal publishes
nothing and keeps what it built as the artifact `rehearsal`. Only a run on a version tag with
`publish` on signs with the real keys and publishes. After a push to `main` whose every job
passed, the job `release` starts such a run when the commits since the last release call for
one; see [releasing.md](releasing.md).

## Packages by hand

On a Mac, signed ad hoc and for this Mac only:

```sh
.github/scripts/release-build.sh --target aarch64-apple-darwin
packaging/notices.sh dist/THIRD-PARTY-NOTICES.txt aarch64-apple-darwin
NOTICES=dist/THIRD-PARTY-NOTICES.txt packaging/package-macos.sh "$(packaging/version.sh)" dist target/aarch64-apple-darwin/release
open dist/YAAR.app
```

On Ubuntu 22.04, the system releases are built on, since a program built on a newer
system does not start on an older one. `package-linux.sh` needs `dpkg-dev`.

```sh
.github/scripts/release-build.sh
packaging/notices.sh dist/THIRD-PARTY-NOTICES.txt "$(uname -m)-unknown-linux-gnu"
NOTICES=dist/THIRD-PARTY-NOTICES.txt packaging/package-linux.sh target/release "$(packaging/version.sh)" dist
sudo apt-get install ./dist/yaar-review_*.deb
```

To try the install script on the files of a rehearsal (the artifact `rehearsal` of a run,
unpacked into `rehearsal/`):

```sh
.github/scripts/serve-site.sh start rehearsal/site rehearsal/release 8000
YAAR_BASE_URL=http://127.0.0.1:8000 YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download bash packaging/install.sh
.github/scripts/serve-site.sh stop 8000
```

The script takes `--dry-run`, which changes nothing, and `--uninstall`, with `--purge` and
`--stop-agents`. A rehearsal's Mac app is signed ad hoc: `YAAR_ALLOW_UNNOTARIZED=1` lets
the script install it from a loopback address only.

| Variable of `install.sh` | Default |
| --- | --- |
| `YAAR_BASE_URL` | The line of `packaging/site/base-url`, which the site's copy holds; the copy in the repository needs it set |
| `YAAR_APT_URL` | `$YAAR_BASE_URL/apt` |
| `YAAR_RELEASES_URL` | `https://github.com/Tsadoq/yaar/releases` |
| `YAAR_DOWNLOAD_URL` | `$YAAR_RELEASES_URL/latest/download` |
| `YAAR_METHOD` | `auto`; `apt` or `archive` forces a route on Linux |
| `YAAR_PREFIX` | `~/.local`, the target of the archive route |
| `YAAR_INSTALL_DIR` | The target folder of the Mac app |
| `YAAR_ALLOW_UNNOTARIZED` | `1` for a rehearsal's ad hoc app, from a loopback address only |

## Layout of the repository

| Folder | Holds |
| --- | --- |
| `crates/shared` | What both programs use: the Link, the diff, the contract of an agent CLI |
| `crates/keeper` | The Keeper program and its parts. No interface code |
| `crates/window` | The Window program and its parts. No database, no git |
| `companions` | The pieces the agent CLIs load: Claude Code's programs, pi's extension |
| `system-tests` | Tests that start the built programs |
| `xtask` | `cargo xtask layout` and `cargo xtask language-servers <folder>` |
| `vendor` | GPUI's crate `gpui-pre-apple` with one patch applied |
| `patches` | That patch and the script that re-vendors the crate; see `patches/README.md` |
| `packaging` | The Mac app, the `.deb`, the archive, the apt repository, the site, the install script, the checklists of a release |
| `.github` | The workflow and the scripts of its steps |
| `docs` | These pages |

## Conventions

- Comments are minimal. A comment says only what the code does not say.
- A test is named by what it shows, as a sentence.
- A commit subject has the form of [Commit messages](#commit-messages).
- No test touches the user's files, the real pi or the real claude. Tests run in scratch
  folders of their own, with a stand-in agent.
- Exact versions for the dependencies whose behaviour the app relies on, pinned in the
  root `Cargo.toml`.
