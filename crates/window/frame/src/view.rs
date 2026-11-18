//! The root view of the window.

use std::path::PathBuf;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;

use crate::controls::{
    Control, OpenFolder, SelectNextWorkspace, SelectPreviousWorkspace, SetBase, ToggleSidebar,
    UnpinSidebar,
};
use crate::keeper;
use crate::link_model::{KeeperLink, LinkStatus};
use crate::models::Models;
use crate::sidebar::Sidebar;
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
///   workspace on screen.
///
/// It shows the reason when the Keeper refuses what the user asked. Whenever the window
/// comes to the front it asks for the list of workspaces and for the changes again,
/// by [`crate::WorkspaceList::refresh`] and [`crate::WorkspaceChanges::refresh`].
///
/// When the window opens, the first pane has the focus, so that its keys work at once.
///
/// For tests, which cannot read what is drawn, two elements carry a selector that
/// `debug_bounds` finds while they are drawn: the notice, `frame-notice`, and the field
/// for the base, `frame-base-field`.
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
    base_field: Option<BaseField>,
    /// Why the dialog for a folder could not be shown, the last time it was asked for.
    no_dialog: Option<String>,
}

/// The one-line field the user types a base into.
struct BaseField {
    input: Entity<InputState>,
    /// Where the focus was when the field opened, and goes back to when it closes.
    opened_from: Option<FocusHandle>,
    _enter: Subscription,
}

impl Frame {
    /// A Frame that shows `link` and redraws when it changes. It has no sidebar and no
    /// pane.
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
        // The notice is the refusal the list holds.
        cx.observe(&models.workspaces, |_, _, cx| cx.notify())
            .detach();

        let sidebar = cx.new(|cx| Sidebar::new(models.workspaces.clone(), cx));
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

        Self {
            link,
            first_frame_marked: false,
            composed: Some(Composed {
                models,
                sidebar,
                panes,
                focus,
                base_field: None,
                no_dialog: None,
            }),
        }
    }

    /// The sidebar; none in a Frame made by [`Frame::new`].
    pub fn sidebar(&self) -> Option<&Entity<Sidebar>> {
        self.composed.as_ref().map(|composed| &composed.sidebar)
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
    /// shows: the reason the Keeper gave when it refused, or why the dialog for a folder
    /// could not be shown. None when nothing is shown; the user asking for the next
    /// thing takes it away. The Frame draws this very sentence.
    pub fn notice(&self, cx: &App) -> Option<String> {
        let composed = self.composed.as_ref()?;
        // A dialog that failed is the newer of the two: it is forgotten as soon as the
        // user asks for anything that could be refused.
        composed.no_dialog.clone().or_else(|| {
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
        }
    }

    fn open_folder(&mut self, _: &OpenFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(composed) = &mut self.composed else {
            return;
        };
        if composed.no_dialog.take().is_some() {
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
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("A branch or a commit as the base"));
        let enter = cx.subscribe_in(
            &input,
            window,
            |frame, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    frame.take_base(window, cx);
                }
            },
        );
        let opened_from = window.focused(cx);
        input.update(cx, |input, cx| input.focus(window, cx));
        composed.base_field = Some(BaseField {
            input,
            opened_from,
            _enter: enter,
        });
        cx.notify();
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
        if composed.no_dialog.take().is_some() {
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

    /// The name of the app and the line about the Keeper, in the middle of the room
    /// they are given: what a window shows that has nothing else to show.
    fn empty(&self, cx: &App) -> Div {
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
    }
}

impl Render for Frame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_marked {
            self.first_frame_marked = true;
            window.on_next_frame(|_, _| trace::mark("first_frame"));
        }
        let theme = cx.theme();
        let (background, foreground, muted, border, danger) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
            theme.danger,
        );
        let root = div().size_full().bg(background).text_color(foreground);
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
        let connected = matches!(self.link.read(cx).status(), LinkStatus::Connected(_));
        let panes = if composed.panes.is_empty() {
            self.empty(cx).flex_1().min_h_0()
        } else {
            // Each pane gets the same share of the room; a pane draws its own edges.
            div().flex_1().min_h_0().flex().flex_row().children(
                composed
                    .panes
                    .iter()
                    .map(|pane| div().flex_1().min_w_0().h_full().child(pane.view.clone())),
            )
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

        root.flex()
            .flex_row()
            .track_focus(&composed.focus)
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::set_base))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::unpin_sidebar))
            .on_action(cx.listener(Self::select_next_workspace))
            .on_action(cx.listener(Self::select_previous_workspace))
            .child(composed.sidebar.clone())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .children(notice)
                    .children(base_field)
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
