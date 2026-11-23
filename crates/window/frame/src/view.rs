//! The root view of the window.

use std::path::PathBuf;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;

use std::time::Duration;

use yaar_link::{Age, Fact, Head, Hello, SessionId, Setting, TerminalPlace, WorkspaceId};

use crate::agents::Wanted;
use crate::controls::{
    CloseSettings, Control, FocusNextPane, FocusPreviousPane, KEY_CONTEXT, MovePanes, NewWorktree,
    OpenFolder, OpenSettings, QuitAndStopAgents, SelectNextWorkspace, SelectPreviousWorkspace,
    SetBase, ShowFinishedTurn, ToggleSidebar, UnpinSidebar,
};
use crate::keeper;
use crate::link_model::{KeeperLink, LinkStatus};
use crate::models::Models;
use crate::settings_screen::SettingsScreen;
use crate::sidebar::{Sidebar, workspace_name};
use crate::theme::Appearance;
use crate::trace;

/// A pane, as the Frame holds it. The Frame knows nothing else of a pane: it lays the
/// view out, moves the focus to it, and routes to it the actions it handles.
pub struct Pane {
    /// The name of the pane, for the Frame's own use and for whoever lists the panes.
    pub name: &'static str,
    /// What the pane draws.
    pub view: AnyView,
    /// Where the keys go when the pane has the focus.
    pub focus: FocusHandle,
}

/// Builds one pane for a window, from the shared models: all the Frame asks of a pane.
pub type BuildPane = Box<dyn FnOnce(&Models, &mut Window, &mut App) -> Pane>;

/// How long the quit that stops everything waits for the stop frame to leave before it quits
/// all the same: a Keeper that takes nothing must not hold the window open.
const STOP_WAIT: Duration = Duration::from_secs(1);

/// The notice of a turn that ended in a workspace the window did not show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnNotice {
    /// The workspace the turn ended in, which the notice's control puts on screen.
    pub workspace: yaar_link::WorkspaceId,
    /// The session whose turn it was.
    pub session: yaar_link::SessionId,
    /// What the notice says: the session's title and the workspace's name, as
    /// `Claude Code 1 finished a turn in feature/login`.
    pub text: String,
}

/// One pane, as its crate hands it to the Window program: everything that has to be
/// registered for the pane to work, as one value. The Window program keeps one list of
/// these and reads both the keys and the panes from it, so a pane cannot be in the
/// window without its keys or the other way round.
#[derive(Debug, Clone, Copy)]
pub struct PaneEntry {
    /// The name of the pane, as [`Pane::name`] carries it.
    pub name: &'static str,
    /// The actions of the pane with their keys.
    pub controls: fn() -> Vec<Control>,
    /// Makes the builder of the pane for one window.
    pub build: fn() -> BuildPane,
}

/// Where the panes after the first lie, beside the first: the terminal pane beside the
/// review pane, in the app as the Window program composes it. There is no other place. It
/// is a setting of the app, [`crate::SettingList::terminal_place`], so the Keeper keeps it
/// and every window shows the place the user chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Beside {
    /// To the right of the first pane. Where they lie until the user moves them.
    #[default]
    Right,
    /// Below the first pane.
    Below,
    /// To the left of the first pane.
    Left,
}

impl Beside {
    /// The place [`crate::MovePanes`] moves the panes to from here: right, below, left,
    /// and right again.
    pub fn next(self) -> Beside {
        match self {
            Beside::Right => Beside::Below,
            Beside::Below => Beside::Left,
            Beside::Left => Beside::Right,
        }
    }

    /// The place a setting of the app names.
    fn of(place: TerminalPlace) -> Beside {
        match place {
            TerminalPlace::Right => Beside::Right,
            TerminalPlace::Below => Beside::Below,
            TerminalPlace::Left => Beside::Left,
        }
    }

    /// The setting of the app that names this place.
    fn as_setting(self) -> TerminalPlace {
        match self {
            Beside::Right => TerminalPlace::Right,
            Beside::Below => TerminalPlace::Below,
            Beside::Left => TerminalPlace::Left,
        }
    }
}

/// The root view of the window: the sidebar, the panes, and a line that says how the
/// Window stands with the Keeper while there is nothing else to show.
///
/// The Frame handles the actions of [`crate::controls`]:
/// - [`crate::OpenFolder`] asks for a folder in the system's dialog and hands it to
///   [`crate::WorkspaceList::open_folder`];
/// - [`crate::SetBase`] opens a one-line field; Enter hands its text to
///   [`crate::WorkspaceList::set_base`], Escape closes it. While no workspace is on
///   screen there is nothing to set a base for, and no field opens;
/// - [`crate::ToggleSidebar`] and [`crate::UnpinSidebar`] pin and unpin the sidebar;
/// - [`crate::SelectNextWorkspace`] and [`crate::SelectPreviousWorkspace`] change the
///   workspace on screen;
/// - [`crate::FocusNextPane`] and [`crate::FocusPreviousPane`] move the focus from pane
///   to pane, in the order they are laid out, round from the last to the first;
/// - [`crate::MovePanes`] sets the next place of [`Beside`] as the setting of the terminal
///   pane's place (`SettingList::set`, so the model has it at once and the Keeper keeps
///   it); the Frame lays the panes out from that setting, which a `SettingChanged` told by
///   the Keeper moves too, and from the settings the Keeper listed when the window
///   connected;
/// - [`crate::OpenSettings`] makes a [`crate::SettingsScreen`] and draws it in the place
///   of the panes, below the notices, the sidebar beside it; the screen takes the focus,
///   and what closes it, the same key or [`crate::CloseSettings`], drops the screen and
///   gives the focus back to what had it when the screen opened (the Frame's own place for
///   it where that is gone). [`Frame::settings_screen`] is the screen while it is open.
///   The keys that move the focus between the panes do nothing meanwhile: no pane is
///   drawn;
/// - [`crate::NewWorktree`] opens a one-line field for the name of a new branch (as the
///   base's: Enter takes the name, Escape gives up, an empty field stays open). Enter tells
///   the list of terminals, with `TerminalList::want`, to make a worktree
///   ([`crate::Wanted::Worktree`]) of the repository of the workspace on screen, from the
///   branch checked out in it, with the agent of [`crate::AgentList::chosen_cli`]. With no
///   workspace on screen, a workspace that has no branch checked out, or no CLI to start,
///   no field opens, the notice says why, and nothing is wanted;
/// - [`crate::ShowFinishedTurn`] puts on screen the workspace of the notice of a finished
///   turn, see [`Frame::turn_notice`];
/// - [`crate::QuitAndStopAgents`] sends the stop frame, waits a moment for it to leave, and
///   quits the app. Nothing in the Frame handles [`crate::Quit`]: the Window program does.
///
/// The first pane fills half the room beside the sidebar; the panes after it share the
/// other half, one above the other when they lie to the right or the left of the first,
/// side by side when they lie below it. Each pane is drawn inside an element with the
/// selector `frame-pane-<name>`, which `debug_bounds` finds while it is drawn.
///
/// The Frame's root element hands every action a pane routed through
/// [`crate::Models::routes`] to that pane.
///
/// It shows the reason when the Keeper refuses what the user asked. Whenever the window
/// comes to the front it asks for the list of workspaces and for the changes again,
/// by [`crate::WorkspaceList::refresh`] and [`crate::WorkspaceChanges::refresh`].
///
/// When the window opens, the first pane has the focus, so that its keys work at once.
/// A click on what the Frame draws around the panes, as on the sidebar or on the notice,
/// leaves the focus where it was.
///
/// Its root element sets the key context [`crate::KEY_CONTEXT`]. A key bound in that
/// context works wherever the focus is in the window, in a pane and in the field for the
/// base too, unless what has the focus binds the same key for itself. A Frame with its
/// sidebar keeps the focus inside what it draws, so such a key always has a place to
/// work from.
///
/// For tests, which cannot read what is drawn, some elements carry a selector that
/// `debug_bounds` finds while they are drawn: the notice, `frame-notice`, the note about
/// an older Keeper, `frame-keeper-note`, with its offer of the quit that stops everything,
/// `frame-keeper-note-stop`, the notice of a finished turn, `frame-turn-notice`, with its
/// control, `frame-turn-notice-show`, the field for the base, `frame-base-field`, the field
/// for a new worktree's branch, `frame-worktree-field`, and each pane's place,
/// `frame-pane-<name>`; the settings screen draws `settings-screen` in the place of the
/// panes.
pub struct Frame {
    link: Entity<KeeperLink>,
    first_frame_marked: bool,
    // None in a Frame made by `new`, which shows the link alone.
    composed: Option<Composed>,
}

struct Composed {
    models: Models,
    sidebar: Entity<Sidebar>,
    panes: Vec<Pane>,
    /// Where the focus is while it is in no pane and in no field. An action reaches
    /// the Frame only from a place of the focus inside what the Frame draws, so the
    /// focus is never left nowhere.
    focus: FocusHandle,
    /// The field for the base, while it is open.
    base_field: Option<LineField>,
    /// The field for the name of a new worktree's branch, while it is open.
    worktree_field: Option<LineField>,
    /// The settings screen, from its opening to its closing.
    settings: Option<OpenScreen>,
    /// Where the panes after the first lie: the setting of the place, as the settings
    /// model told it last.
    beside: Beside,
    /// Why a new worktree was not asked for, the last time it was tried; the user asking
    /// for anything that could be refused takes it away.
    worktree_notice: Option<String>,
    /// Why the dialog for a folder could not be shown, the last time it was asked for; the
    /// user asking for anything that could be refused takes it away, a new worktree
    /// included.
    no_dialog: Option<String>,
    /// The workspace and the session of the last turn that ended where the window did not
    /// look, until its workspace comes on screen.
    finished_elsewhere: Option<(WorkspaceId, SessionId)>,
}

/// The one-line field the user types a base or a branch's name into.
struct LineField {
    input: Entity<InputState>,
    /// Where the focus was when the field opened, and goes back to when it closes.
    opened_from: Option<FocusHandle>,
    _enter: Subscription,
}

/// The settings screen of the Frame while it is open.
struct OpenScreen {
    screen: Entity<SettingsScreen>,
    /// Where the focus was when the screen opened, and goes back to when it closes.
    opened_from: Option<FocusHandle>,
}

impl Frame {
    /// A Frame that shows `link` and redraws when it changes. It has no sidebar and no
    /// pane, and it gives the focus no place: a key bound in a context, that of the
    /// Frame included, does nothing in it.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        cx.observe(&link, |_, _, cx| cx.notify()).detach();
        Self {
            link,
            first_frame_marked: false,
            composed: None,
        }
    }

    /// A Frame with its sidebar and with the panes that `panes` build, in that order.
    pub fn with_panes(
        models: Models,
        panes: Vec<BuildPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let link = models.link.clone();
        cx.observe(&link, |_, _, cx| cx.notify()).detach();
        // The notice is the refusal the list holds; and the notice of a finished turn goes
        // once its workspace is on screen, whatever put it there, and does not come back
        // when the workspace is left.
        cx.observe(&models.workspaces, |frame, workspaces, cx| {
            if let Some(composed) = &mut frame.composed
                && let Some((workspace, _)) = &composed.finished_elsewhere
                && workspaces.read(cx).selected() == Some(workspace)
            {
                composed.finished_elsewhere = None;
            }
            cx.notify();
        })
        .detach();
        // The notice names the session by its title.
        cx.observe(&models.agents, |_, _, cx| cx.notify()).detach();
        // The place of the panes is the user's setting, kept by the Keeper.
        cx.observe(&models.settings, |frame, settings, cx| {
            if let Some(composed) = &mut frame.composed {
                let place = Beside::of(settings.read(cx).terminal_place());
                if composed.beside != place {
                    composed.beside = place;
                    cx.notify();
                }
            }
        })
        .detach();
        cx.subscribe(&link, |frame, _, fact: &Fact, cx| {
            if let Fact::TurnEnded {
                workspace, session, ..
            } = fact
            {
                frame.turn_ended(workspace, session, cx);
            }
        })
        .detach();

        let sidebar =
            cx.new(|cx| Sidebar::with_agents(models.workspaces.clone(), models.agents.clone(), cx));
        let width = window.viewport_size().width;
        sidebar.update(cx, |sidebar, cx| sidebar.set_window_width(width, cx));
        cx.observe_window_bounds(window, |frame, window, cx| {
            let width = window.viewport_size().width;
            if let Some(composed) = &frame.composed {
                composed
                    .sidebar
                    .update(cx, |sidebar, cx| sidebar.set_window_width(width, cx));
            }
        })
        .detach();

        // Files change, branches are switched and worktrees are made while the user is
        // in another app, and no fact tells of it.
        cx.observe_window_activation(window, |frame, window, cx| {
            if !window.is_window_active() {
                return;
            }
            if let Some(composed) = &frame.composed {
                composed
                    .models
                    .workspaces
                    .update(cx, |list, cx| list.refresh(cx));
                composed
                    .models
                    .changes
                    .update(cx, |changes, cx| changes.refresh(cx));
            }
        })
        .detach();

        let focus = cx.focus_handle();
        // What had the focus may leave the window, as a row of a pane does when its
        // file is no longer changed.
        cx.on_focus_lost(window, |frame, window, cx| {
            if let Some(composed) = &frame.composed {
                composed.focus.focus(window, cx);
            }
        })
        .detach();

        let panes: Vec<Pane> = panes
            .into_iter()
            .map(|build| build(&models, window, cx))
            .collect();
        match panes.first() {
            Some(first) => first.focus.focus(window, cx),
            None => focus.focus(window, cx),
        }

        let beside = Beside::of(models.settings.read(cx).terminal_place());
        Self {
            link,
            first_frame_marked: false,
            composed: Some(Composed {
                models,
                sidebar,
                panes,
                focus,
                base_field: None,
                worktree_field: None,
                settings: None,
                beside,
                worktree_notice: None,
                no_dialog: None,
                finished_elsewhere: None,
            }),
        }
    }

    /// The sidebar; none in a Frame made by [`Frame::new`].
    pub fn sidebar(&self) -> Option<&Entity<Sidebar>> {
        self.composed.as_ref().map(|composed| &composed.sidebar)
    }

    /// Where the panes after the first lie now: the place the setting of the terminal
    /// pane's place names, [`Beside::Right`] while none is set. The Frame lays the panes
    /// out from this value, and it follows the setting at once.
    pub fn beside(&self) -> Beside {
        self.composed
            .as_ref()
            .map(|composed| composed.beside)
            .unwrap_or_default()
    }

    /// The settings screen while it is open; none while it is closed, and in a Frame made
    /// by [`Frame::new`]. A screen is made at each opening and dropped at the closing.
    pub fn settings_screen(&self) -> Option<&Entity<SettingsScreen>> {
        self.composed
            .as_ref()?
            .settings
            .as_ref()
            .map(|open| &open.screen)
    }

    /// What the window says about a Keeper of an older version than its own, as the
    /// sentence it draws under the selector `frame-keeper-note`; none with a Keeper of the
    /// same version, of a newer one, or with none reached. It names the Keeper's version
    /// and says that the Keeper and what it runs go on under that version: with another
    /// protocol the window can ask that Keeper for nothing, with the same one it works
    /// with it as it is.
    ///
    /// The window's own version is this crate's. The workspace gives every crate the
    /// app's one version, and the Window program tells the Keeper the same in its hello,
    /// so the two cannot differ.
    pub fn keeper_note(&self, cx: &App) -> Option<String> {
        let mine = Hello::for_this_process(env!("CARGO_PKG_VERSION"));
        let (keeper, can_ask) = match self.link.read(cx).status() {
            LinkStatus::Connected(keeper) => (keeper, true),
            LinkStatus::OtherVersion(keeper) => (keeper, false),
            LinkStatus::Starting | LinkStatus::Failed(_) | LinkStatus::Lost(_) => return None,
        };
        if keeper.age_against(&mine) != Age::Older {
            return None;
        }
        let version = &keeper.app_version;
        let outcome = if can_ask {
            "this window works with it as it is"
        } else {
            "this window can ask it nothing"
        };
        Some(format!(
            "The Keeper that is running is of version {version}, older than this window. \
             It and what runs in it go on under version {version}, and {outcome}."
        ))
    }

    /// The notice of a turn that ended in a workspace the window did not show, drawn under
    /// the selector `frame-turn-notice` with its control under `frame-turn-notice-show`,
    /// which does what [`crate::ShowFinishedTurn`] does. None when there is no such
    /// notice: a turn that ended in the workspace on screen makes none, and putting the
    /// notice's workspace on screen, by its control or otherwise, takes it away. Of two
    /// turns that ended elsewhere, the later one's notice is shown.
    pub fn turn_notice(&self, cx: &App) -> Option<TurnNotice> {
        let composed = self.composed.as_ref()?;
        let (workspace, session) = composed.finished_elsewhere.as_ref()?;
        let list = composed.models.workspaces.read(cx);
        // The notice goes with the workspace's coming on screen, and is not asked to wait
        // for the observer that drops it.
        if list.selected() == Some(workspace) {
            return None;
        }
        let name = workspace_name(list.workspace(workspace)?);
        // The list of agents may not hold the session yet: the notice says what it can.
        let title = composed
            .models
            .agents
            .read(cx)
            .session(session)
            .map_or("An agent", |session| session.title.as_str());
        Some(TurnNotice {
            workspace: workspace.clone(),
            session: session.clone(),
            text: format!("{title} finished a turn in {name}"),
        })
    }

    /// The Keeper told that a turn ended: unless the workspace is on screen, it is the one
    /// the notice is about now, in place of the notice before it.
    fn turn_ended(&mut self, workspace: &WorkspaceId, session: &SessionId, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        if composed.models.workspaces.read(cx).selected() == Some(workspace) {
            return;
        }
        composed.finished_elsewhere = Some((workspace.clone(), session.clone()));
        cx.notify();
    }

    /// The names of the panes, in the order they are laid out.
    pub fn panes(&self) -> Vec<&'static str> {
        self.composed
            .iter()
            .flat_map(|composed| &composed.panes)
            .map(|pane| pane.name)
            .collect()
    }

    /// Why the last thing the user asked for was not done, as the sentence the Frame
    /// shows: the reason the Keeper gave when it refused, why the dialog for a folder
    /// could not be shown, or why no new worktree could be asked for (no workspace on
    /// screen, no branch checked out in it, no agent CLI). None when nothing is shown; the
    /// user asking for the next thing takes it away. The Frame draws this very sentence.
    pub fn notice(&self, cx: &App) -> Option<String> {
        let composed = self.composed.as_ref()?;
        // A dialog that failed is the newer of the two: it is forgotten as soon as the
        // user asks for anything that could be refused.
        composed
            .no_dialog
            .clone()
            .or_else(|| composed.worktree_notice.clone())
            .or_else(|| {
                let refusal = composed.models.workspaces.read(cx).refusal()?;
                Some(refusal.to_string())
            })
    }

    /// The line the empty window shows about the Keeper.
    pub fn status_line(&self, cx: &App) -> String {
        match self.link.read(cx).status() {
            LinkStatus::Starting => "Starting the Keeper".to_owned(),
            LinkStatus::Connected(keeper) => {
                format!("Keeper {} is running", keeper.app_version)
            }
            LinkStatus::OtherVersion(keeper) => format!(
                "The Keeper that is running belongs to version {} of the app",
                keeper.app_version
            ),
            LinkStatus::Failed(why) => format!("No Keeper: {why}"),
            LinkStatus::Lost(why) => format!("The Keeper went away: {why}"),
        }
    }

    fn open_folder(&mut self, _: &OpenFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        if composed.no_dialog.take().is_some() | composed.worktree_notice.take().is_some() {
            cx.notify();
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |frame, cx| {
            let chosen = chosen.await;
            frame
                .update(cx, |frame, cx| {
                    let Some(composed) = &mut frame.composed else {
                        return;
                    };
                    match chosen {
                        Ok(Ok(Some(folders))) => {
                            if let Some(folder) = folders.into_iter().next() {
                                composed
                                    .models
                                    .workspaces
                                    .update(cx, |list, cx| list.open_folder(folder, cx));
                            }
                        }
                        // The user gave up, or the dialog went away with the app.
                        Ok(Ok(None)) | Err(_) => {}
                        // Where the desktop offers no such dialog, the key would
                        // otherwise do nothing and say nothing.
                        Ok(Err(error)) => {
                            composed.no_dialog =
                                Some(format!("the dialog for a folder cannot be shown: {error}"));
                            cx.notify();
                        }
                    }
                })
                .ok();
        })
        .detach();
    }

    fn set_base(&mut self, _: &SetBase, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        if let Some(open) = &composed.base_field {
            // Open already: the key brings the focus back to it and keeps its text.
            open.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if composed.models.workspaces.read(cx).selected().is_none() {
            return;
        }
        let field = Self::open_field(
            "A branch or a commit as the base",
            Self::take_base,
            window,
            cx,
        );
        if let Some(composed) = &mut self.composed {
            composed.base_field = Some(field);
        }
        cx.notify();
    }

    /// A one-line field that has the focus, holds nothing, and calls `enter` when the user
    /// presses Enter in it. It remembers where the focus was, to give it back.
    fn open_field(
        placeholder: &'static str,
        enter: fn(&mut Self, &mut Window, &mut Context<Self>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LineField {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |frame, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    enter(frame, window, cx);
                }
            },
        );
        let opened_from = window.focused(cx);
        input.update(cx, |input, cx| input.focus(window, cx));
        LineField {
            input,
            opened_from,
            _enter: subscription,
        }
    }

    /// Enter in the field: hands its text to the list as the base of the workspace on
    /// screen and closes the field. A field with nothing in it stays open: there is
    /// nothing to ask the Keeper, and nothing for it to refuse.
    fn take_base(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        let Some(field) = &composed.base_field else {
            return;
        };
        // No name of a branch or of a commit starts or ends with a blank.
        let base = field.input.read(cx).value().trim().to_owned();
        if base.is_empty() {
            return;
        }
        // The user asks for something the Keeper can refuse: what was said of the last
        // ask, of a dialog or of a new worktree, is no longer the news.
        if composed.no_dialog.take().is_some() | composed.worktree_notice.take().is_some() {
            cx.notify();
        }
        let workspaces = composed.models.workspaces.clone();
        self.close_base_field(window, cx);
        workspaces.update(cx, |list, cx| list.set_base(base, cx));
    }

    /// Closes the field for the base and gives the focus back to where it came from.
    fn close_base_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        let Some(field) = composed.base_field.take() else {
            return;
        };
        field
            .opened_from
            .unwrap_or_else(|| composed.focus.clone())
            .focus(window, cx);
        cx.notify();
    }

    /// Opens the field for the name of a new worktree's branch, or says why it cannot.
    fn new_worktree(&mut self, _: &NewWorktree, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        if let Some(open) = &composed.worktree_field {
            // Open already: the key brings the focus back to it and keeps its text.
            open.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        // A new worktree is an ask that can be refused: the notice of an older one, of a
        // dialog that failed included, is no longer the news. The dialog's case has no
        // test: GPUI's test platform answers a path prompt with a folder or none and
        // cannot make it fail.
        composed.worktree_notice = None;
        composed.no_dialog = None;
        // The user is not asked for a name that could not be taken: the reasons that do
        // not depend on the name are told first.
        if let Err(why) = self.worktree_wanted(String::new(), cx) {
            if let Some(composed) = &mut self.composed {
                composed.worktree_notice = Some(why);
            }
            cx.notify();
            return;
        }
        let field = Self::open_field(
            "The name of the new branch",
            Self::take_worktree,
            window,
            cx,
        );
        if let Some(composed) = &mut self.composed {
            composed.worktree_field = Some(field);
        }
        cx.notify();
    }

    /// What a new worktree on `branch` is asked for as: the repository of the workspace on
    /// screen, the branch checked out in it, and the agent of the chosen CLI. Why it cannot
    /// be asked for otherwise, as a sentence.
    fn worktree_wanted(&self, branch: String, cx: &App) -> Result<Wanted, String> {
        const NONE_ON_SCREEN: &str = "No worktree is on screen to make a new one from.";
        let composed = self.composed.as_ref().ok_or("There is no window.")?;
        let list = composed.models.workspaces.read(cx);
        let selected = list.selected().ok_or(NONE_ON_SCREEN)?;
        let (repository, workspace) = list
            .repositories()
            .iter()
            .find_map(|repository| {
                let workspace = repository
                    .workspaces
                    .iter()
                    .find(|workspace| &workspace.id == selected)?;
                Some((repository, workspace))
            })
            .ok_or(NONE_ON_SCREEN)?;
        let name = workspace_name(workspace);
        let from = match &workspace.head {
            Some(Head::Branch(from)) => from.clone(),
            Some(Head::Detached(_)) => {
                return Err(format!(
                    "The worktree on screen has no branch checked out ({name}); a new worktree is made from a branch."
                ));
            }
            None => {
                return Err(format!(
                    "The worktree on screen ({name}) is gone, and a new worktree cannot be made from it."
                ));
            }
        };
        let Some(cli) = composed.models.agents.read(cx).chosen_cli() else {
            return Err(
                "No agent CLI is installed, so no agent can be started in a new worktree."
                    .to_owned(),
            );
        };
        Ok(Wanted::Worktree {
            repository: repository.id.clone(),
            from,
            branch,
            agent: cli.agent.clone(),
        })
    }

    /// Enter in the field for a new worktree: wants the worktree of the name typed, from
    /// what is on screen now, and closes the field. A field with nothing in it stays open.
    /// What was on screen when the field opened may have changed: then the notice says why
    /// the worktree is not wanted, and the field closes.
    fn take_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &self.composed else {
            return;
        };
        let Some(field) = &composed.worktree_field else {
            return;
        };
        // No name of a branch starts or ends with a blank.
        let branch = field.input.read(cx).value().trim().to_owned();
        if branch.is_empty() {
            return;
        }
        let wanted = self.worktree_wanted(branch, cx);
        self.close_worktree_field(window, cx);
        let Some(composed) = &mut self.composed else {
            return;
        };
        match wanted {
            Ok(wanted) => {
                composed.worktree_notice = None;
                composed.no_dialog = None;
                // The worktree is asked of the terminals, not of the list of workspaces,
                // which would have forgotten its older refusal had it been asked.
                composed
                    .models
                    .workspaces
                    .update(cx, |list, cx| list.forget_refusal(cx));
                composed
                    .models
                    .terminals
                    .update(cx, |list, cx| list.want(wanted, cx));
            }
            Err(why) => composed.worktree_notice = Some(why),
        }
        cx.notify();
    }

    /// Closes the field for a new worktree and gives the focus back to where it came from.
    fn close_worktree_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        let Some(field) = composed.worktree_field.take() else {
            return;
        };
        field
            .opened_from
            .unwrap_or_else(|| composed.focus.clone())
            .focus(window, cx);
        cx.notify();
    }

    /// Opens the settings screen in the place of the panes and gives it the focus; with it
    /// open, closes it.
    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        if composed.settings.is_some() {
            self.close_settings(&CloseSettings, window, cx);
            return;
        }
        let models = composed.models.clone();
        let screen = cx.new(|cx| SettingsScreen::new(models, cx));
        let opened_from = window.focused(cx);
        screen.read(cx).focus().clone().focus(window, cx);
        composed.settings = Some(OpenScreen {
            screen,
            opened_from,
        });
        cx.notify();
    }

    /// Drops the settings screen and gives the focus back to where it was when the screen
    /// opened. With none open it does nothing.
    fn close_settings(&mut self, _: &CloseSettings, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        let Some(open) = composed.settings.take() else {
            return;
        };
        open.opened_from
            .unwrap_or_else(|| composed.focus.clone())
            .focus(window, cx);
        cx.notify();
    }

    /// Takes the focus when it is nowhere in what the Frame drew last. A pane that was
    /// given the focus and drew no place for it would otherwise leave the keys of the
    /// Frame dead until the first click.
    fn keep_the_focus_in_what_is_drawn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(composed) = &self.composed
            && !composed.focus.contains_focused(window, cx)
        {
            composed.focus.focus(window, cx);
        }
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(composed) = &self.composed {
            composed
                .sidebar
                .update(cx, |sidebar, cx| sidebar.pin_other_form(cx));
        }
    }

    fn unpin_sidebar(&mut self, _: &UnpinSidebar, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(composed) = &self.composed {
            composed
                .sidebar
                .update(cx, |sidebar, cx| sidebar.pin(None, cx));
        }
    }

    fn select_next_workspace(
        &mut self,
        _: &SelectNextWorkspace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(composed) = &self.composed {
            composed
                .models
                .workspaces
                .update(cx, |list, cx| list.select_next(cx));
        }
    }

    fn select_previous_workspace(
        &mut self,
        _: &SelectPreviousWorkspace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(composed) = &self.composed {
            composed
                .models
                .workspaces
                .update(cx, |list, cx| list.select_previous(cx));
        }
    }

    /// Moves the focus to the pane after the one that holds it, round from the last to
    /// the first; with the focus in no pane, to the first.
    fn focus_next_pane(&mut self, _: &FocusNextPane, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus_between_panes(true, window, cx);
    }

    /// Moves the focus to the pane before the one that holds it, round from the first to
    /// the last; with the focus in no pane, to the last.
    fn focus_previous_pane(
        &mut self,
        _: &FocusPreviousPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_focus_between_panes(false, window, cx);
    }

    fn move_focus_between_panes(&mut self, forward: bool, window: &mut Window, cx: &mut App) {
        let Some(composed) = &self.composed else {
            return;
        };
        let count = composed.panes.len();
        // The panes are not drawn while the settings screen is: a pane given the focus
        // then would hold keys nobody can see.
        if count == 0 || composed.settings.is_some() {
            return;
        }
        let holding = composed
            .panes
            .iter()
            .position(|pane| pane.focus.contains_focused(window, cx));
        let to = match (holding, forward) {
            (Some(at), true) => (at + 1) % count,
            (Some(at), false) => (at + count - 1) % count,
            (None, true) => 0,
            (None, false) => count - 1,
        };
        composed.panes[to].focus.focus(window, cx);
    }

    /// Puts the workspace of the notice of a finished turn on screen; with no notice, it
    /// does nothing. The notice goes once its workspace is on screen.
    fn show_finished_turn(&mut self, _: &ShowFinishedTurn, _: &mut Window, cx: &mut Context<Self>) {
        let Some(notice) = self.turn_notice(cx) else {
            return;
        };
        if let Some(composed) = &self.composed {
            composed
                .models
                .workspaces
                .update(cx, |list, cx| list.select(&notice.workspace, cx));
        }
    }

    /// Quits the Window and stops what the Keeper holds: the stop frame goes first, and
    /// the Window waits a moment for it to leave, since an app that quits at once could
    /// take the frame down with it.
    fn quit_and_stop_agents(
        &mut self,
        _: &QuitAndStopAgents,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_everything_and_quit(cx);
    }

    fn stop_everything_and_quit(&mut self, cx: &mut Context<Self>) {
        self.link
            .update(cx, |link, _| link.stop_everything_and_wait(STOP_WAIT));
        cx.quit();
    }

    /// Sets the place after the one the setting names now. The model has it at once, the
    /// observer of the settings lays the panes out there, and the Keeper is asked to keep
    /// it.
    fn move_panes(&mut self, _: &MovePanes, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(composed) = &self.composed {
            let settings = composed.models.settings.clone();
            let next = Beside::of(settings.read(cx).terminal_place()).next();
            settings.update(cx, |settings, cx| {
                settings.set(Setting::TerminalPlace(next.as_setting()), cx);
            });
        }
    }

    /// The note about an older Keeper, as the element that shows it, with the offer of the
    /// quit that stops everything: the one way out the window has from a Keeper it cannot
    /// replace by itself.
    fn keeper_note_line(&self, cx: &Context<Self>) -> Option<Div> {
        let note = self.keeper_note(cx)?;
        let theme = cx.theme();
        let offer = div()
            .id("frame-keeper-note-stop")
            .debug_selector(|| "frame-keeper-note-stop".to_owned())
            .flex_none()
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .text_color(theme.foreground)
            .cursor_pointer()
            .hover(|offer| offer.bg(theme.list_hover))
            .on_click(cx.listener(|frame, _, _, cx| frame.stop_everything_and_quit(cx)))
            .child("Quit and stop agents");
        Some(
            div()
                .debug_selector(|| "frame-keeper-note".to_owned())
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .text_sm()
                .text_color(theme.muted_foreground)
                .border_b_1()
                .border_color(theme.border)
                .child(div().flex_1().min_w_0().child(note))
                .child(offer),
        )
    }

    /// The panes as the Frame lays them out: the first takes half of the room, and the
    /// others share the other half, stacked where they lie to the right or the left of
    /// the first and side by side where they lie below it.
    fn laid_out(&self, composed: &Composed) -> Div {
        let selector = |pane: &Pane| {
            let name = pane.name;
            move || format!("frame-pane-{name}")
        };
        let slot = |pane: &Pane| {
            div()
                .debug_selector(selector(pane))
                .size_full()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(pane.view.clone())
        };
        let (first, others) = composed.panes.split_first().expect("a pane to lay out");
        let first = slot(first);
        if others.is_empty() {
            return div().flex_1().min_h_0().flex().child(first);
        }
        let below = composed.beside == Beside::Below;
        let rest = if below {
            div().flex_row()
        } else {
            div().flex_col()
        }
        .flex()
        .size_full()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .children(others.iter().map(slot));
        let area = div().flex_1().min_h_0().flex();
        match composed.beside {
            Beside::Right => area.flex_row().child(first).child(rest),
            Beside::Left => area.flex_row().child(rest).child(first),
            Beside::Below => area.flex_col().child(first).child(rest),
        }
    }

    /// The name of the app and the line about the Keeper, in the middle of the room
    /// they are given: what a window shows that has nothing else to show.
    fn empty(&self, cx: &Context<Self>) -> Div {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(div().text_xl().child("YAAR"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.status_line(cx)),
            )
            // A composed Frame draws the note above its panes, where it is seen with them.
            .children(
                self.composed
                    .is_none()
                    .then(|| self.keeper_note_line(cx))
                    .flatten(),
            )
    }
}

impl Render for Frame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_marked {
            self.first_frame_marked = true;
            window.on_next_frame(|_, _| trace::mark("first_frame"));
            // Once this frame is drawn it shows whether the pane that was given the
            // focus has drawn a place for it.
            cx.defer_in(window, Self::keep_the_focus_in_what_is_drawn);
        }
        let theme = cx.theme();
        let (background, foreground, muted, border, danger) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
            theme.danger,
        );
        let root = div()
            .size_full()
            // A key of the app that is no Cmd key is bound in this context. It has to
            // hold wherever the focus can be in the window, so the outermost element
            // sets it.
            .key_context(KEY_CONTEXT)
            .bg(background)
            .text_color(foreground);
        let Some(composed) = &self.composed else {
            return root.child(self.empty(cx));
        };

        let notice = self.notice(cx).map(|notice| {
            div()
                .debug_selector(|| "frame-notice".to_owned())
                .flex_none()
                .px_3()
                .py_1()
                .text_sm()
                .text_color(danger)
                .border_b_1()
                .border_color(border)
                .child(notice)
        });
        let turn_notice = self.turn_notice(cx).map(|notice| {
            div()
                .debug_selector(|| "frame-turn-notice".to_owned())
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .text_sm()
                .border_b_1()
                .border_color(border)
                .child(div().flex_1().min_w_0().truncate().child(notice.text))
                .child(
                    div()
                        .id("frame-turn-notice-show")
                        .debug_selector(|| "frame-turn-notice-show".to_owned())
                        .flex_none()
                        .px_2()
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .cursor_pointer()
                        .on_click(cx.listener(|frame, _, window, cx| {
                            frame.show_finished_turn(&ShowFinishedTurn, window, cx);
                        }))
                        .child("Show"),
                )
        });
        let base_field = composed.base_field.as_ref().map(|field| {
            div()
                .debug_selector(|| "frame-base-field".to_owned())
                .flex_none()
                .p_2()
                .border_b_1()
                .border_color(border)
                // The field lets Escape pass once it has nothing of its own to close.
                .on_action(cx.listener(|frame, _: &Escape, window, cx| {
                    frame.close_base_field(window, cx);
                }))
                .child(Input::new(&field.input))
        });
        let worktree_field = composed.worktree_field.as_ref().map(|field| {
            div()
                .debug_selector(|| "frame-worktree-field".to_owned())
                .flex_none()
                .p_2()
                .border_b_1()
                .border_color(border)
                .on_action(cx.listener(|frame, _: &Escape, window, cx| {
                    frame.close_worktree_field(window, cx);
                }))
                .child(Input::new(&field.input))
        });
        let connected = matches!(self.link.read(cx).status(), LinkStatus::Connected(_));
        // The settings screen takes the place of the panes.
        let panes = if let Some(open) = &composed.settings {
            // The element that holds the screen tracks the screen's focus handle: the
            // focus the Frame gave the screen at its opening is then in what is drawn,
            // whatever the screen draws, and a click on the screen takes it.
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .track_focus(open.screen.read(cx).focus())
                .child(open.screen.clone())
        } else if composed.panes.is_empty() {
            self.empty(cx).flex_1().min_h_0()
        } else {
            self.laid_out(composed)
        };
        // With panes in the window, how it stands with the Keeper is worth a line only
        // while the panes can show nothing for want of one.
        let status = (!connected && !composed.panes.is_empty()).then(|| {
            div()
                .flex_none()
                .px_3()
                .py_1()
                .text_sm()
                .text_color(muted)
                .border_t_1()
                .border_color(border)
                .child(self.status_line(cx))
        });

        let root = composed.models.routes.apply(root);
        root.flex()
            .flex_row()
            .track_focus(&composed.focus)
            // An element that tracks a focus takes it at any press inside it that
            // nothing further in took. Here that would take the focus from a pane at
            // every click in the sidebar or on the notice, and the keys of the pane
            // would be dead until a click into it. The Frame's own place for the focus
            // is only for when there is no other, so a press never moves the focus
            // there: it stays where it was.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::set_base))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::close_settings))
            .on_action(cx.listener(Self::new_worktree))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::unpin_sidebar))
            .on_action(cx.listener(Self::select_next_workspace))
            .on_action(cx.listener(Self::select_previous_workspace))
            .on_action(cx.listener(Self::focus_next_pane))
            .on_action(cx.listener(Self::focus_previous_pane))
            .on_action(cx.listener(Self::move_panes))
            .on_action(cx.listener(Self::show_finished_turn))
            .on_action(cx.listener(Self::quit_and_stop_agents))
            .child(composed.sidebar.clone())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .children(notice)
                    .children(turn_notice)
                    .children(self.keeper_note_line(cx))
                    .children(base_field)
                    .children(worktree_field)
                    .child(panes)
                    .children(status),
            )
    }
}

/// What [`open`] gives back.
pub struct Opened {
    /// The window.
    pub window: AnyWindowHandle,
    /// Its root view.
    pub frame: Entity<Frame>,
    /// The shared models of the window.
    pub models: Models,
}

/// Opens the app's window with these panes and links it to the Keeper on `socket`,
/// starting the Keeper when none answers.
pub fn open(
    socket: PathBuf,
    app_version: &'static str,
    panes: Vec<BuildPane>,
    cx: &mut App,
) -> Result<Opened> {
    let link = cx.new(|_| KeeperLink::new());
    let models = Models::new(link.clone(), cx);
    // The system's setting is what the application reports for its windows, told to the
    // model before the window exists, so that the first frame, which the toolkit draws
    // inside its opening of the window, is in YAAR's definition of the system's mode and
    // not in the toolkit's own colours of it. Until a theme the user named arrives from the
    // Keeper, the window follows the system either way.
    let system = Appearance::from(cx.window_appearance());
    models
        .looks
        .update(cx, |looks, cx| looks.system_is(system, cx));
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // What a Linux desktop matches the desktop entry and the icon by.
        app_id: Some("io.github.tsadoq.yaar".into()),
        titlebar: Some(TitlebarOptions {
            title: Some("YAAR".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (window, frame) = gpui_kit::open_window(options, cx, {
        let models = models.clone();
        move |window, cx| cx.new(|cx| Frame::with_panes(models, panes, window, cx))
    })?;

    // Reaching the Keeper may mean starting a process: off the thread that draws.
    let attaching = cx
        .background_executor()
        .spawn(async move { keeper::attach(&socket, app_version).map_err(|e| e.to_string()) });
    let weak = link.downgrade();
    cx.spawn(async move |cx| {
        let outcome = attaching.await;
        trace::mark("keeper_attached");
        weak.update(cx, |link, cx| link.attached(outcome, cx)).ok();
    })
    .detach();

    Ok(Opened {
        window,
        frame,
        models,
    })
}
