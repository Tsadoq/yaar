//! The root view of the window.

use std::path::PathBuf;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;

use crate::keeper;
use crate::link_model::{KeeperLink, LinkStatus};
use crate::trace;

/// The root view of the window. It holds no pane yet.
pub struct Frame {
    link: Entity<KeeperLink>,
    first_frame_marked: bool,
}

impl Frame {
    /// A Frame that shows `link` and redraws when it changes.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        cx.observe(&link, |_, _, cx| cx.notify()).detach();
        Self {
            link,
            first_frame_marked: false,
        }
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

/// Opens the app's window and links it to the Keeper on `socket`, starting the Keeper
/// when none answers. Returns the window and the model of the link.
pub fn open(
    socket: PathBuf,
    app_version: &'static str,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<KeeperLink>)> {
    let link = cx.new(|_| KeeperLink::new());
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
    let (window, _frame) = gpui_kit::open_window(options, cx, {
        let link = link.clone();
        move |_, cx| cx.new(|cx| Frame::new(link, cx))
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

    Ok((window, link))
}
