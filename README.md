# YAAR

Yet another agent reviewer: a desktop app for reviewing what a coding agent changes, while
it changes it. You run Claude Code or pi in its terminal, read the diff beside it, comment
on lines, and send the comments back to the agent.

**Pre-release.** Today the app opens a git repository, shows what a branch changed, lets
you comment on any line of the diff and keeps those comments, and runs terminals beside the
diff. It answers what a name in the diff is, where it is defined and who uses it, from a
language server (below). It hears Claude Code, which you run in one of those terminals as always, and pi, once
you have said yes to one question the app asks (below): the sidebar shows whether each works,
waits for you or has finished a turn, Flow shows what it does as it works, and Changes shows
what the turn changed. A review round with either works end to end: you attach your comments
to your next message, the agent receives them with the message you submit, answers each thread
with a tool of the app, and the threads follow the code it rewrites. You still press Enter
yourself. Until you say yes, pi runs in a terminal as any program does, and the app writes
nothing of pi's and does not hear it.

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
- **Attaches a review to your next message.** `a` in the review pane attaches every unsent
  comment on an unresolved thread; `t` attaches the thread on your line alone, and such a
  send counts as a round too. While a review is attached you can still edit its comments and
  a new draft joins it, but not when one thread is attached alone. Attaching works while the
  agent works and while it waits at a permission prompt. The review stays attached until a
  message you typed carries it: the answer to a permission prompt carries nothing, and nor
  does a turn Claude Code starts by itself. `shift-d` detaches it, and every comment stays a
  draft.
- **Shows the attachment at the agent's terminal.** While a review is attached, a line over
  the bottom of the agent's terminal says so, with three controls: View (Cmd-Alt-V), which
  shows exactly the text Claude Code will receive and keeps it up to date while you edit;
  Detach (Cmd-Alt-X); and "Address each comment." (Cmd-Alt-A), which types that sentence at
  the prompt, for you to submit or to word your own request after. The app writes nothing
  else at the prompt and never presses Enter. The sentence is offered while the agent works
  too, and not while it waits at a permission prompt.
- **Delivers the review whole, to the agent it belongs to.** When you submit the message, the
  review goes with it as Claude Code takes it: under the prompt Claude Code shows
  `yaar: review round 1 attached (2 threads)`. The instruction on how to answer each thread
  always comes first, whatever you wrote. A review whose text, with the instruction, fits in
  10,000 UTF-16 units is in the hook's answer; a longer one is a file in the app's own folder
  that Claude Code reads with its read tool, which the app lets through without a dialog for
  that one file. A review goes only to the session it belongs to, never to another session in
  the same worktree. A message you queue while Claude Code works carries the review as Claude
  Code takes the queued message.
  The comments count as sent, in a new round, when Claude Code takes the message.
- **Lets the agent reply, and only you resolve.** Claude Code answers each thread with the
  tool `reply_to_thread`, which the app's plugin gives it as `plugin:yaar-review:yaar`. The
  tool has no way to resolve. A reply may mark the thread addressed. The app lets the tool run
  without a dialog and answers no other prompt.
- **Does the same for pi.** When you press Enter in pi the review is added to your message,
  instruction first, whole at any length, and it shows in pi's transcript as part of the
  message. pi answers each thread with the same tool, `reply_to_thread`, one tool line each,
  and the tool has no way to resolve. Enter while pi works, which pi takes after the running
  tool, and Alt-Enter, after the run, carry the review as well; the comments count as sent
  in a new round when pi reports the message, which is at Enter.
- **Lists every thread with its state**: draft, sent, addressed, waiting for you (the agent
  replied and did not mark it addressed) or resolved. `x` resolves the thread on your line,
  `u` reopens it, Shift-X resolves every addressed thread.
- **Shows rounds.** A line above the tree names each round and says it is sent until Claude
  Code's first edit after it, then answered. A round's control, or Shift-S, shows in Changes
  what changed since that round's comments were sent.
- **Moves a thread to the code that replaced its line.** A thread whose line only moved follows
  it with no notice. When Claude Code rewrote the line the thread sits on the new code and
  says the code changed, and when it deleted the line the thread sits at the point of
  deletion and says so; `w` shows the line you wrote the comment on.
- **Offers to start an agent for a review.** Attaching a review whose agent is not running
  offers to start that CLI in the worktree with that review (its control, or Shift-N); the
  new session takes that review and its first message carries it. The review of no agent
  offers each CLI the app supports, Claude Code and pi: `ctrl-n` chooses the next one, and
  Shift-N starts the chosen one, the first until you choose another.
- **Offers a launcher for each agent CLI next to the shell.** A row at the top of the terminal
  pane holds a button for a shell and one for each CLI, Claude Code and pi. A click opens a
  terminal for the worktree on screen that runs that CLI, as New Terminal opens a shell.
  Cmd-Shift-T opens the chosen CLI, Claude Code until you press Cmd-Alt-C, which chooses the
  next one. The launcher starts the CLI through your login shell, so it is the same program
  as the one you type in a shell of the app.
- **Follows an agent that runs in no terminal of the app** by its folder, and delivers to it
  and hears its replies as to any other.
- **Keeps reviews through a restart.** After a quit, a crash or a restart of the machine every
  review is as it was left: threads, drafts, rounds, turns and what is attached. A session
  that was running is listed as one to resume.
- **Resumes a session a restart cut off.** When the app opens after a restart of the machine,
  or after a Cmd-Alt-Q, a line under the launchers offers "Resume" and the session's name for
  each session of the worktree on screen that was running. A click, or Cmd-Shift-R for the
  first one not yet opened, opens a terminal that runs the CLI's own resume command with the
  session's id: `claude --resume '<id>'`, or `pi --session-dir '<folder>' --session '<id>'`.
  It brings back that session, with its review and what was attached, and the line goes when
  the session reports.
- **Marks a file viewed**, which the app forgets when the file changes, and **marks the
  change reviewed**, which records the worktree as it is and starts the next review from
  it. Neither makes a commit.
- **Commits on request.** The commit control asks for a message and makes an ordinary git
  commit of every uncommitted change of the worktree, with your own git identity and
  hooks. It marks nothing reviewed. Nothing commits unless you press it.
- **Keeps a review for each agent session.** Each worktree has one review that belongs to
  no agent. The first agent session started in the worktree, of either CLI, takes it over.
  When the session ends its review stays open, and the next session of the same CLI in that
  worktree takes it over; a session of the other CLI does not. Two sessions in one worktree,
  of one CLI or of both, have a review each; with several, the line above Changes names them
  and Shift-A, or a click on a name, shows the other's.
  The app lists a worktree that git removed, marked "worktree gone", for as long as its
  review holds a comment.
- **Hears Claude Code and pi and shows what each is doing.** The sidebar marks each worktree whose
  agent waits for your permission, has finished a turn you have not reviewed yet, works, or
  is not running; with several sessions in a worktree it shows the first of these in that
  order. An agent that waits at a permission prompt carries the same mark on its terminal.
  pi asks no permission, so a pi session never waits for one. The app never answers a prompt
  or any other dialog of the agent: you answer in the terminal. The one thing it lets through
  is what its own plugin asks for: the reply tool and the read of one review file. A program
  that is neither Claude Code nor pi, a CLI that was not started with the app's pieces (a pi
  you have not said yes for included), and an agent the app does not support are not heard:
  their changes show as changes no agent made.
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
  window closes. The files Changes lists at a turn's end are those of that session's review:
  the paths credited to the session and those no agent made, never a path only another
  session changed.
- **Marks a file two agents changed.** Where two sessions in one worktree changed the same
  file, each session's review shows that file whole, with every change in it, and marks its
  heading "also changed by another agent". The mark is drawn in the list a turn's end selects
  and in the uncommitted changes of the review; a file only one session changed, and a file
  no agent made, carry none.
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
  you send the agent a message while follow is on, the review pane switches to Flow; a turn
  Claude Code starts by itself, as at the end of a background task, does not switch it.
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
- **Shows what a language server reports for a name in the diff.** Click on a name of a
  line, or move to it with `l` and `h`, and press Shift-K: the answer shows under the rows.
  The mouse resting on a name for 400 ms asks the same. `g d` goes to the definition and
  opens it in a read-only file view that takes the place of the rows; Escape, or "Back to the
  diff" in the view, returns to the line, the name and the scroll you left. `g r` lists the
  references: `j`, `k`, Enter and Escape move in the list, open one and close it, and a click
  opens one. A click with Cmd (Ctrl on Linux) held goes to the definition. TypeScript and
  JavaScript, Python and Rust have code intelligence, and no other language does.
- **Answers a removed line from the old code.** On a removed line, in the unified and in the
  split view, Shift-K and `g d` ask about the version the change is counted from. At the
  first such question the Keeper writes that version, as plain files, into the app's cache
  folder, and the app starts a second server for it there; the worktree is not touched, and
  the files go when another change is shown or the window closes.
- **Says when a name is found by text match.** While no language server is ready for a
  file, and for a file of a language the app has no server for, the answer to a hover, a
  definition or a list of references is found by searching the files of the worktree for the
  name, and is marked as possibly wrong, in the hover and over the list.
- **Shows the language server of the code on screen.** A line under the bar of the review
  pane, which stays in Changes, in Flow and over the file view, says "ready", "starting;
  names are found by text match meanwhile" or "no language server found" for the file of your
  line, and for the old side on a removed line. It is a line of the pane and never a dialog.
- **Comments on a line reached through a definition or a reference**, in the diff or not,
  in the repository or not: `c` in the file view.
- **Looks for a language server in this order**, and uses the first that starts: the one you
  registered, the project's own (`node_modules` for TypeScript, `.venv` for Python), one
  installed on the machine, one the app downloaded. A server that fails its trial run or does
  not start is passed over for the next. Within each of these the order is that of the
  built-in table:

  | Language | Servers, in order | The app downloads |
  | --- | --- | --- |
  | TypeScript and JavaScript | tsgo, typescript-language-server, vtsls | tsgo |
  | Python | basedpyright, pyright, ty, pyrefly | ty |
  | Rust | rust-analyzer: the first program of that name on the search path of the Window (rustup's runs the project's toolchain), and no copy of the project's own | rust-analyzer |

  "On the machine" means in a folder of the search path the Window was started with. A Window
  started from the Finder or the Dock on macOS has the system's short search path and finds
  none of rustup's, Homebrew's or nvm's; one started with `cargo run` from a shell has the
  shell's.
- **Downloads a language server only after you say yes.** When none is found for a language
  that has a download for your machine, the line offers "Download" with the server's name and
  version (Shift-I). Nothing is fetched before that. The download is one plain GET of the
  publisher's address, npm's registry for tsgo and GitHub for ty and rust-analyzer, with no
  account, no cookie, no proxy and a user agent of `language-server-download`; nothing is
  uploaded. The bytes are checked against the SHA-256 digest pinned in the app and are
  refused when it differs. The server is unpacked into the app's data folder, below, and
  started. When the version the app pins is newer than the one you downloaded, the line
  offers "Update to" (Shift-U); nothing updates by itself. A server you registered is never
  downloaded.
- **Asks before it runs a server that sits inside a repository.** A project's own server,
  such as `node_modules/.../tsc` or `.venv/bin/ty`, runs code from the repository. The line
  asks once for the repository, "Run them" (Shift-Y) or "Not from this repository" (Shift-Q),
  and the answer is kept for the repository, in all its worktrees, and told to every window.
  Until you answer, the next candidate that is not inside the repository runs.
- **Takes a server of your own for a language.** The file `language-servers.toml` in the
  app's data folder holds a row for a language, with the command of its server:

  ```toml
  [[rust.server]]
  command = ["/path/to/rust-analyzer"]
  ```

  It takes the built-in row's place, so that only your servers are tried for those files,
  and the line shows its state as it shows a built-in server's. What such a server answers is
  its own. A row that cannot be read is reported and passed over, and the others are read.
- **Runs the servers of the workspace on screen, and stops the others.** A server starts when
  the workspace comes on screen, if its worktree holds a file the row names (a `Cargo.toml`
  for Rust, a `tsconfig.json` or `package.json` for TypeScript, a `pyproject.toml` or
  `requirements.txt` for Python), and otherwise at the first question about one of its files.
  A workspace off screen for ten minutes has its servers stopped, the old side's included, and
  they start again when it comes back; the ten minutes are fixed. Quitting the
  window stops every server, and waits up to three seconds for them. Each window has a
  Navigator of its own.
- **Keeps a language server out of your repository.** rust-analyzer builds into the app's cache
  folder and runs cargo offline, tsgo has its automatic acquisition of types turned off, and
  nothing a server writes goes into a worktree.
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
  reports to the Keeper and prints what the Keeper answers, and decides nothing: `/hooks`
  lists them as the plugin's. It also names the tool server `yaar-claude-tools` in its
  `.mcp.json`, which `/mcp` lists as `plugin:yaar-review:yaar` with the one tool
  `reply_to_thread`. Nothing
  is written into your Claude Code settings, your `~/.claude` or your worktree, the app
  reads no credential of any CLI, and it does not use Claude Code's editor connection:
  variables that name one are left out of the shell. A `claude` started outside the app
  loads nothing of it.
- **Writes into pi's settings only after you say yes, and then one entry.** When pi is
  installed (the folder `~/.pi/agent` exists), the terminal pane asks, in a line under the
  launchers and the resume line, whether the app may add its extension to pi's settings. The
  line names both files: the extension, `agents/pi/extension/yaar.ts` in the app's data
  folder (see "A Keeper left running"), and `~/.pi/agent/settings.json`. It is asked each
  time a window opens until you answer, and not again after; it blocks nothing, and every
  terminal and key works while it is up. `Install` or Cmd-Alt-E says yes, `No` or
  Cmd-Alt-Shift-E says no.
  A yes adds one string, the extension's absolute path, at the end of the list `extensions`
  in that file, and changes no other byte: not its layout, not the order of its keys. With no
  such list the app adds one, and with no file it writes one as pi would. After it, `pi` typed
  in a shell of the app and `pi` started from its launcher are the same program and load the
  extension, and so does a pi you start in another terminal program, where the extension does
  nothing and prints nothing: it acts only where the shells of the app set
  `YAAR_PI_REPORT`, the reporting program it runs.
  To take the yes back, press Cmd-Alt-Shift-E: it removes that one string and nothing else.
  Pressing Cmd-Alt-E again puts it back. You can also delete the string by hand: the app then
  asks again and never writes it back by itself. A settings file the app cannot read as pi
  reads it, because it is not JSON, holds comments, or its `extensions` is not a list of
  strings, is left as it is, and the line says why. Until a yes and after a no the app writes
  nothing of pi's, and pi runs in the app as any program. Your answer, a no as well, is kept
  in `agents/pi/consent` in the app's data folder.

### Not there yet

- A settings screen (the form of Flow, and your answers about language servers inside a
  repository, are the settings kept, set by their keys; your answer about pi's extension is
  changed by its two keys alone), and creating a worktree. The registry of language servers
  is a file you edit by hand.
- Follow is not kept: every new window starts with it on, however you left the last one.
- Older turns in the timeline: it holds the turns the window has seen in this worktree,
  or the newest one when it opens. An earlier turn cannot be picked.
- A file the agent reads through an `@` mention or a shell command is in neither the
  timeline nor the files read of the live diff: the app hears reads by the read tool of
  Claude Code or of pi only.
- A file you change by hand between two turns shows in the next turn's live diff, which
  counts from the state the worktree was in when that turn started.
- The live diff finds its files with one `git status` for each question, one question at a
  time, so on a large repository it can lag behind the writes by about a second.
- The pieces of the CLIs are not packaged yet: the Mac app and the `.deb` hold the Window and
  the Keeper only. Build the workspace, which puts `yaar-claude-hook`, `yaar-claude-tools`
  and `yaar-pi-report`, pi's reporting program, beside `yaar-keeper`.
- The state of a session that waits for permission stays on until the tool ends: after you
  approve a long command at its dialog, the mark stays until the command finishes.
- A workspace with one idle session and one that ended shows "not running" while the
  ended session still holds its review, and no mark once another session has taken that
  review over, since the ended session is then no longer listed. A session that is idle,
  with nothing to review, draws no mark.
- Comments on a range of lines.
- Two worktrees that share one id at different paths: the app refuses the second.

Known limits of what is there:

- A language server that was running when the Window crashed may go on running on macOS:
  rust-analyzer did not end when its input closed. On Linux the kernel ends each server with
  the Window. A quit of the Window stops every server.
- The download takes no proxy of your shell, so behind a proxy that the network requires
  it cannot fetch. Servers that run on your own Node (typescript-language-server, vtsls,
  basedpyright, pyright) and pyrefly are found on the machine and never downloaded. ty is a
  beta.
- A change an agent makes reaches a server only through the list of changed files of the
  worktree on screen, which the app sends it; tsgo and rust-analyzer watch the folder too.
- The question about a repository takes the line until you answer it, even when a later
  candidate is ready, and nothing lets you change an answer once given.
- Two windows that update one language server: the second window keeps running the older
  version until it searches again.
- Claude Code reports a permission prompt and no other dialog, and pi reports none, so the
  sentence control is offered while another dialog, such as the trust dialog or a question of
  one of your own pi extensions, is up. Nothing is typed until you press the control.
- A review whose text, with the instruction, is longer than 10,000 UTF-16 units is a file, and
  only Claude Code's read tool gets it without a dialog. If the model reads it with a shell command, a search or a glob, Claude
  Code asks you as it asks for any such call.
- A message you queue while Claude Code works fires its hook when you press Enter, and the app
  takes that for a new turn: the account of the running turn ends there as interrupted, and a
  second turn starts. The review is delivered right either way. A message queued in pi
  splits nothing.
- A hook program gives up after two seconds, and the app keeps its answer within 1.5 seconds
  of the report reaching it. If the answer then takes more than the half second left to reach
  the hook program, the comments show as sent and Claude Code never had them.
  A message whose report waited longer than 1.5 seconds is sent no review: the review stays
  attached and goes with the next message.
- The text of an attachment and a review that Claude Code takes inline ride in one frame of
  the Link, at most 16 MiB. A longer text is refused with the reason.
- A round sent in a turn that still runs is not counted: asking for what changed since it
  says in a notice that the turn still runs and selects nothing, and the next press passes to
  the round before. Once the turn ends, the round is counted to the newest state like any
  other. Until the pane has read the history at all, no round is counted and the notice says
  that no state is recorded to count to yet.
- After a yes, pi's subagents read the same settings, so each subagent loads the extension
  and reports a session of its own, from its parent's terminal; its writes are credited to it,
  not to no agent. The app does not tell a subagent from a session you started.
- Another extension of yours that handles a message of pi before the app's does can keep the
  message from the app: the turn the app opened at the message then waits for an end that may
  not come, and the session shows as working until the next run ends.
- A pi session that ended before its first message has no session file, and the app still
  offers to resume it; pi then refuses in its own words.
- pi pointed at another settings folder with `PI_CODING_AGENT_DIR` never reads the entry the
  app wrote under `~/.pi/agent`, and the app hears nothing of it.
- A list of files at a turn's end counts credits from the first state the app recorded when
  nothing was reviewed before, so it can hold a file too many and never one too few. A change
  an agent made before the Keeper last started again counts as no agent's, which shows it in
  every session's list.

| On macOS | On Linux | Does | Works |
| --- | --- | --- | --- |
| Cmd-O | Ctrl-O | Opens a folder | anywhere |
| Cmd-Shift-B | Ctrl-Shift-B | Asks for a branch or a commit and makes it the base | anywhere |
| Cmd-B | Ctrl-B | Pins the sidebar as a rail, or as the full list | anywhere |
| Cmd-Alt-B | Ctrl-Alt-B | Lets the width of the window choose the form of the sidebar again | anywhere |
| Cmd-Alt-Down, Cmd-Alt-Up | Ctrl-Alt-Down, Ctrl-Alt-Up | Next and previous worktree | anywhere, but on macOS not in the field for the base |
| Cmd-T | Ctrl-T | Opens a terminal for the worktree on screen | anywhere |
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
| Cmd-Q | Ctrl-Q | Quits; the agents and terminals go on | anywhere |
| Cmd-Alt-Q | Ctrl-Alt-Q | Quits and stops every agent and program the app holds | anywhere |
| Cmd-Alt-N | Ctrl-Alt-N | Shows the worktree where an agent just finished a turn | anywhere |
| Cmd-Alt-V | Ctrl-Alt-V | Shows, or closes, what will be sent of the review attached at the agent's terminal | anywhere; acts on the terminal that has the focus in the pane |
| Cmd-Alt-X | Ctrl-Alt-X | Detaches the review attached at the agent's terminal | anywhere |
| Cmd-Alt-A | Ctrl-Alt-A | Types "Address each comment." at the agent's prompt, with no Enter | anywhere, while the agent waits at no permission prompt |
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
| `l`, `h` | Next and previous name on the line | vim, one step right and left, here a name at a time; vim's word motions `w` and `b` are not used, since `w` shows the original line |
| Shift-K | Shows what the language server reports for the name | vim, `K` looks up the word under the cursor |
| `g d` | Goes to the definition of the name and opens it in the file view | vim, `gd` |
| `g r` | Lists the references to the name | neovim's `grr`; vim's `gr` replaces text, which a read-only view never does |
| Escape | Returns from the file view to the place left in the diff; with nothing to return from, does nothing | tuicr, Escape leaves a mode; vim's `ctrl-o` jumps back, but it is Open Folder on Linux |
| Shift-I | Downloads the language server the line offers | the app; "install" |
| Shift-U | Updates the downloaded language server, when the line offers it | the app |
| Shift-Y | Answers yes to the question about running language servers from the repository | the app; "yes" |
| Shift-Q | Answers no to it | the app |
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
| `a` | Attaches the review to your next message to its agent; offers to start the agent when it does not run | the app; neither vim nor tuicr has the action |
| `t` | Attaches the thread on your line alone | the app; neither has the action |
| Shift-D | Detaches what is attached, and keeps every draft | the app; vim's `D` deletes to the line's end |
| `x` | Resolves the thread on your line | the app; tuicr resolves remote threads through a command only |
| `u` | Reopens the resolved thread on your line | the app; vim's `u` undoes, the nearest action |
| Shift-X | Resolves every addressed thread of the review | the app; neither has the action |
| `w` | Shows the line a moved thread's comment was written on, and hides it again | the app; neither has the action |
| Shift-S | Shows in Changes what changed since a round was sent: the newest round that has a state, then the round before at each press, and after the first round the newest again | the app; vim's `S` substitutes a line |
| Shift-N | Starts the agent the offer after an attach names: the chosen CLI, the first until `ctrl-n` chooses another | the app; neither has the action |
| `ctrl-n` | Chooses the next CLI of the offer, and after the last the first | the app; vim's `ctrl-n` is a second key for a line down, which `j` and Down already give |

In the tree of Changes, as plain keys:

| Key | Does | From |
| --- | --- | --- |
| `j`, `k`, Down, Up | Moves the cursor one entry down, one up | tuicr, commit selector |
| Space | Marks the entry under the cursor | tuicr, commit selector |
| Enter | Shows the change of the marked entries, first to last, or of the one under the cursor | tuicr, commit selector |

In the list of references, as plain keys:

| Key | Does | From |
| --- | --- | --- |
| `j`, `k`, Down, Up | Moves to the next and the previous entry | tuicr's lists and its commit selector |
| Enter | Opens the entry in the file view | tuicr, commit selector |
| Escape | Closes the list and gives the focus back to the rows | tuicr |

The file view takes no input: no key changes what it shows, and `j`, `k` move the line you are
on. `c` writes a comment on that line.

In the comment box the keys type text. Enter closes the box, Shift-Enter starts a new line,
and Escape closes it too: the draft stays either way, and `d d` deletes it.

On macOS the field where you type a base takes Cmd-Alt-Down and Cmd-Alt-Up for itself while
it has the focus, so those two keys do not change the worktree there; they do again once
the focus is elsewhere.

On Linux the keys of the app are those of the Mac with Ctrl in place of Cmd. The three keys
at the agent's terminal act on the terminal that has the focus in the pane, and work from the
review pane, as every key of the app does there. With nothing pinned, the sidebar is a rail
in a window narrower than 1000 px.

### Keys in a terminal

Every key typed in a focused terminal reaches its program, but the kept keys. These never
reach the program, and each runs its action in the app. Their names are those the app binds,
as GPUI writes them.

| On macOS | On Linux |
| --- | --- |
| `ctrl-tab`, `ctrl-shift-tab`: the next and the previous pane | the same |
| `shift-pageup`, `shift-pagedown`: scroll the history | the same |
| `cmd-c`, `cmd-v`: copy and paste | `ctrl-shift-c`, `ctrl-shift-v` |
| every Cmd key: `cmd-o`, `cmd-shift-b`, `cmd-b`, `cmd-alt-b`, `cmd-alt-down`, `cmd-alt-up`, `cmd-alt-p`, `cmd-alt-n`, `cmd-t`, `cmd-]`, `cmd-[`, `cmd-d`, `cmd-q`, `cmd-alt-q`, `cmd-alt-v`, `cmd-alt-x`, `cmd-alt-a`, `cmd-shift-t`, `cmd-alt-c`, `cmd-shift-r`, `cmd-alt-e`, `cmd-alt-shift-e` run their action, and a Cmd key that has none sends nothing, but Cmd-Left, Cmd-Right and Cmd-Backspace edit the line as in other Mac terminals and send `^A`, `^E` and `^U` | none: the keys of the app are Ctrl keys |

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

The Navigator keeps what it downloads and what you register in the same folder, under
`language-servers/<server>/<version>/` and in `language-servers.toml`. What can be made again is
in the cache folder, `~/Library/Caches/yaar-review/navigator/` on macOS and
`$XDG_CACHE_HOME/yaar-review/navigator/` or `~/.cache/yaar-review/navigator/` on Linux: the old
side of a change that the Keeper writes there, the log of each server, and rust-analyzer's build
folders. The Keeper writes only below that folder when the Window asks it for an old side.

It keeps the opened repositories, the bases you set, and every review with its comments,
drafts, replies, rounds and attachments in one database, in
`~/Library/Application Support/yaar-review/` on macOS and in `$XDG_DATA_HOME/yaar-review/` or
`~/.local/share/yaar-review/` on Linux, which also holds the plugin folder for Claude Code and
the files of the long reviews it delivered, under `agents/claude/reviews/`, kept as a record
of what was sent. For pi it holds `agents/pi/extension/yaar.ts`, the extension, written again at
each start of the Keeper, and `agents/pi/consent`, your answer to the question about it. The states of
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
that drive git set the environment of their process. One test, the one of pi's extension
(`crates/keeper/adapter-pi/tests/extension.rs`), runs Node, 22.18 or later, and fails with a
sentence that says so where Node is missing or older; no other test and nothing of the app
uses Node. Four tests run a real language server, tsgo, ty or rust-analyzer: they read the
folder that `YAAR_LANGUAGE_SERVERS` names and fail with a sentence that says so when it is
unset or lacks a pinned version; they never skip. `cargo xtask language-servers <folder>` fills
that folder from the publishers, with the app's own downloader, and finds nothing to fetch in a
folder that is already filled. One test, LSP-01, downloads the three servers from their
publishers into its own scratch folder; every other test of the downloader is served by a
listener on the loopback address, and every other test of the Navigator runs a stand-in
language server for an invented language. Each real server a test starts has a home, cache,
temporary and cargo folder of its own. CI also runs `cargo deny check` and
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
states and the commit, the database, the workspaces with their watcher, the review engine
(threads, rounds, the send, the follower that moves threads), the terminals (the Dock), the
Switchboard that follows each agent session with the steps it takes, and the adapters of
Claude Code and of pi, with the companion of Claude Code (its hook program and its tool server),
the extension of pi and its reporting program; in the Window, the Navigator, which finds,
starts and asks the language servers, with the registry of servers, the downloader, the old
side of a diff and the search by text match, the frame with its sidebar and
its settings, the review pane with Changes and Flow and the file view, and the terminal pane with the launchers,
the resume line, the question about pi's extension and the attachment at the agent's terminal.
Every other crate is still a shell that says in its `lib.rs` which part of the app it will
hold.

Tests come at three levels. Unit tests sit in a crate's source. Component tests sit in its
`tests/` folder and use only what other crates may use; those of the Window run on GPUI's
test platform, which draws nothing. System tests sit in `system-tests/` and start the built
programs.

## Licence

Apache License, Version 2.0; see `LICENSE` and `NOTICE`. Contributions are taken under the
same licence, with no contributor agreement.
