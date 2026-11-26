# Using YAAR

| Abbreviation | Meaning |
| --- | --- |
| CLI | A program run from a terminal; here an agent CLI, Claude Code or pi |
| GPUI | The interface toolkit the Window draws with |

The keys below are the Mac's; on Linux, Ctrl takes the place of Cmd. Every key is listed
under [Keys](#keys).

## Open a repository

Cmd-O opens a folder that holds a git repository or one of its worktrees. The sidebar lists
each opened repository with all its worktrees under it, each named by its branch.
Cmd-Alt-Down and Cmd-Alt-Up move to the next and the previous worktree. Cmd-B pins the
sidebar as a rail or as the full list; with nothing pinned, it is a rail in a window
narrower than 1000 px. Opened again, the app lists the same repositories.

The app writes to your repository only when you commit or make a worktree, and both run
your own git. It also writes one small file, `ade-id`, into the git folder of each
worktree, by which it knows the worktree again after its folder moved.

## The panes

The window holds the sidebar, the review pane and the terminal pane.

### The review pane

The review pane has two views, Changes and Flow. `f` switches between them.

- **Changes** shows what the worktree changed: the commits and each changed file as a
  diff, committed or not. A branch is compared with the default branch of its repository,
  from the point where it left it. On the default branch the app lists the commits that
  are not on its upstream, and the uncommitted changes. Cmd-Shift-B sets another base, a
  branch or a commit, and the app keeps it. `s` switches between the unified and the split
  diff.
- **The tree** above the files lists the commits of the branch and the agent turns that
  ended on them, in the order they happened, and last the uncommitted changes. Click an
  entry, or press `tab` and use `j`, `k`, Space and Enter, to review one commit, one turn
  or a range of them.
- **Flow** shows what the agent does. It opens in **Live diff**, following each file the
  agent reads into a syntax-highlighted view centered on its read line range and switching
  smoothly to the turn's live diff on each edit or write. Shift-L switches to **Timeline**,
  which lists every read, edit, new file and shell command of the turn in order.
- **Follow** is on in every new window. While it is on, Flow moves to each step as the
  agent takes it, and a message you send switches the pane to Flow. Scrolling, clicking
  or typing in the pane pauses it, and Shift-G resumes it. Shift-F turns it off and on.
- **At a turn's end** Changes selects what changed since you last reviewed. Shift-T makes
  it keep the selection and offer the change instead; Shift-O takes the offer.
- **Marks**: `r` marks a file viewed, until the file changes. Shift-R marks the change
  reviewed: the app records the worktree and starts the next review from it. Neither makes
  a commit.
- **Commit**: Shift-C asks for a message and commits every uncommitted change, with your
  git identity and hooks. Nothing commits unless you ask.

### The terminal pane

Each terminal starts your login shell (`SHELL`, else the shell of your user, else
`/bin/sh`) in the worktree on screen. Keys, mouse and paste reach the program as in any
terminal. Each worktree keeps its own terminals, which run on while another worktree is on
screen.

- Cmd-T opens a shell. A single header bar at the top of the pane holds open terminal tabs
  and a `+ ▾` launcher: `+` opens the chosen agent CLI (Cmd-Shift-T; Cmd-Alt-C chooses the
  next), and `▾` lists each agent CLI alongside Shell and lets you switch the default.
  The launcher starts the CLI through your login shell, so it is the same program you type
  in a shell.
- Cmd-] and Cmd-[ move between terminals, Cmd-Shift-W (or a tab's close button) closes a
  terminal, and Cmd-D (or `Split`) shows two terminals at once.
- Cmd-Alt-P (or `Dock ▾`) moves the pane to the left of the diff, below it, and back to
  the right.
- The window's title is the one the program of the focused terminal sets.

## Agents

The app hears Claude Code, and pi once you allow its extension. The sidebar marks each
worktree whose agent waits for your permission, has finished a turn you have not reviewed,
works, or is not running. An agent that waits at a permission prompt carries the same mark
on its terminal. The app never answers a prompt or a dialog of the agent: you answer in
the terminal. An agent the app does not hear runs as any program, and its changes show as
changes no agent made.

**Claude Code.** Every shell of the app sets `CLAUDE_CODE_PLUGIN_DIRS`, after any value you
have, to a plugin folder in the app's data folder. The plugin, `yaar-review`, runs the hook
program `yaar-claude-hook` and the tool server `yaar-claude-tools`, which `/mcp` lists as
`plugin:yaar-review:yaar` with one tool, `reply_to_thread`. Nothing is written into your
Claude Code settings, your `~/.claude` or your worktree. A `claude` started outside the app
loads nothing of it.

**pi.** When `~/.pi/agent` exists, a line in the terminal pane asks whether the app may add
its extension to pi's settings. `Install` or Cmd-Alt-E says yes; `No` or Cmd-Alt-Shift-E
says no. A yes adds one string, the absolute path of `agents/pi/extension/yaar.ts` in the
app's data folder, to the list `extensions` of `~/.pi/agent/settings.json`, and changes no
other byte. Cmd-Alt-Shift-E later takes that string out again. Outside the app the
extension does nothing: it acts only where a shell of the app sets `YAAR_PI_REPORT`. Until
you say yes, the app writes nothing of pi's and does not hear it.

**Resume.** After a restart of the machine or a Cmd-Alt-Q, a `↺ N ▾` control in the
terminal header lists each session of the worktree that was running. Clicking a session,
or pressing Cmd-Shift-R, runs `claude --resume '<id>'` or
`pi --session-dir '<folder>' --session '<id>'`, and the session comes back with its
review. You can also dismiss individual offers (`×`) or clear them all (`Dismiss all`)
without deleting any saved reviews.

## Comments and review rounds

- **Comment.** `c` opens a comment box on any line of a diff, removed lines included, in
  both views. A draft is saved at every key, and survives closing the window, a crash and
  a restart. `i` edits a draft, `d d` deletes it, and `o` replies in the thread of a sent
  comment.
- **Attach.** `a` attaches every unsent comment on an unresolved thread to your next
  message to the agent; `t` attaches the thread on your line alone. Shift-D detaches, and
  every comment stays a draft. Attaching works while the agent works and while it waits at
  a permission prompt.
- **At the agent's terminal** a line says a review is attached, with three controls: View
  (Cmd-Alt-V) shows the exact text the agent will receive; Detach (Cmd-Alt-X); and
  "Address each comment." (Cmd-Alt-A) types that sentence at the prompt. The app never
  presses Enter.
- **Send.** The review goes with the message you submit, instruction first, and only to
  the session it belongs to. Claude Code shows `yaar: review round 1 attached (2 threads)`
  under the prompt. A review of up to 10,000 UTF-16 units goes in the hook's answer; a
  longer one is a file that Claude Code reads with its read tool. pi takes the review as
  part of the message, whole at any length. The comments then count as sent, in a new
  round.
- **Replies.** The agent answers each thread with `reply_to_thread`, and may mark it
  addressed. The tool cannot resolve. A thread is draft, sent, addressed, waiting for you
  (the agent replied and did not mark it addressed) or resolved. `x` resolves the thread
  on your line, `u` reopens it, and Shift-X resolves every addressed thread.
- **Rounds.** A line above the tree names each round. Shift-S shows in Changes what changed
  since a round was sent.
- **Threads follow the code.** A thread whose line moved follows it. When the agent
  rewrote the line, the thread sits on the new code and says so; when it deleted the line,
  the thread sits where the line was. `w` shows the line you commented on.
- **Copy.** `y` copies the review as text to the clipboard, under an instruction for the
  agent. The copied comments count as sent, in a new round.
- **Start an agent for a review.** Attaching a review whose agent is not running offers to
  start that CLI with the review (Shift-N; `ctrl-n` chooses another CLI).
- **One review for each session.** Each worktree has one review that belongs to no agent;
  the first session started in it takes it over. Two sessions in one worktree have a
  review each; switch between them from the session picker dropdown in the review header
  (grouped into active sessions and past reviews) or with Shift-A. A file both changed is
  marked "also changed by another agent".

## Worktrees with an agent

Cmd-Shift-N, or File > New Worktree with Agent…, asks for the name of a new branch. Enter
makes a worktree of the repository on screen on that branch, from the branch checked out
in the worktree on screen, and opens a terminal in it that runs the chosen agent CLI. Your
own git makes it, as `git worktree add -b`, with your hooks.

The worktree lies in the folder where the repository's linked worktrees already lie, else
in `<repository>.worktrees/`, else in the folder you set for that repository in the
settings. A `/` in the branch's name is a `-` in the folder's (`feature/login` lies in
`feature-login`). A branch that exists, a name git refuses and a folder that is already
there are refused with the reason, and nothing is made. When the agent's terminal cannot
be opened, the worktree and its branch are taken away again.

A worktree is removed with git. The sidebar lists a worktree git removed as "worktree
gone" for as long as its review holds a comment.

## Settings

Cmd-comma (Ctrl-comma on Linux), or YAAR > Settings… in the menu bar on macOS, shows the
settings screen in the place of the panes; the same key or Escape closes it. A choice is
in force at once and is kept.

| Row | Values | In force |
| --- | --- | --- |
| When a turn ends | Select what changed (the default), Keep the selection | at the next turn's end |
| Language servers off screen stop after | 1, 2, 5, 10 (the default), 15, 30 or 60 minutes | at once |
| Theme | As the system (the default), Light, Dark | at once, the whole window |
| Base of this worktree | Enter opens the field for the base, as Cmd-Shift-B does | as Cmd-Shift-B sets it |
| Keep recorded states nothing refers to for | 1, 7, 14, 30 (the default), 60, 90, 180 or 365 days | a sweep runs at once |
| Terminal pane | Right (the default), Below, Left | at once |
| Option key in a terminal (macOS) | By the keyboard layout (the default), Meta, Characters | at the next key |
| Extension of the agent | one row for each agent CLI that asked leave to write into its settings: Install, Don't install | as the two keys answer |
| Language servers inside the repository | one row for each repository you answered: Run them, Keep them out | a yes searches again |
| Recorded states | the size of each worktree's states and the total, and Clean up now | at once |
| New worktrees go in | one row for each repository: Enter opens a field for an absolute folder; an empty field goes back to beside the repository | at the next worktree of that repository |

A folder for new worktrees must be absolute and must not lie inside a worktree or the git
folder of the repository.

**Recorded states.** At the end of every turn and at each send of a review, the app
records the worktree as a state, in a git repository of its own under `states/` in its
data folder, never in yours. Clean up now, or Shift-C on the settings screen, deletes at
once every state nothing refers to. A state is referred to while a thread sits on it, a
round names it or a reviewed mark names it; the newest state of each worktree, the start
of a running turn and the states of the last hour are kept too. Otherwise a state nothing
refers to goes once the time set (30 days by default) and one hour have passed. The Keeper
checks once it has started, then every hour.

## Themes

Until you choose one, the window follows the system's light or dark setting, also when it
changes while the app runs. Light or Dark in the settings fixes the theme; "As the system"
follows it again. On Linux the setting comes from the desktop's portal, and where there is
none the app is light. The terminal's colours are the theme's, and a program that asks for
its colours is answered from the theme.

## Language servers

The app answers what a name in the diff is, where it is defined and who uses it, for
TypeScript and JavaScript, Python and Rust.

- Click a name, or move to it with `l` and `h`, and press Shift-K for the hover; resting
  the mouse on a name for 400 ms asks the same. `g d` opens the definition in a read-only
  file view; Escape returns to the diff. `g r` lists the references. A click with Cmd held
  goes to the definition. `c` comments on a line of the file view.
- On a removed line the questions are about the old version. The Keeper writes that
  version, as plain files, into the app's cache folder, and the app starts a second server
  there. The worktree is not touched.
- While no server is ready, and for other languages, the answer is found by searching the
  worktree for the name, and is marked as possibly wrong.
- A line under the bar of the review pane says "ready", "starting; names are found by text
  match meanwhile" or "no language server found".

The app tries, in order: the server you registered, the project's own (`node_modules` for
TypeScript, `.venv` for Python, rustup's rust-analyzer for Rust), one installed on the
machine, one the app downloaded. A server that fails its trial run is passed over.

| Language | Servers, in order | The app downloads |
| --- | --- | --- |
| TypeScript and JavaScript | tsgo, typescript-language-server, vtsls | tsgo |
| Python | basedpyright, pyright, ty, pyrefly | ty |
| Rust | rust-analyzer: rustup's, in `$CARGO_HOME/bin` or `~/.cargo/bin`, as the project's own; then the first program of that name on the search path of the Window | rust-analyzer |

- **Environment.** The first time a server is needed, the Window runs your login shell
  once (`-l -i -c`, for at most 10 seconds) and adds what it prints to the servers'
  environment, so a Window started from the Finder or the Dock finds rustup's, Homebrew's
  and nvm's programs. Rust's server runs with `RUSTUP_AUTO_INSTALL=0`, so it never installs
  a toolchain.
- **Downloads.** Nothing is fetched until you say yes. When no server is found, the line
  offers "Download" (Shift-I). The download is one plain GET of the publisher's address,
  checked against the SHA-256 digest pinned in the app. When the app pins a newer version,
  the line offers "Update to" (Shift-U).
- **A server inside the repository** runs code from the repository, so the line asks once
  for each repository: "Run them" (Shift-Y) or "Not from this repository" (Shift-Q).
- **Your own server.** The file `language-servers.toml` in the app's data folder takes a
  row for a language, which replaces the built-in row:

  ```toml
  [[rust.server]]
  command = ["/path/to/rust-analyzer"]
  ```

- **Start and stop.** A server starts when its workspace comes on screen, if the worktree
  holds a file its row names, such as `Cargo.toml`, `package.json` or `pyproject.toml`,
  else at the first question about one of its files. A workspace off screen for the stop
  time has its servers stopped. Quitting stops
  every server. Nothing a server writes goes into a worktree.

## Quitting, and a Keeper left running

Cmd-Q quits the Window, and the agents and terminals go on. Cmd-Alt-Q quits and stops every
agent and program the app holds. Cmd-W closes the window; the app quits the plain way when
its last window closes. The menu bar on macOS holds YAAR, File and Window; Linux has no
menu bar.

The Keeper stays for as long as a program runs in one of its terminals, and leaves five
seconds after the last window closed when none does. A turn that ends while no window is
open is recorded, and the next window shows every terminal's screen and every review as it
was. An older Keeper, left from before an update, is named in a note at the top of the
window, which offers the same two quits. To end the Keeper by hand:

```sh
pkill -x yaar-keeper
```

Each program in a terminal then gets the hang-up of its closing terminal. On a system where
`/bin/sh` is dash, as on Debian and Ubuntu, a job started with `&` can outlive its shell.

The Keeper watches the files of the worktree on screen, and of each worktree where an agent
runs, to tell a change an agent made from one made by you. A change made before the Keeper
last started shows as one no agent made. The app's data and cache folders are listed under
[Where state lives](architecture.md#where-state-lives).

## Limits

- Comments sit on one line; a range of lines is not there.
- The app does not end a hook. A hook that never ends holds every later creation of a
  worktree until the Keeper ends.
- The Keeper and the Window take your login shell's environment by running your start-up
  files. One that prompts or hangs costs 10 seconds and leaves the program's own
  environment.
- A file the agent reads through an `@` mention or a shell command is not in the timeline.
- A long review is a file. If the model reads it with a shell command, a search or a glob,
  Claude Code asks you as for any such call.
- A message whose report reaches the app more than 1.5 seconds late is sent no review; the
  review stays attached for the next message.
- A message you queue while Claude Code works counts as the start of a second turn.
- After a yes, pi's subagents load the extension too, and each reports a session of its
  own.
- pi pointed at another settings folder with `PI_CODING_AGENT_DIR` never reads the entry
  the app wrote under `~/.pi/agent`.
- The download of a language server takes no proxy.
- A language server that was running when the Window crashed may go on running on macOS.
- `RUSTUP_AUTO_INSTALL=0` is honoured from rustup 1.28 on.

## Keys

### Keys of the app

| On macOS | On Linux | Does | Works |
| --- | --- | --- | --- |
| Cmd-O | Ctrl-O | Opens a folder | anywhere |
| Cmd-Shift-B | Ctrl-Shift-B | Asks for a branch or a commit and makes it the base | anywhere |
| Cmd-comma | Ctrl-comma | Opens the settings screen in the place of the panes, and closes it | anywhere |
| Cmd-Shift-N | Ctrl-Shift-N | Asks for the name of a new branch and makes a worktree on it, with the chosen agent CLI started in it | anywhere |
| Cmd-B | Ctrl-B | Pins the sidebar as a rail, or as the full list | anywhere |
| Cmd-Alt-B | Ctrl-Alt-B | Lets the width of the window choose the form of the sidebar again | anywhere |
| Cmd-Alt-Down, Cmd-Alt-Up | Ctrl-Alt-Down, Ctrl-Alt-Up | Next and previous worktree | anywhere, but on macOS not in the field for the base |
| Cmd-T | Ctrl-T | Opens a terminal for the worktree on screen | anywhere |
| Cmd-Shift-W | Ctrl-Shift-W | Closes the focused terminal | anywhere |
| Cmd-Shift-T | Ctrl-Shift-T | Opens a terminal that runs the chosen agent CLI, for the worktree on screen | anywhere |
| Cmd-Alt-C | Ctrl-Alt-C | Chooses the next agent CLI for Cmd-Shift-T, and after the last the first | anywhere |
| Cmd-Shift-R | Ctrl-Shift-R | Resumes the first session of the worktree on screen that a restart cut off and that has no resume open | anywhere |
| Cmd-Alt-E | Ctrl-Alt-E | Says yes to the question about pi's extension, or, with none showing, yes again | anywhere |
| Cmd-Alt-Shift-E | Ctrl-Alt-Shift-E | Says no to it, or, with none showing, no: it takes the entry out of pi's settings | anywhere |
| Cmd-], Cmd-[ | Ctrl-], Ctrl-[ | Next and previous terminal of the worktree on screen | anywhere |
| Cmd-D | Ctrl-D | Split view of two terminals of the worktree on screen, on and off | anywhere |
| Cmd-Alt-P | Ctrl-Alt-P | Moves the terminal pane to the left of the diff, below it, and back to the right | anywhere |
| Ctrl-Tab, Ctrl-Shift-Tab | the same | Moves the focus to the next and the previous pane | anywhere, a terminal included |
| Cmd-C, Cmd-V | Ctrl-Shift-C, Ctrl-Shift-V | Copies the selected text of a terminal, pastes into it | in a terminal |
| Shift-PageUp, Shift-PageDown | the same | Scrolls the history of a terminal | in a terminal |
| Cmd-W | Ctrl-W | Closes the window; the agents and terminals go on | anywhere |
| Cmd-H | none | Hides the app | anywhere |
| Cmd-Alt-H | none | Hides every other app | anywhere |
| Cmd-M | none | Puts the window in the Dock | anywhere |
| Cmd-K | none; the key reaches the program | Empties the history and the screen above the prompt's line | in a terminal |
| Cmd-A | none; the key reaches the program | Selects everything in the terminal | in a terminal |
| Cmd-Home, Cmd-End | none; the keys reach the program | Scrolls the terminal to the top of its history and back to the bottom | in a terminal |
| Cmd-Up, Cmd-Down | none; the keys reach the program | Scrolls the terminal one line up and one line down | in a terminal |
| Cmd-Q | Ctrl-Q | Quits; the agents and terminals go on | anywhere |
| Cmd-Alt-Q | Ctrl-Alt-Q | Quits and stops every agent and program the app holds | anywhere |
| Cmd-Alt-N | Ctrl-Alt-N | Shows the worktree where an agent just finished a turn | anywhere |
| Cmd-Alt-V | Ctrl-Alt-V | Shows, or closes, what will be sent of the review attached at the agent's terminal | anywhere; acts on the terminal that has the focus in the pane |
| Cmd-Alt-X | Ctrl-Alt-X | Detaches the review attached at the agent's terminal | anywhere |
| Cmd-Alt-A | Ctrl-Alt-A | Types "Address each comment." at the agent's prompt, with no Enter | anywhere, while the agent waits at no permission prompt |
| `j` or Down, `k` or Up | the same | One line down, one line up | in the diff |
| `s` | the same | Switches between the unified and the split diff, on the same line | in the diff |

On Linux, a terminal with the focus keeps only some of these keys; see
[Keys in a terminal](#keys-in-a-terminal).

### Keys in the review pane

They work in the diff and in Flow, as plain keys.

| Key | Does |
| --- | --- |
| `j`, `k`, Down, Up | One line down, one line up |
| `s` | Switches between the unified and the split diff |
| `l`, `h` | Next and previous name on the line |
| Shift-K | Shows what the language server reports for the name |
| `g d` | Goes to the definition of the name and opens it in the file view |
| `g r` | Lists the references to the name |
| Escape | Returns from the file view to the place left in the diff; with nothing to return from, does nothing |
| Shift-I | Downloads the language server the line offers |
| Shift-U | Updates the downloaded language server, when the line offers it |
| Shift-Y | Answers yes to the question about running language servers from the repository |
| Shift-Q | Answers no to it |
| `c` | Opens a comment box on the line |
| `i` | Edits the draft on the line |
| `o` | Writes a reply in the thread of a sent comment |
| `d d` | Deletes the draft on the line |
| `m`, Shift-M | Next and previous thread |
| `y` | Copies the review as text, sending its comments |
| `r` | Marks the file viewed, or takes the mark away |
| Shift-R | Marks the change reviewed |
| Shift-C | Asks for a message and commits every uncommitted change |
| Shift-T | Keeps the selection at a turn end, and offers what changed, or goes back to selecting it |
| Shift-O | Takes the offer: shows what changed since the last review |
| Shift-A | Shows the review of the next agent session of the worktree |
| `tab` | Moves the focus from the diff to the tree of Changes, and back |
| `f` | Switches between Changes and Flow, in both views |
| Shift-F | Turns follow on and off, in both views |
| Shift-L | Switches Flow between the timeline and the live diff, and keeps the form for the worktree; in Flow only, in Changes it does nothing |
| Shift-G | Resumes follow at the newest step; in Flow only, in Changes it does nothing |
| `a` | Attaches the review to your next message to its agent; offers to start the agent when it does not run |
| `t` | Attaches the thread on your line alone |
| Shift-D | Detaches what is attached, and keeps every draft |
| `x` | Resolves the thread on your line |
| `u` | Reopens the resolved thread on your line |
| Shift-X | Resolves every addressed thread of the review |
| `w` | Shows the line a moved thread's comment was written on, and hides it again |
| Shift-S | Shows in Changes what changed since a round was sent: the newest round that has a state, then the round before at each press, and after the first round the newest again |
| Shift-N | Starts the agent the offer after an attach names: the chosen CLI, the first until `ctrl-n` chooses another |
| `ctrl-n` | Chooses the next CLI of the offer, and after the last the first |

In the tree of Changes, as plain keys:

| Key | Does |
| --- | --- |
| `j`, `k`, Down, Up | Moves the cursor one entry down, one up |
| Space | Marks the entry under the cursor |
| Enter | Shows the change of the marked entries, first to last, or of the one under the cursor |

In the list of references, as plain keys:

| Key | Does |
| --- | --- |
| `j`, `k`, Down, Up | Moves to the next and the previous entry |
| Enter | Opens the entry in the file view |
| Escape | Closes the list and gives the focus back to the rows |

The file view takes no input: no key changes what it shows, and `j`, `k` move the line you are
on. `c` writes a comment on that line.

In the comment box the keys type text. Enter closes the box, Shift-Enter starts a new line,
and Escape closes it too: the draft stays either way, and `d d` deletes it.

### Keys on the settings screen

They work while the screen has the focus, as plain keys.

| Key | Does |
| --- | --- |
| `escape` | Closes the screen and gives the focus back to where it was |
| `j`, `k` | Moves the cursor to the next and the previous row |
| `l`, `h` | Chooses the next and the previous value of the row under the cursor, stopping at the last and the first |
| `enter` | Acts on the row under the cursor where the row acts: the base and the folder for worktrees open their field |
| `shift-c` | Cleans up the recorded states at once |

In the field for a folder, Enter sends the text, an empty field meaning beside the repository,
and Escape closes it; a refusal keeps the field open under its reason.

### Keys in a terminal

Every key typed in a focused terminal reaches its program, but the kept keys. These never
reach the program, and each runs its action in the app. Their names are those the app binds,
as GPUI writes them.

| On macOS | On Linux |
| --- | --- |
| `ctrl-tab`, `ctrl-shift-tab`: the next and the previous pane | the same |
| `shift-pageup`, `shift-pagedown`: scroll the history | the same |
| `cmd-c`, `cmd-v`: copy and paste | `ctrl-shift-c`, `ctrl-shift-v` |
| `cmd-k`, `cmd-a`, `cmd-home`, `cmd-end`, `cmd-up`, `cmd-down`: empty the history and the screen above the prompt's line, select everything, scroll to the top, to the bottom, one line up, one line down; none sends a byte to the program | none: the keys reach the program |
| every Cmd key: `cmd-o`, `cmd-shift-b`, `cmd-,`, `cmd-shift-n`, `cmd-b`, `cmd-alt-b`, `cmd-alt-down`, `cmd-alt-up`, `cmd-alt-p`, `cmd-alt-n`, `cmd-t`, `cmd-shift-w`, `cmd-]`, `cmd-[`, `cmd-d`, `cmd-w`, `cmd-h`, `cmd-alt-h`, `cmd-m`, `cmd-q`, `cmd-alt-q`, `cmd-alt-v`, `cmd-alt-x`, `cmd-alt-a`, `cmd-shift-t`, `cmd-alt-c`, `cmd-shift-r`, `cmd-alt-e`, `cmd-alt-shift-e` run their action, and a Cmd key that has none sends nothing, but Cmd-Left, Cmd-Right and Cmd-Backspace edit the line as in other Mac terminals and send `^A`, `^E` and `^U` | none: the keys of the app are Ctrl keys |

On Linux the keys of the app that are not kept, New Terminal, Split View, Quit and the rest,
work only outside a terminal: in a terminal `ctrl-t`, `ctrl-d`, `ctrl-q` and the others
belong to the program. Leave the terminal with `ctrl-tab` first, then press them. Besides
`ctrl-tab`, `ctrl-shift-tab`, `shift-pageup` and `shift-pagedown`, the only keys kept in a
terminal on Linux are `ctrl-shift-c` and `ctrl-shift-v`. On macOS the row "Option key in a
terminal" of the settings chooses what Option does. By the keyboard layout, the default, it
types characters as the layout says, and on the U.S. layout it acts as Meta, so that Option-B
is `ESC b`; Meta makes it Meta whatever the layout; Characters makes it type characters. On
Linux Alt is always Meta. Text typed through an input method or a dead key reaches
the program once it is composed.
