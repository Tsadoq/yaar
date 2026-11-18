//! The root view of the window.

use std::path::PathBuf;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;

use crate::controls::Control;
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
///   [`crate::WorkspaceList::set_base`], Escape closes it;
/// - [`crate::ToggleSidebar`] and [`crate::UnpinSidebar`] pin and unpin the sidebar;
/// - [`crate::SelectNextWorkspace`] and [`crate::SelectPreviousWorkspace`] change the
///   workspace on screen.
///
/// It shows the reason when the Keeper refuses what the user asked. Whenever the window
/// comes to the front it asks for the list of workspaces and for the changes again,
/// by [`crate::WorkspaceList::refresh`] and [`crate::WorkspaceChanges::refresh`].
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
        todo!()
    }

    /// The sidebar; none in a Frame made by [`Frame::new`].
    pub fn sidebar(&self) -> Option<&Entity<Sidebar>> {
        todo!()
    }

    /// The names of the panes, in the order they are laid out.
    pub fn panes(&self) -> Vec<&'static str> {
        todo!()
    }

    /// Why the Keeper refused the last thing the user asked for, as the sentence the
    /// Frame shows; none when nothing is shown. The Frame draws this very sentence.
    pub fn notice(&self, cx: &App) -> Option<String> {
        todo!()
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
}

impl Render for Frame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_marked {
            self.first_frame_marked = true;
            window.on_next_frame(|_, _| trace::mark("first_frame"));
        }
        let status = self.status_line(cx);
        let theme = cx.theme();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(div().text_xl().child("YAAR"))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(status),
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
