# Architecture

| Abbreviation | Meaning |
| --- | --- |
| CLI | A program run from a terminal; here an agent CLI, Claude Code or pi |
| GPUI | The interface toolkit the Window draws with |
| LSP | Language Server Protocol, which the app speaks to language servers |

## Two programs

YAAR is two processes that talk over a Unix socket.

- **The Keeper** (`yaar-keeper`) has no interface. It owns what must survive a closed
  window: the terminals and the programs in them, the agent sessions, the reviews, the
  database and every call of git.
- **The Window** (`yaar-review`) draws. It holds no database and runs no git. It asks the
  Keeper for what it shows, and the Keeper tells it what changed.

The Window finds the Keeper's socket, or starts the Keeper from beside its own program
(`../Helpers/` inside `YAAR.app`, `../libexec/yaar-review/` in the `.deb` and the
archive). The [guide](guide.md#quitting-and-a-keeper-left-running) says when it leaves.

Three more programs run inside the agent CLIs and only report to the Keeper:
`yaar-claude-hook` and `yaar-claude-tools` for Claude Code, and `yaar-pi-report` for pi.

## The crates

One Cargo workspace. The folder of a crate gives its group.

| Group | Crate | Holds |
| --- | --- | --- |
| Shared | `yaar-link` (`crates/shared/link`) | The Link: the messages between the programs, the socket, the app's folders, the login shell |
| Shared | `yaar-diff` (`crates/shared/diff`) | The line diff (histogram, as `git diff --histogram`) and word emphasis |
| Shared | `yaar-agent` (`crates/shared/agent`) | The contract of an agent CLI: the events, and what an Adapter and a way of delivery supply |
| Keeper | `yaar-keeper` (`crates/keeper/keeper`) | The Keeper program: starts the parts below, answers each request, tells each Window what changed |
| Keeper | `yaar-dock` (`crates/keeper/dock`) | Pseudo-terminals, the ring of recent output, each shell's environment, the two quits, the turn marks in terminal bytes |
| Keeper | `yaar-switchboard` (`crates/keeper/switchboard`) | What the agent CLIs report, and each agent session with its CLI, terminal, review and state |
| Keeper | `yaar-adapter-claude` (`crates/keeper/adapter-claude`) | Claude Code: its hook events, delivery, replies, plugin folder, launcher and resume command |
| Keeper | `yaar-adapter-pi` (`crates/keeper/adapter-pi`) | pi: the same, the extension it writes, its one entry in pi's settings, and the program `yaar-pi-report` |
| Keeper | `yaar-review-engine` (`crates/keeper/review-engine`) | Reviews, threads, comments, rounds, turns, attachment, thread following, the review as text, the clean-up of states |
| Keeper | `yaar-store` (`crates/keeper/store`) | The SQLite database and its one writer |
| Keeper | `yaar-recorder` (`crates/keeper/recorder`) | Every call of git: reading the repository, recording states, the commit, adding a worktree |
| Keeper | `yaar-workspaces` (`crates/keeper/workspaces`) | Workspaces, worktree identity, bases, new worktrees, the file watcher, who changed a path |
| Window | `yaar-window` (`crates/window/window`) | The Window program: registers the panes, binds the keys and the menus |
| Window | `yaar-frame` (`crates/window/frame`) | The window, sidebar, layout, settings screen, key routing, the connection to the Keeper, the shared models |
| Window | `yaar-review-pane` (`crates/window/review-pane`) | Changes and Flow, the diff views, threads and comment boxes, file views, follow |
| Window | `yaar-terminal-pane` (`crates/window/terminal-pane`) | The terminal emulator and its drawing, launchers, the attachment at the agent's terminal |
| Window | `yaar-navigator` (`crates/window/navigator`) | Language servers: registry, search, downloader, driver, the old side |
| Window | `yaar-ui` (`crates/window/ui`) | The colours, fonts, themes and shared visual pieces of the Window, and what maps them onto the toolkit |
| Window | `yaar-text-search` (`crates/window/text-search`) | Whole-word search of a worktree, while no language server is ready |
| Companion | `yaar-companion-claude` (`companions/claude`) | Claude Code's hook program `yaar-claude-hook` and tool server `yaar-claude-tools` |
| Companion | none (`companions/pi`) | pi's extension, `yaar.ts`, in TypeScript |
| System tests | `yaar-system-tests` (`system-tests`) | Tests that start the built programs, a stand-in agent and a scripted Window |
| xtask | `xtask` (`xtask`) | `cargo xtask layout` and `cargo xtask language-servers <folder>` |

## The rules between the crates

`cargo xtask layout` reads `cargo metadata` and fails on any breach of six rules. A
dependency counts when it is a normal or a build dependency, direct or not; what only
tests use does not count.

1. No `*-pane` crate depends on another `*-pane` crate.
2. No crate of the Keeper, no shared crate and no companion depends on GPUI.
3. No crate of the Window depends on a crate of the Keeper, nor the reverse.
4. Only `yaar-keeper` depends on an Adapter, and an Adapter depends on shared crates only.
5. The programs the packages ship hold no GPUI test support and no `harness` feature.
6. `yaar-ui` depends on no crate of the workspace.

`clippy.toml` adds one more: no crate opens a pseudo-terminal through the terminal
emulator's crate, because terminals belong to the Keeper's Dock.

Three places take a new thing of a kind without changes elsewhere: an agent CLI is one
Adapter crate, listed in `yaar_keeper::adapters`, and its companion; a pane is one entry
of `yaar_window::panes()`; a language is one row of
`crates/window/navigator/registry.toml`.

## The Link

The Link (`yaar-link`) is the only way between the programs.

- **Frames.** A message travels as one frame: a length of four bytes, little endian; one
  byte that names the kind; the payload. The fields are laid out in `wire.rs`. A frame is
  at most 16 MiB.
- **Versions.** Each side opens with a hello that carries its protocol version. The hello
  and the shutdown never change shape, so a Window of one version can always tell a Keeper
  of another who it is and ask it to stop. Every other frame is spoken only between sides
  of the same version, and a frame with fields left over is refused.
- **Kinds.** A Window sends requests and gets replies or refusals. The Keeper tells every
  attached Window facts unasked, such as a changed review or a turn that ended. Terminal
  bytes travel as input and output frames, a stream with no reply.
- **Agents.** A program inside an agent CLI connects to the same socket, says hello, sends
  one request whose origin is an agent, takes the reply and leaves.
- **The socket.** `YAAR_SOCKET` names it outright. Otherwise it is `keeper.sock` under
  `$XDG_RUNTIME_DIR/yaar-review/` on Linux when that is set, else in the data folder. A
  path longer than the system allows is replaced by `/tmp/yaar-review-<hash>/keeper.sock`,
  used only while that folder is private to the user. Whoever connects refuses a socket
  where a process of another user listens.

In the Window, the Frame holds the one connection. Each fact updates a shared model
(workspaces, changes, agents, terminals, settings and others), and a pane observes the
models it shows. A pane never holds another pane: what the user wants travels as an
action that the Frame routes.

## Where state lives

| Place | macOS | Linux | Holds |
| --- | --- | --- | --- |
| Data folder | `~/Library/Application Support/yaar-review/` | `$XDG_DATA_HOME/yaar-review/` or `~/.local/share/yaar-review/` | `store.db`; `states/`; `agents/claude/` and `agents/pi/`; `language-servers/` and `language-servers.toml` |
| Cache folder | `~/Library/Caches/yaar-review/` | `$XDG_CACHE_HOME/yaar-review/` or `~/.cache/yaar-review/` | `navigator/`: exports of old sides, server logs, rust-analyzer's build folders |
| Each worktree's git folder | | | `ade-id`, by which a moved worktree is known again |

- `store.db` is the SQLite database: the opened repositories and their worktrees, bases,
  reviews, comments, drafts, rounds, sessions, the steps of turns, marks and settings. The
  Keeper is its one writer.
- `states/` holds one bare git repository of the app's own for each worktree, holding its
  recorded states.
- `agents/<name>/` is each Adapter's folder: Claude Code's plugin and the files of its long
  reviews, pi's extension and the user's answer about it.

## How an agent is heard

The contract is `yaar-agent`. An agent CLI is turned into seven events: a session started
or ended, a turn started or ended, a file read, a file written, and waiting for permission
on or off. A shell command is a step of its own, so that a write the watcher sees can be
credited to the session whose command ran. An Adapter supplies its CLI's names, the files
it writes into the app's folder, the variables every shell gets, the marks to watch for in
the terminal bytes, the reading of reports, the way it delivers a review, and the command
that resumes a session.

- **Claude Code.** The Adapter writes a plugin folder, and every shell of the app adds it
  to `CLAUDE_CODE_PLUGIN_DIRS`. The plugin's hooks run `yaar-claude-hook`, which passes
  the hook's input unread to the Keeper and prints the Keeper's answer. A review goes back
  in that answer, or as a file under `agents/claude/reviews/` when it is long. The
  plugin's tool server, `yaar-claude-tools`, gives the agent `reply_to_thread`.
- **pi.** The Adapter writes `agents/pi/extension/yaar.ts`. After the user's yes it adds
  that path to `extensions` in `~/.pi/agent/settings.json`. The extension runs the program
  that `YAAR_PI_REPORT` names, `yaar-pi-report`, for each event, adds an attached review
  to the user's message, and registers `reply_to_thread`.

The Switchboard keeps what arrives while no Window is attached and tracks each session's
state. Workspaces watch the worktree on screen and each worktree where an agent runs, with
FSEvents on macOS and inotify on Linux, and credit each changed path to the session that
wrote it, or to no agent.

## How git is read

The Recorder is the one place that starts git. Every run uses the user's own git and
configuration, so it sees what the user's git sees, and:

- turns off every hook and the file-system monitor;
- never writes or locks the user's index: where git would refresh it, it works on a copy;
- names its repository by folder, never by an inherited `GIT_DIR` or similar variable;
- never prompts, and reads git's answers with `LC_ALL=C`.

The contents of files come from one `git cat-file --batch` process kept for each worktree
and each state store. The diff itself is computed in the app, by `yaar-diff`.

From git 2.43 on, three reads use newer forms: `-z` after `worktree list`, the atom
`ahead-behind` of `for-each-ref`, and `-Z` after `cat-file --batch`. An older git, down to
2.34, is asked in the forms git 2.34 has.

**Recorded states.** A state holds what `git add -A` and a commit would hold, without
touching the user's repository. The user's git runs on a copy of the worktree's index,
with the real repository's objects readable and every new object written to the state
store. The changed paths are staged into the copy, the tree is written, and a commit of it
is named by a ref of the store. The store then copies whatever objects it lacks, so a
state still reads after the real repository is gone.

**The old side of a diff.** When the user asks about a removed line, the Navigator asks,
through the Frame, for the version the change is counted from. The Keeper writes that
version's tree as plain files into a folder of the Navigator's cache. The Navigator links
the dependency folders its registry row names, such as `node_modules`, from the worktree
into it, and starts a second server there. The folder goes when another change is shown
or the window closes.

## Language servers

The Navigator runs in the Window, on a thread of its own, and its servers are the Window's
children: a Window that quits stops them, and on Linux the kernel kills them when the
Navigator's thread ends. It speaks LSP with no code for a particular language: every
difference between servers is a field of a row of the registry. The built-in table is
`crates/window/navigator/registry.toml`, compiled in; the user's `language-servers.toml`
uses the same format, except that each of its servers names a `command` and none is
downloaded. A downloaded server is checked against the SHA-256 digest its row
pins. The same downloader fills the folder of real servers the tests use
(`cargo xtask language-servers <folder>`). While no server is ready, `yaar-text-search`
answers by whole-word search, and the answer is marked as possibly wrong.
