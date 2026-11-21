//! The line of the language server of the code on screen: whether it is ready, starting or
//! missing, the offer to download a missing one, and the question before a server inside a
//! repository first runs (FR-057, FR-059, FR-062, FR-063, FR-067, FR-070).
//!
//! # Which server
//!
//! The code on screen is the file of the line the reviewer is on, or of the file view's
//! line: its server is `Navigation::state_for` of the workspace on screen, that file, and
//! for a removed line the version its old side is read in (the version a change is counted
//! from, as a comment on that line records it). The one exception is the file view's file
//! of the worktree or of an export: it is read in the workspace its kind names, which may be
//! another workspace of the repository, so that the line names the server that answers the
//! file view's asks. A file outside the worktree is read in the workspace on screen. The
//! pane observes the model and draws the line again at each change.
//!
//! # The line
//!
//! One line, under the bar and above everything else of the pane, in Changes, in Flow and
//! over the file view alike, under the selector `review-servers`. It stays while its server
//! is known: it is a line of the pane and never a dialog, and nothing the reviewer does but
//! acting on it takes it away (FR-059). It says, for a built-in row and one the user
//! registered alike (FR-070), the row's title and where its server stands, with the text
//! selector `review-servers-state:<ready|starting|missing|asking|downloading>`:
//!
//! | State | Words, under `review-servers-text:<words>` | Controls |
//! | --- | --- | --- |
//! | ready | `<title>: ready` | the update control, when the state shows an update: [`UPDATE`] `<version>` under `review-servers-update`, key [`crate::UpdateServer`] (`shift-u`) |
//! | starting | `<title>: starting; names are found by text match meanwhile` | |
//! | missing, with an offer | `<title>: no language server found` | [`DOWNLOAD`] `<server> <version>` under `review-servers-download`, key [`crate::DownloadServer`] (`shift-i`) |
//! | missing, no offer | `<title>: no language server found; names are found by text match` | |
//! | asking | `<title>: <program> lies inside the repository <name>. Run language servers from this repository?` | [`ALLOW`] under `review-servers-yes`, [`REFUSE`] under `review-servers-no`, keys [`crate::AllowServers`] (`shift-y`) and [`crate::RefuseServers`] (`shift-q`) |
//! | asking, answered | `<title>: language servers of the repository <name> may run`, or `... are kept out` | |
//! | downloading | `<title>: downloading <server> <version>` | |
//!
//! Each control's drawn text carries the selector `review-servers-<control>:<its words>`,
//! as the question for leave's do, so that a control that does one thing and reads another
//! fails its test.
//!
//! The question is asked once for a repository. The Navigator keeps telling the question
//! until it has heard the answer, and the pane already knows the answer from the Frame's
//! model (`Navigation::consent`, set at once by the click or the key, and told again by
//! the Keeper), so a told state that still asks about a repository with an answer shows
//! the state `asking` with the words of the row "asking, answered" and no controls.
//!
//! A server is known for the code on screen when the Navigator told its state. The
//! server of a removed line's old side is told once the old side is first used, so until
//! then the line of a removed line is that of the new side's server of the language, its
//! offer and controls included; once a state of the old side is told, the line is the old
//! side's. A file of no known language has no line, nor has one whose language has no
//! state told.
//!
//! # What the controls do
//!
//! The download control asks `Navigation::download` of the row (FR-058); the update control
//! `Navigation::update`. The question's controls answer through `Navigation::answer_consent`
//! for the repository asked about, which the Keeper keeps as a setting of that repository
//! and tells every Window: a repository with an answer is never asked again, from any of its
//! worktrees (FR-062), and the line shows the next state the Navigator tells. A key whose
//! control is not on the line (nothing to download, nothing to update, no question shown)
//! says so in the notice and sends nothing.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::Side;
use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{FileKind, ServerState, State};

use crate::ReviewPane;
use crate::draw::ROW_HEIGHT;
use crate::threads::path_in;

/// The words of the control that downloads, before the server's name and version.
pub const DOWNLOAD: &str = "Download";

/// The words of the control that updates, before the version.
pub const UPDATE: &str = "Update to";

/// The words of the control that answers yes.
pub const ALLOW: &str = "Run them";

/// The words of the control that answers no.
pub const REFUSE: &str = "Not from this repository";

/// What the notice says when a key of the line has nothing to act on.
const NOTHING_TO_DOWNLOAD: &str = "No language server waits to be downloaded";
const NOTHING_TO_UPDATE: &str = "No language server update waits";
const NO_QUESTION: &str = "No question about language servers waits";

/// The code whose server the line tells of.
struct Code {
    workspace: WorkspaceId,
    /// The file's path in the version it is read in: its old path on a removed line.
    path: std::path::PathBuf,
    /// The version of the old side for a removed line; none for the files on disk.
    old_side: Option<String>,
    /// Whether the server of the new side stands in for the old side's while the Navigator
    /// has told no state of that: so on a removed line of the diff.
    new_side_until_told: bool,
}

/// What a control of the line does when it is clicked.
#[derive(Clone, Copy)]
enum Act {
    Download,
    Update,
    Allow,
    Refuse,
}

impl ReviewPane {
    /// The state of the server of the code on screen, as the line shows it; none while
    /// there is none.
    pub fn server_on_screen(&self, cx: &App) -> Option<ServerState> {
        let code = self.code_on_screen(cx)?;
        let navigation = self.models.navigation.read(cx);
        navigation
            .state_for(&code.workspace, &code.path, code.old_side.as_deref())
            .or_else(|| {
                code.new_side_until_told
                    .then(|| navigation.state_for(&code.workspace, &code.path, None))
                    .flatten()
            })
            .cloned()
    }

    /// The code on screen: the file of the line the reviewer is on, read in the version
    /// its side of the line belongs to.
    ///
    /// While a file view is open its file is the code on screen: a file of the worktree is
    /// read on disk, one of an export in the version it lies in, and one outside the
    /// worktree in the files on disk of the workspace on screen, as the asks of the file
    /// view place it.
    fn code_on_screen(&self, cx: &App) -> Option<Code> {
        if let Some(shown) = self.file_view() {
            let file = &shown.file;
            return match &file.kind {
                FileKind::Worktree {
                    workspace,
                    relative,
                } => Some(Code {
                    workspace: workspace.clone(),
                    path: relative.clone(),
                    old_side: None,
                    new_side_until_told: false,
                }),
                FileKind::Version {
                    workspace,
                    version,
                    relative,
                } => Some(Code {
                    workspace: workspace.clone(),
                    path: relative.clone(),
                    old_side: Some(version.clone()),
                    new_side_until_told: false,
                }),
                FileKind::Outside => Some(Code {
                    workspace: self.models.changes.read(cx).workspace()?.clone(),
                    path: file.path.clone(),
                    old_side: None,
                    new_side_until_told: false,
                }),
            };
        }
        let position = self.position()?;
        let file = &self.files[*self.by_path.get(&position.path)?].file;
        let workspace = self.changes.as_ref()?.workspace.clone();
        let old_side = match position.side {
            Side::New => None,
            Side::Old => Some(self.old_version()?),
        };
        Some(Code {
            workspace,
            path: path_in(file, position.side).to_path_buf(),
            new_side_until_told: old_side.is_some(),
            old_side,
        })
    }

    /// The name of a repository, as the workspaces list it.
    fn repository_name(&self, repository: &RepositoryId, cx: &App) -> String {
        self.models
            .workspaces
            .read(cx)
            .repositories()
            .iter()
            .find(|listed| &listed.id == repository)
            .map_or_else(|| repository.to_string(), |listed| listed.name.clone())
    }

    /// The repository the question on the line is about, while the question waits: the
    /// state asks and the settings hold no answer for the repository.
    fn question(&self, cx: &App) -> Option<RepositoryId> {
        let State::Asking { repository, .. } = self.server_on_screen(cx)?.state else {
            return None;
        };
        self.models
            .navigation
            .read(cx)
            .consent(&repository, cx)
            .is_none()
            .then_some(repository)
    }

    /// The name of the state the line draws, its words, and its controls.
    fn the_line(
        &self,
        server: &ServerState,
        cx: &App,
    ) -> (&'static str, String, Vec<(&'static str, String, Act)>) {
        let title = &server.title;
        let mut controls = Vec::new();
        let (name, words) = match &server.state {
            State::Ready => ("ready", format!("{title}: ready")),
            State::Starting => (
                "starting",
                format!("{title}: starting; names are found by text match meanwhile"),
            ),
            State::Missing { offer: Some(offer) } => {
                controls.push((
                    "download",
                    format!("{DOWNLOAD} {} {}", offer.server, offer.version),
                    Act::Download,
                ));
                ("missing", format!("{title}: no language server found"))
            }
            State::Missing { offer: None } => (
                "missing",
                format!("{title}: no language server found; names are found by text match"),
            ),
            State::Asking {
                repository,
                program,
            } => {
                let name = self.repository_name(repository, cx);
                let answer = self.models.navigation.read(cx).consent(repository, cx);
                let words = match answer {
                    None => {
                        controls.push(("yes", ALLOW.to_owned(), Act::Allow));
                        controls.push(("no", REFUSE.to_owned(), Act::Refuse));
                        format!(
                            "{title}: {} lies inside the repository {name}. \
                             Run language servers from this repository?",
                            program.display()
                        )
                    }
                    Some(true) => {
                        format!("{title}: language servers of the repository {name} may run")
                    }
                    Some(false) => {
                        format!("{title}: language servers of the repository {name} are kept out")
                    }
                };
                ("asking", words)
            }
            State::Downloading { offer } => (
                "downloading",
                format!("{title}: downloading {} {}", offer.server, offer.version),
            ),
        };
        if let Some(update) = &server.update
            && matches!(server.state, State::Ready)
        {
            controls.push((
                "update",
                format!("{UPDATE} {}", update.version),
                Act::Update,
            ));
        }
        (name, words, controls)
    }

    /// The line, drawn under the bar; none while no server is known for the code on screen.
    pub(crate) fn render_servers(&self, cx: &Context<Self>) -> Option<Div> {
        let server = self.server_on_screen(cx)?;
        let (name, words, controls) = self.the_line(&server, cx);
        let theme = cx.theme();
        let colour = match server.state {
            State::Missing { .. } | State::Asking { .. } => theme.warning,
            _ => theme.muted_foreground,
        };
        let state = div()
            .debug_selector(|| format!("review-servers-state:{name}"))
            .flex_none()
            .text_color(colour)
            .child(name);
        // The words may need more than one line: the controls keep their place beside
        // them, and are never pushed out of the pane.
        let text = div()
            .debug_selector(|| format!("review-servers-text:{words}"))
            .flex_1()
            .min_w_0()
            .child(words.clone());
        let mut line = div()
            .debug_selector(|| "review-servers".to_owned())
            .flex_none()
            .min_h(ROW_HEIGHT)
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .border_b_1()
            .border_color(theme.border)
            .child(state)
            .child(text);
        for (control, words, act) in controls {
            line = line.child(
                div()
                    .debug_selector(|| format!("review-servers-{control}"))
                    .flex_none()
                    .px_2()
                    .bg(theme.secondary)
                    .child(
                        div()
                            .debug_selector(|| format!("review-servers-{control}:{words}"))
                            .child(words.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pane, _: &MouseDownEvent, _, cx| {
                            pane.acted(cx);
                            pane.act(act, cx);
                        }),
                    ),
            );
        }
        Some(line)
    }

    /// The Navigation model changed: the line is drawn again. The settings that hold the
    /// answers on the servers inside a repository are heard from the pane's making, in
    /// `ReviewPane::new`.
    pub(crate) fn servers_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// Does what a control of the line, or its key, does.
    fn act(&mut self, act: Act, cx: &mut Context<Self>) {
        let server = self.server_on_screen(cx);
        match act {
            Act::Download => {
                let Some(server) = server
                    .filter(|server| matches!(server.state, State::Missing { offer: Some(_) }))
                else {
                    return self.say(NOTHING_TO_DOWNLOAD, cx);
                };
                self.models
                    .navigation
                    .update(cx, |navigation, _| navigation.download(&server.language));
            }
            Act::Update => {
                let Some(server) = server.filter(|server| {
                    server.update.is_some() && matches!(server.state, State::Ready)
                }) else {
                    return self.say(NOTHING_TO_UPDATE, cx);
                };
                self.models
                    .navigation
                    .update(cx, |navigation, _| navigation.update(&server.language));
            }
            Act::Allow | Act::Refuse => {
                let Some(repository) = self.question(cx) else {
                    return self.say(NO_QUESTION, cx);
                };
                let allowed = matches!(act, Act::Allow);
                self.models.navigation.update(cx, |navigation, cx| {
                    navigation.answer_consent(repository, allowed, cx);
                });
                // The question goes at once, as the model holds the answer now.
                cx.notify();
            }
        }
    }

    /// What [`crate::DownloadServer`] does.
    pub(crate) fn download_server(
        &mut self,
        _: &crate::DownloadServer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.act(Act::Download, cx);
    }

    /// What [`crate::UpdateServer`] does.
    pub(crate) fn update_server(
        &mut self,
        _: &crate::UpdateServer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.act(Act::Update, cx);
    }

    /// What [`crate::AllowServers`] does.
    pub(crate) fn allow_servers(
        &mut self,
        _: &crate::AllowServers,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.act(Act::Allow, cx);
    }

    /// What [`crate::RefuseServers`] does.
    pub(crate) fn refuse_servers(
        &mut self,
        _: &crate::RefuseServers,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.act(Act::Refuse, cx);
    }
}
