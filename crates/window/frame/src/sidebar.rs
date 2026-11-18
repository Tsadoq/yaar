//! The sidebar: every repository with its worktrees under it, as a full list or as a rail.

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use yaar_link::{RepositoryId, WorkspaceId};

use crate::models::WorkspaceList;

/// The form the sidebar is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarForm {
    /// Every repository by its name, with its worktrees under it by their branches.
    Full,
    /// A narrow strip with one mark for each workspace.
    Rail,
}

/// The width of the window below which the sidebar is a rail while the user has pinned
/// neither form.
pub const RAIL_BELOW: Pixels = px(1000.);

/// The width of the sidebar as the full list.
pub const FULL_WIDTH: Pixels = px(240.);

/// The width of the sidebar as a rail.
pub const RAIL_WIDTH: Pixels = px(48.);

/// The form of the sidebar: the pinned one if the user pinned one, whatever the width;
/// else a rail in a window narrower than [`RAIL_BELOW`] and the full list in any other.
pub fn sidebar_form(window_width: Pixels, pinned: Option<SidebarForm>) -> SidebarForm {
    pinned.unwrap_or(if window_width < RAIL_BELOW {
        SidebarForm::Rail
    } else {
        SidebarForm::Full
    })
}

/// One row of the sidebar. The rail holds the same rows as the full list, drawn
/// narrower.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarRow {
    /// A repository; its worktrees are the rows that follow, up to the next repository.
    Repository {
        /// Its id.
        id: RepositoryId,
        /// Its name.
        name: String,
    },
    /// A worktree of the repository above.
    Workspace {
        /// Its id.
        id: WorkspaceId,
        /// The name it is listed by: its branch, or the commit it sits on when no branch
        /// is checked out.
        name: String,
        /// Whether it is the workspace on screen.
        selected: bool,
    },
}

/// The sidebar of the window: a view, and the model of what it shows.
///
/// A click on a workspace puts it on screen. A click on the pin control does what
/// [`crate::ToggleSidebar`] does.
///
/// For tests, which cannot read what is drawn, three kinds of element carry a selector
/// that `debug_bounds` finds: the sidebar as a whole, `sidebar`; each workspace row,
/// `sidebar-workspace-<id>`; and the pin control, `sidebar-pin`.
pub struct Sidebar {
    workspaces: Entity<WorkspaceList>,
    pinned: Option<SidebarForm>,
    form: SidebarForm,
    /// The width of the window, once a Frame told it.
    window_width: Option<Pixels>,
}

impl Sidebar {
    /// A sidebar that lists `workspaces`, with neither form pinned.
    ///
    /// By itself it knows no window. It follows the width of a window only inside a
    /// [`crate::Frame`] made by [`crate::Frame::with_panes`], which watches its window
    /// and tells the sidebar; until it is told a width, the form is the full list.
    pub fn new(workspaces: Entity<WorkspaceList>, cx: &mut Context<Self>) -> Self {
        cx.observe(&workspaces, |_, _, cx| cx.notify()).detach();
        Self {
            workspaces,
            pinned: None,
            form: SidebarForm::Full,
            window_width: None,
        }
    }

    /// The form the sidebar has now, at the width the window has now.
    pub fn form(&self) -> SidebarForm {
        self.form
    }

    /// The form the user pinned, if any.
    pub fn pinned(&self) -> Option<SidebarForm> {
        self.pinned
    }

    /// Pins a form, which the sidebar then keeps at any width; with none, the form
    /// follows the width of the window again.
    pub fn pin(&mut self, form: Option<SidebarForm>, cx: &mut Context<Self>) {
        if self.pinned != form {
            self.pinned = form;
            // The pin control shows whether a form is pinned.
            cx.notify();
        }
        self.settle(cx);
    }

    /// The rows the sidebar shows, top to bottom. These are the rows it draws from, not
    /// a second account of them.
    pub fn rows(&self, cx: &App) -> Vec<SidebarRow> {
        let list = self.workspaces.read(cx);
        let on_screen = list.selected();
        let mut rows = Vec::new();
        for repository in list.repositories() {
            rows.push(SidebarRow::Repository {
                id: repository.id.clone(),
                name: repository.name.clone(),
            });
            for workspace in &repository.workspaces {
                rows.push(SidebarRow::Workspace {
                    id: workspace.id.clone(),
                    name: workspace.head.label(),
                    selected: on_screen == Some(&workspace.id),
                });
            }
        }
        rows
    }

    /// Pins the form the sidebar does not have now: what [`crate::ToggleSidebar`] and
    /// the pin control do.
    pub(crate) fn pin_other_form(&mut self, cx: &mut Context<Self>) {
        let other = match self.form {
            SidebarForm::Full => SidebarForm::Rail,
            SidebarForm::Rail => SidebarForm::Full,
        };
        self.pin(Some(other), cx);
    }

    /// Takes the width of the window the sidebar is in: the Frame tells it when the
    /// window opens and whenever its size changes.
    pub(crate) fn set_window_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.window_width = Some(width);
        self.settle(cx);
    }

    /// Works out the form again, after the pin or the width changed.
    fn settle(&mut self, cx: &mut Context<Self>) {
        let form = match self.window_width {
            Some(width) => sidebar_form(width, self.pinned),
            None => self.pinned.unwrap_or(SidebarForm::Full),
        };
        if form != self.form {
            self.form = form;
            cx.notify();
        }
    }

    /// Draws one row, in the form the sidebar has.
    fn row(&self, row: SidebarRow, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let rail = self.form == SidebarForm::Rail;
        match row {
            // In the rail a repository is the line between two groups of marks.
            SidebarRow::Repository { .. } if rail => div()
                .mx_2()
                .my_1()
                .h(px(1.))
                .flex_none()
                .bg(colors.border)
                .into_any_element(),
            SidebarRow::Repository { name, .. } => div()
                .px_3()
                .pt_3()
                .pb_1()
                .flex_none()
                .text_xs()
                .text_color(colors.muted)
                .truncate()
                .child(name)
                .into_any_element(),
            SidebarRow::Workspace { id, name, selected } => {
                let selector = format!("sidebar-workspace-{id}");
                let row = div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .flex_none()
                    .cursor_pointer()
                    .text_sm()
                    .when(selected, |row| {
                        row.bg(colors.selected).text_color(colors.selected_text)
                    })
                    .when(!selected, |row| row.hover(|row| row.bg(colors.hover)))
                    .on_click(cx.listener(move |sidebar, _, _, cx| {
                        sidebar
                            .workspaces
                            .update(cx, |list, cx| list.select(&id, cx));
                    }));
                if rail {
                    row.mx_2()
                        .my_0p5()
                        .size_8()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .child(mark(&name))
                        .into_any_element()
                } else {
                    row.pl_5()
                        .pr_3()
                        .py_1()
                        .truncate()
                        .child(name)
                        .into_any_element()
                }
            }
        }
    }
}

/// The colours the sidebar draws with, taken from the theme once for each frame.
struct Colors {
    border: Hsla,
    muted: Hsla,
    hover: Hsla,
    selected: Hsla,
    selected_text: Hsla,
}

/// What stands for a workspace in the rail: the first character of the last part of its
/// name, so that `feature/login` and `feature/search` do not share a mark.
fn mark(name: &str) -> String {
    let last_part = name.rsplit('/').next().unwrap_or(name);
    last_part
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_default()
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rail = self.form == SidebarForm::Rail;
        let theme = cx.theme();
        let (background, foreground) = (theme.sidebar, theme.sidebar_foreground);
        let colors = Colors {
            border: theme.sidebar_border,
            muted: theme.muted_foreground,
            hover: theme.list_hover,
            selected: theme.sidebar_accent,
            selected_text: theme.sidebar_accent_foreground,
        };
        let rows: Vec<AnyElement> = self
            .rows(cx)
            .into_iter()
            .map(|row| self.row(row, &colors, cx))
            .collect();
        let pin = div()
            .id("sidebar-pin")
            .debug_selector(|| "sidebar-pin".to_owned())
            .flex_none()
            .h_8()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .border_t_1()
            .border_color(colors.border)
            // Muted while the width decides, plain once a form is pinned.
            .when(self.pinned.is_none(), |pin| pin.text_color(colors.muted))
            .hover(|pin| pin.bg(colors.hover))
            .on_click(cx.listener(|sidebar, _, _, cx| sidebar.pin_other_form(cx)))
            // The way the sidebar goes at a click: narrower from the full list, wider
            // from the rail.
            .child(if rail { "»" } else { "«" });

        div()
            .debug_selector(|| "sidebar".to_owned())
            .flex_none()
            .h_full()
            .w(if rail { RAIL_WIDTH } else { FULL_WIDTH })
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .border_r_1()
            .border_color(colors.border)
            .child(
                div()
                    .id("sidebar-rows")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(pin)
    }
}

#[cfg(test)]
mod tests {
    use super::{RAIL_BELOW, SidebarForm, sidebar_form};
    use gpui_kit::px;

    #[test]
    fn the_width_decides_the_form_only_while_nothing_is_pinned() {
        use SidebarForm::{Full, Rail};
        let just_below = RAIL_BELOW - px(0.5);
        for (width, pinned, form) in [
            (px(0.), None, Rail),
            (just_below, None, Rail),
            // The set width itself is not narrower than the set width.
            (RAIL_BELOW, None, Full),
            (px(4000.), None, Full),
            (px(0.), Some(Full), Full),
            (just_below, Some(Full), Full),
            (RAIL_BELOW, Some(Rail), Rail),
            (px(4000.), Some(Rail), Rail),
            (px(0.), Some(Rail), Rail),
            (px(4000.), Some(Full), Full),
        ] {
            assert_eq!(
                sidebar_form(width, pinned),
                form,
                "a window {width:?} wide with {pinned:?} pinned"
            );
        }
    }
}
