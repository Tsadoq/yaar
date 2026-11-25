use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use yaar_link::{Age, Fact, Head, Hello, SessionId, Setting, TerminalPlace, WorkspaceId};
use yaar_ui::metrics::{GUTTER, SPACE_6, cursor_col_resize, cursor_row_resize};
use yaar_ui::pieces::{Banner, Button, ButtonKind, EmptyState, Field, Pane as UiPane, Toast, Tone};
use yaar_ui::themes::{Appearance, palette};
use yaar_ui::typography::{TypeRole, Typed};

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
use crate::trace;

/// Height of the transparent macOS title-bar drag strip; zero where the system draws the frame.
pub const TITLEBAR_HEIGHT: Pixels = if cfg!(target_os = "macos") {
    px(32.)
} else {
    px(0.)
};

/// Space above the pane cards: the transparent title bar on macOS, one gutter on other systems.
pub const TOP_INSET: Pixels = if cfg!(target_os = "macos") {
    TITLEBAR_HEIGHT
} else {
    GUTTER
};

pub struct Pane {
    pub name: &'static str,
    pub view: AnyView,
    pub focus: FocusHandle,
}

pub type BuildPane = Box<dyn FnOnce(&Models, &mut Window, &mut App) -> Pane>;

/// How long quitting with the stop waits for the stop frame to leave before it quits anyway:
/// a Keeper that takes nothing must not hold the window open.
const STOP_WAIT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnNotice {
    pub workspace: yaar_link::WorkspaceId,
    pub session: yaar_link::SessionId,
    pub text: String,
}

/// One pane as its crate hands it to the Window program: a pane cannot be in the window without its
/// keys.
#[derive(Debug, Clone, Copy)]
pub struct PaneEntry {
    pub name: &'static str,
    pub controls: fn() -> Vec<Control>,
    pub build: fn() -> BuildPane,
}

/// Where the panes after the first lie; kept by the Keeper as `SettingList::terminal_place`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Beside {
    #[default]
    Right,
    Below,
    Left,
}

impl Beside {
    pub fn next(self) -> Beside {
        match self {
            Beside::Right => Beside::Below,
            Beside::Below => Beside::Left,
            Beside::Left => Beside::Right,
        }
    }

    fn of(place: TerminalPlace) -> Beside {
        match place {
            TerminalPlace::Right => Beside::Right,
            TerminalPlace::Below => Beside::Below,
            TerminalPlace::Left => Beside::Left,
        }
    }

    fn as_setting(self) -> TerminalPlace {
        match self {
            Beside::Right => TerminalPlace::Right,
            Beside::Below => TerminalPlace::Below,
            Beside::Left => TerminalPlace::Left,
        }
    }
}

/// The root view of the window: the sidebar, the panes, and a line about the Keeper while there is
/// nothing else to show. Handles the actions of [`crate::controls`] except [`crate::Quit`].
pub struct Frame {
    link: Entity<KeeperLink>,
    first_frame_marked: bool,
    #[cfg(target_os = "macos")]
    titlebar_drag: bool,
    composed: Option<Composed>,
}

struct PaneSlot {
    pane: Pane,
    slot_focus: FocusHandle,
    _focus_subscriptions: Vec<Subscription>,
}

struct Composed {
    models: Models,
    sidebar: Entity<Sidebar>,
    panes: Vec<PaneSlot>,
    /// Where the focus is while it is in no pane and no field: an action reaches the Frame only
    /// from a place inside what it draws.
    focus: FocusHandle,
    base_field: Option<LineField>,
    worktree_field: Option<LineField>,
    settings: Option<OpenScreen>,
    beside: Beside,
    split_ratio: Option<f32>,
    dragging_split: bool,
    panes_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    worktree_notice: Option<String>,
    no_dialog: Option<String>,
    /// The last turn that ended in a workspace not on screen, until that workspace comes on screen
    /// or the user dismisses its toast.
    finished_elsewhere: Option<(WorkspaceId, SessionId)>,
    showing_welcome: bool,
}

struct LineField {
    input: Entity<InputState>,
    opened_from: Option<FocusHandle>,
    _enter: Subscription,
}

struct OpenScreen {
    screen: Entity<SettingsScreen>,
    opened_from: Option<FocusHandle>,
}

impl Frame {
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        cx.observe(&link, |_, _, cx| cx.notify()).detach();
        Self {
            link,
            first_frame_marked: false,
            #[cfg(target_os = "macos")]
            titlebar_drag: false,
            composed: None,
        }
    }

    pub fn with_panes(
        models: Models,
        panes: Vec<BuildPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let link = models.link.clone();
        cx.observe(&link, |_, _, cx| cx.notify()).detach();
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
        cx.observe(&models.agents, |_, _, cx| cx.notify()).detach();
        cx.observe(&models.settings, |frame, settings, cx| {
            if let Some(composed) = &mut frame.composed {
                let place = Beside::of(settings.read(cx).terminal_place());
                if composed.beside != place {
                    let was_below = composed.beside == Beside::Below;
                    let now_below = place == Beside::Below;
                    if was_below != now_below {
                        composed.split_ratio = None;
                    }
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

        // Files change, branches are switched and worktrees are made while the user is in another
        // app, and no fact tells of it.
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
        // What had the focus may leave the window, as a pane's row does when its file is no longer
        // changed: the focus then goes to the Frame's own place.
        cx.on_focus_lost(window, |frame, window, cx| {
            if let Some(composed) = &frame.composed {
                composed.focus.focus(window, cx);
            }
        })
        .detach();

        let panes: Vec<PaneSlot> = panes
            .into_iter()
            .map(|build| {
                let pane = build(&models, window, cx);
                let slot_focus = cx.focus_handle();
                let target = pane.focus.clone();
                let _focus_subscriptions = vec![
                    cx.on_focus_in(&slot_focus, window, |_, _, cx| cx.notify()),
                    cx.on_focus_out(&slot_focus, window, |_, _, _, cx| cx.notify()),
                    cx.on_focus(&slot_focus, window, move |_, window, cx| {
                        target.focus(window, cx);
                    }),
                ];
                PaneSlot {
                    pane,
                    slot_focus,
                    _focus_subscriptions,
                }
            })
            .collect();
        match panes.first() {
            Some(first) => first.pane.focus.focus(window, cx),
            None => focus.focus(window, cx),
        }

        let beside = Beside::of(models.settings.read(cx).terminal_place());
        Self {
            link,
            first_frame_marked: false,
            #[cfg(target_os = "macos")]
            titlebar_drag: false,
            composed: Some(Composed {
                models,
                sidebar,
                panes,
                focus,
                base_field: None,
                worktree_field: None,
                settings: None,
                beside,
                split_ratio: None,
                dragging_split: false,
                panes_bounds: Rc::new(Cell::new(None)),
                worktree_notice: None,
                no_dialog: None,
                finished_elsewhere: None,
                showing_welcome: false,
            }),
        }
    }

    pub fn sidebar(&self) -> Option<&Entity<Sidebar>> {
        self.composed.as_ref().map(|composed| &composed.sidebar)
    }

    pub fn beside(&self) -> Beside {
        self.composed
            .as_ref()
            .map(|composed| composed.beside)
            .unwrap_or_default()
    }

    pub fn settings_screen(&self) -> Option<&Entity<SettingsScreen>> {
        self.composed
            .as_ref()?
            .settings
            .as_ref()
            .map(|open| &open.screen)
    }

    /// The note about a Keeper of an older version than this window; none for a Keeper of the same
    /// or a newer version, or while no Keeper is reached.
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

    /// A turn that ended in a workspace not on screen; of two such turns the later.
    pub fn turn_notice(&self, cx: &App) -> Option<TurnNotice> {
        let composed = self.composed.as_ref()?;
        let (workspace, session) = composed.finished_elsewhere.as_ref()?;
        let list = composed.models.workspaces.read(cx);
        if list.selected() == Some(workspace) {
            return None;
        }
        let name = workspace_name(list.workspace(workspace)?);
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

    fn dismiss_turn_notice(&mut self, cx: &mut Context<Self>) {
        if let Some(composed) = &mut self.composed
            && composed.finished_elsewhere.take().is_some()
        {
            cx.notify();
        }
    }

    pub fn panes(&self) -> Vec<&'static str> {
        self.composed
            .iter()
            .flat_map(|composed| &composed.panes)
            .map(|slot| slot.pane.name)
            .collect()
    }

    /// Why the last thing the user asked for was not done; the next ask clears it.
    pub fn notice(&self, cx: &App) -> Option<String> {
        let composed = self.composed.as_ref()?;
        composed
            .no_dialog
            .clone()
            .or_else(|| composed.worktree_notice.clone())
            .or_else(|| {
                let refusal = composed.models.workspaces.read(cx).refusal()?;
                Some(refusal.to_string())
            })
    }

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

    fn shows_welcome(composed: &Composed, cx: &App) -> bool {
        composed.settings.is_none() && !composed.panes.is_empty() && {
            let list = composed.models.workspaces.read(cx);
            list.loaded() && list.repositories().is_empty()
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
                        Ok(Ok(None)) | Err(_) => {}
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

    /// A field with nothing in it stays open: there is nothing to ask the Keeper.
    fn take_base(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        let Some(field) = &composed.base_field else {
            return;
        };
        let base = field.input.read(cx).value().trim().to_owned();
        if base.is_empty() {
            return;
        }
        // A new ask clears the older notices.
        if composed.no_dialog.take().is_some() | composed.worktree_notice.take().is_some() {
            cx.notify();
        }
        let workspaces = composed.models.workspaces.clone();
        self.close_base_field(window, cx);
        workspaces.update(cx, |list, cx| list.set_base(base, cx));
    }

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

    fn new_worktree(&mut self, _: &NewWorktree, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        if let Some(open) = &composed.worktree_field {
            open.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        composed.worktree_notice = None;
        composed.no_dialog = None;
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

    /// Enter in the field for a new worktree: closes the field and wants the worktree from what is
    /// on screen now, which may have changed since the field opened; then the notice says why
    /// nothing is wanted. An empty field stays open.
    fn take_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &self.composed else {
            return;
        };
        let Some(field) = &composed.worktree_field else {
            return;
        };
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
                composed
                    .models
                    .workspaces
                    // The worktree is wanted of the terminals, not of the workspace list, so the
                    // list's older refusal is cleared here, as an ask of the list would have
                    // cleared it.
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

    /// A pane given the focus that drew no place for it would leave the Frame's keys dead until the
    /// first click.
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

    fn focus_next_pane(&mut self, _: &FocusNextPane, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus_between_panes(true, window, cx);
    }

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
        // The panes are not drawn while settings or the welcome screen are open.
        if count == 0 || composed.settings.is_some() || Self::shows_welcome(composed, cx) {
            return;
        }
        let holding = composed
            .panes
            .iter()
            .position(|slot| slot.slot_focus.contains_focused(window, cx));
        let to = match (holding, forward) {
            (Some(at), true) => (at + 1) % count,
            (Some(at), false) => (at + count - 1) % count,
            (None, true) => 0,
            (None, false) => count - 1,
        };
        composed.panes[to].pane.focus.focus(window, cx);
    }

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

    /// The stop frame goes first and the Window waits a moment for it: quitting at once could take
    /// it down.
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

    fn move_panes(&mut self, _: &MovePanes, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(composed) = &self.composed {
            let settings = composed.models.settings.clone();
            let next = Beside::of(settings.read(cx).terminal_place()).next();
            settings.update(cx, |settings, cx| {
                settings.set(Setting::TerminalPlace(next.as_setting()), cx);
            });
        }
    }

    #[cfg(target_os = "macos")]
    fn titlebar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        Some(
            div()
                .id("frame-titlebar")
                .debug_selector(|| "frame-titlebar".to_owned())
                .flex_none()
                .w_full()
                .h(TITLEBAR_HEIGHT)
                .bg(palette(cx).window)
                .window_control_area(WindowControlArea::Drag)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|frame, event: &MouseDownEvent, window, _| {
                        if event.click_count >= 2 {
                            frame.titlebar_drag = false;
                            window.titlebar_double_click();
                        } else {
                            frame.titlebar_drag = true;
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|frame, _: &MouseUpEvent, _, _| {
                        frame.titlebar_drag = false;
                    }),
                )
                .on_mouse_move(cx.listener(|frame, _: &MouseMoveEvent, window, _| {
                    if frame.titlebar_drag {
                        frame.titlebar_drag = false;
                        window.start_window_move();
                    }
                }))
                .into_any_element(),
        )
    }

    #[cfg(not(target_os = "macos"))]
    fn titlebar(&self, _: &Context<Self>) -> Option<AnyElement> {
        None
    }

    fn keeper_note_line(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let note = self.keeper_note(cx)?;
        let offer = Button::new("frame-keeper-note-stop", "Quit and stop agents")
            .kind(ButtonKind::Secondary)
            .small()
            .selector("frame-keeper-note-stop")
            .on_click(cx.listener(|frame, _, _, cx| frame.stop_everything_and_quit(cx)));
        Some(
            div()
                .flex_none()
                .child(
                    Banner::new(note)
                        .tone(Tone::Warn)
                        .card()
                        .selector("frame-keeper-note")
                        .action(offer),
                )
                .into_any_element(),
        )
    }

    fn prompt_field(
        &self,
        selector: &'static str,
        cancel_id: &'static str,
        input: &Entity<InputState>,
        on_close: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> AnyElement {
        let palette = palette(cx);
        let footer = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .typed(TypeRole::Small)
                    .text_color(palette.text_dim)
                    .child("Enter to confirm, Escape to cancel"),
            )
            .child(
                Button::new(cancel_id, "Cancel")
                    .kind(ButtonKind::Ghost)
                    .small()
                    .on_click(cx.listener(move |frame, _, window, cx| on_close(frame, window, cx))),
            );
        div()
            .debug_selector(move || selector.to_owned())
            .flex_none()
            // The field lets Escape pass once it has nothing of its own to close.
            .on_action(cx.listener(move |frame, _: &Escape, window, cx| {
                on_close(frame, window, cx);
            }))
            .child(Field::input(input).footer(footer))
            .into_any_element()
    }

    fn laid_out(&self, composed: &Composed, window: &Window, cx: &Context<Self>) -> Div {
        let palette = palette(cx);
        let pane_box = |slot: &PaneSlot| {
            let focused = slot.slot_focus.contains_focused(window, cx);
            div()
                .track_focus(&slot.slot_focus)
                .flex()
                .flex_col()
                .min_w_0()
                .min_h_0()
                .child(
                    UiPane::new()
                        .selector(format!("frame-pane-{}", slot.pane.name))
                        .focused(focused)
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(slot.pane.view.clone()),
                )
        };
        let (first_slot, others) = composed.panes.split_first().expect("a pane to lay out");
        if others.is_empty() {
            return div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .child(pane_box(first_slot).flex_1());
        }
        let below = composed.beside == Beside::Below;
        let (first_weight, rest_weight) = match composed.split_ratio {
            Some(ratio) => (ratio, 1.0 - ratio),
            None if below => (68., 32.),
            None => (62., 38.),
        };
        let mut first = pane_box(first_slot).flex_basis(px(0.)).flex_shrink(1.);
        first.style().flex_grow = Some(first_weight);

        let mut rest = if below {
            div().flex_row()
        } else {
            div().flex_col()
        }
        .flex()
        .gap(GUTTER)
        .flex_basis(px(0.))
        .flex_shrink(1.)
        .min_w_0()
        .min_h_0()
        .children(others.iter().map(|slot| pane_box(slot).flex_1()));
        rest.style().flex_grow = Some(rest_weight);

        let dragging = composed.dragging_split;
        let grip = if below {
            div().h(px(2.)).w(px(32.))
        } else {
            div().w(px(2.)).h(px(32.))
        }
        .when(dragging, |el| el.bg(palette.focus))
        .when(!dragging, |el| {
            el.group_hover("frame-split-handle", move |style| style.bg(palette.edge))
        });

        let handle = div()
            .id("frame-split-handle")
            .debug_selector(|| "frame-split-handle".to_owned())
            .group("frame-split-handle")
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .map(|el| {
                if below {
                    cursor_row_resize(el.h(GUTTER).w_full())
                } else {
                    cursor_col_resize(el.w(GUTTER).h_full())
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|frame, event: &MouseDownEvent, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    let Some(composed) = &mut frame.composed else {
                        return;
                    };
                    if event.click_count >= 2 {
                        composed.dragging_split = false;
                        composed.split_ratio = None;
                    } else {
                        composed.dragging_split = true;
                    }
                    cx.notify();
                }),
            )
            .child(grip);

        let measured = composed.panes_bounds.clone();
        let measure = canvas(
            |bounds, _, _| bounds,
            move |_, bounds, _, _| {
                measured.set(Some(bounds));
            },
        )
        .absolute()
        .size_full();

        let area = div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .child(measure);
        match composed.beside {
            Beside::Right => area.flex_row().child(first).child(handle).child(rest),
            Beside::Left => area.flex_row().child(rest).child(handle).child(first),
            Beside::Below => area.flex_col().child(first).child(handle).child(rest),
        }
    }

    fn welcome(&self, cx: &Context<Self>) -> AnyElement {
        let open = Button::new("frame-open-folder", "Open Folder…")
            .kind(ButtonKind::Primary)
            .selector("frame-open-folder")
            .on_click(cx.listener(|frame, _, window, cx| {
                frame.open_folder(&OpenFolder, window, cx);
            }));
        UiPane::new()
            .selector("frame-welcome")
            .flex_1()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                EmptyState::new("Open a repository")
                    .display()
                    .line("Pick a git repository or worktree to review diffs and run agents beside your code.")
                    .action(open),
            )
            .into_any_element()
    }

    fn empty(&self, cx: &Context<Self>) -> AnyElement {
        let note = self
            .composed
            .is_none()
            .then(|| self.keeper_note_line(cx))
            .flatten();
        UiPane::new()
            .flex_1()
            .size_full()
            .items_center()
            .justify_center()
            .gap(GUTTER)
            .child(EmptyState::new("YAAR").display().line(self.status_line(cx)))
            .children(note)
            .into_any_element()
    }
}

impl Render for Frame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_marked {
            self.first_frame_marked = true;
            window.on_next_frame(|_, _| trace::mark("first_frame"));
            cx.defer_in(window, Self::keep_the_focus_in_what_is_drawn);
        }
        let palette = palette(cx);
        let root = div()
            .relative()
            .size_full()
            // A key of the app that is no Cmd key is bound in this context, so it has to be set by
            // the outermost element to hold wherever the focus is.
            .key_context(KEY_CONTEXT)
            .bg(palette.window)
            .text_color(palette.text)
            .flex()
            .flex_col();

        if let Some(composed) = &mut self.composed {
            let now_welcome = Self::shows_welcome(composed, cx);
            if composed.showing_welcome && !now_welcome && composed.settings.is_none() {
                cx.defer_in(window, |frame, window, cx| {
                    if let Some(composed) = &frame.composed
                        && composed.settings.is_none()
                        && let Some(first) = composed.panes.first()
                    {
                        first.pane.focus.focus(window, cx);
                    }
                });
            }
            composed.showing_welcome = now_welcome;
        }

        let Some(composed) = &self.composed else {
            return root.children(self.titlebar(cx)).child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .px(GUTTER)
                    .pb(GUTTER)
                    .when(!cfg!(target_os = "macos"), |el| el.pt(GUTTER))
                    .child(self.empty(cx)),
            );
        };

        let notice = self.notice(cx).map(|notice| {
            div().flex_none().child(
                Banner::new(notice)
                    .tone(Tone::Danger)
                    .card()
                    .selector("frame-notice"),
            )
        });
        let turn_notice = self.turn_notice(cx).map(|notice| {
            let entity = cx.entity().downgrade();
            div().absolute().bottom(SPACE_6).right(SPACE_6).child(
                Toast::new("frame-turn-notice", notice.text)
                    .selector("frame-turn-notice")
                    .action(
                        Button::new("frame-turn-notice-show", "Show")
                            .kind(ButtonKind::Primary)
                            .small()
                            .selector("frame-turn-notice-show")
                            .on_click(cx.listener(|frame, _, window, cx| {
                                frame.show_finished_turn(&ShowFinishedTurn, window, cx);
                            })),
                    )
                    .on_dismiss(move |_, cx| {
                        entity
                            .update(cx, |frame, cx| frame.dismiss_turn_notice(cx))
                            .ok();
                    }),
            )
        });
        let base_field = composed.base_field.as_ref().map(|field| {
            self.prompt_field(
                "frame-base-field",
                "frame-base-cancel",
                &field.input,
                Self::close_base_field,
                cx,
            )
        });
        let worktree_field = composed.worktree_field.as_ref().map(|field| {
            self.prompt_field(
                "frame-worktree-field",
                "frame-worktree-cancel",
                &field.input,
                Self::close_worktree_field,
                cx,
            )
        });
        let connected = matches!(self.link.read(cx).status(), LinkStatus::Connected(_));
        let panes: AnyElement = if let Some(open) = &composed.settings {
            div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .track_focus(open.screen.read(cx).focus())
                .child(open.screen.clone())
                .into_any_element()
        } else if composed.panes.is_empty() {
            self.empty(cx)
        } else if composed.showing_welcome {
            self.welcome(cx)
        } else {
            self.laid_out(composed, window, cx).into_any_element()
        };
        let status = (!connected && !composed.panes.is_empty()).then(|| {
            div()
                .flex_none()
                .child(Banner::new(self.status_line(cx)).tone(Tone::Plain).card())
        });

        let root = composed.models.routes.apply(root);
        root.track_focus(&composed.focus)
            // An element that tracks a focus takes it at any press nothing further in took, which
            // would pull the focus from a pane at every click in the sidebar or on the notice. The
            // press never moves it.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .on_mouse_move(cx.listener(|frame, event: &MouseMoveEvent, _, cx| {
                let Some(composed) = &mut frame.composed else {
                    return;
                };
                if !composed.dragging_split {
                    return;
                }
                if !event.dragging() {
                    composed.dragging_split = false;
                    cx.notify();
                    return;
                }
                let Some(bounds) = composed.panes_bounds.get() else {
                    return;
                };
                let ratio = match composed.beside {
                    Beside::Right => {
                        let span = f32::from(bounds.size.width - GUTTER);
                        if span <= 0. {
                            return;
                        }
                        (f32::from(event.position.x - bounds.origin.x) - f32::from(GUTTER) / 2.)
                            / span
                    }
                    Beside::Left => {
                        let span = f32::from(bounds.size.width - GUTTER);
                        if span <= 0. {
                            return;
                        }
                        1.0 - (f32::from(event.position.x - bounds.origin.x)
                            - f32::from(GUTTER) / 2.)
                            / span
                    }
                    Beside::Below => {
                        let span = f32::from(bounds.size.height - GUTTER);
                        if span <= 0. {
                            return;
                        }
                        (f32::from(event.position.y - bounds.origin.y) - f32::from(GUTTER) / 2.)
                            / span
                    }
                };
                composed.split_ratio = Some(ratio.clamp(0.25, 0.80));
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|frame, _: &MouseUpEvent, _, cx| {
                    if let Some(composed) = &mut frame.composed
                        && composed.dragging_split
                    {
                        composed.dragging_split = false;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|frame, _: &MouseUpEvent, _, cx| {
                    if let Some(composed) = &mut frame.composed
                        && composed.dragging_split
                    {
                        composed.dragging_split = false;
                        cx.notify();
                    }
                }),
            )
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
            .children(self.titlebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_row()
                    .gap(GUTTER)
                    .pr(GUTTER)
                    .pb(GUTTER)
                    .when(!cfg!(target_os = "macos"), |el| el.pt(GUTTER))
                    .child(composed.sidebar.clone())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .gap(GUTTER)
                            .children(notice)
                            .children(self.keeper_note_line(cx))
                            .children(base_field)
                            .children(worktree_field)
                            .child(panes)
                            .children(status),
                    ),
            )
            .children(turn_notice)
    }
}

pub struct Opened {
    pub window: AnyWindowHandle,
    pub frame: Entity<Frame>,
    pub models: Models,
}

pub fn open(
    socket: PathBuf,
    app_version: &'static str,
    panes: Vec<BuildPane>,
    cx: &mut App,
) -> Result<Opened> {
    let link = cx.new(|_| KeeperLink::new());
    let models = Models::new(link.clone(), cx);
    // The system's setting is told to the model before the window exists, so that the first frame,
    // which the toolkit draws inside its opening of the window, is in YAAR's colours and not the
    // toolkit's.
    let system = Appearance::from(cx.window_appearance());
    models
        .looks
        .update(cx, |looks, cx| looks.system_is(system, cx));
    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let titlebar = if cfg!(target_os = "macos") {
        TitlebarOptions {
            title: Some("YAAR".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.), px(10.))),
        }
    } else {
        TitlebarOptions {
            title: Some("YAAR".into()),
            ..Default::default()
        }
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // What a Linux desktop matches the desktop entry and the icon by.
        app_id: Some("io.github.tsadoq.yaar".into()),
        titlebar: Some(titlebar),
        app_owns_titlebar_drag: cfg!(target_os = "macos"),
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
