//! The line naming the language server of the code on screen; a removed line uses the server
//! of its old side's version. The Navigator keeps asking about a repository until it hears
//! the answer: one the pane already holds shows as answered, with no controls.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_diff::Side;
use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{FileKind, ServerState, State};

use crate::ReviewPane;
use crate::draw::ROW_HEIGHT;
use crate::threads::path_in;

pub const DOWNLOAD: &str = "Download";

pub const UPDATE: &str = "Update to";

pub const ALLOW: &str = "Run them";

pub const REFUSE: &str = "Not from this repository";

const NOTHING_TO_DOWNLOAD: &str = "No language server waits to be downloaded";
const NOTHING_TO_UPDATE: &str = "No language server update waits";
const NO_QUESTION: &str = "No question about language servers waits";

struct Code {
    workspace: WorkspaceId,
    path: std::path::PathBuf,
    old_side: Option<String>,
    /// The new side's server stands in on a removed line until a state of the old side is told.
    new_side_until_told: bool,
}

#[derive(Clone, Copy)]
enum Act {
    Download,
    Update,
    Allow,
    Refuse,
}

impl ReviewPane {
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

    fn repository_name(&self, repository: &RepositoryId, cx: &App) -> String {
        self.models
            .workspaces
            .read(cx)
            .repositories()
            .iter()
            .find(|listed| &listed.id == repository)
            .map_or_else(|| repository.to_string(), |listed| listed.name.clone())
    }

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
        // The words may wrap: the controls keep their place beside them.
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

    /// The Navigation model changed. The settings that hold the answers on servers inside a
    /// repository are watched from `ReviewPane::new`, not here.
    pub(crate) fn servers_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

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
                cx.notify();
            }
        }
    }

    pub(crate) fn download_server(
        &mut self,
        _: &crate::DownloadServer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.act(Act::Download, cx);
    }

    pub(crate) fn update_server(
        &mut self,
        _: &crate::UpdateServer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.act(Act::Update, cx);
    }

    pub(crate) fn allow_servers(
        &mut self,
        _: &crate::AllowServers,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.acted(cx);
        self.act(Act::Allow, cx);
    }

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
