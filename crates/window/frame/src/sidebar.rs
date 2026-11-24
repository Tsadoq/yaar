use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use yaar_link::{RepositoryId, SessionState, Workspace, WorkspaceId};

use crate::agents::AgentList;
use crate::models::WorkspaceList;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarForm {
    Full,
    Rail,
}

/// The width of the window below which the sidebar is a rail while the user has pinned neither
/// form.
pub const RAIL_BELOW: Pixels = px(1000.);

pub const FULL_WIDTH: Pixels = px(240.);

pub const RAIL_WIDTH: Pixels = px(48.);

pub fn sidebar_form(window_width: Pixels, pinned: Option<SidebarForm>) -> SidebarForm {
    pinned.unwrap_or(if window_width < RAIL_BELOW {
        SidebarForm::Rail
    } else {
        SidebarForm::Full
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarRow {
    Repository {
        id: RepositoryId,
        name: String,
    },
    Workspace {
        id: WorkspaceId,
        /// The name it is listed by: its branch, or the commit it sits on when no branch is checked
        /// out; for a worktree that is gone, the name of its folder.
        name: String,
        selected: bool,
        gone: bool,
        agent: Option<SessionState>,
    },
}

/// Elements carry `sidebar-...` selectors for `debug_bounds`, since tests cannot read what is
/// drawn.
pub struct Sidebar {
    workspaces: Entity<WorkspaceList>,
    agents: Option<Entity<AgentList>>,
    pinned: Option<SidebarForm>,
    form: SidebarForm,
    window_width: Option<Pixels>,
}

impl Sidebar {
    /// It follows the width of a window only inside a [`crate::Frame`] made by
    /// [`crate::Frame::with_panes`], which tells the sidebar; until told a width, the form is the
    /// full list.
    pub fn new(workspaces: Entity<WorkspaceList>, cx: &mut Context<Self>) -> Self {
        cx.observe(&workspaces, |_, _, cx| cx.notify()).detach();
        Self {
            workspaces,
            agents: None,
            pinned: None,
            form: SidebarForm::Full,
            window_width: None,
        }
    }

    pub fn with_agents(
        workspaces: Entity<WorkspaceList>,
        agents: Entity<AgentList>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&agents, |_, _, cx| cx.notify()).detach();
        let mut sidebar = Self::new(workspaces, cx);
        sidebar.agents = Some(agents);
        sidebar
    }

    pub fn form(&self) -> SidebarForm {
        self.form
    }

    pub fn pinned(&self) -> Option<SidebarForm> {
        self.pinned
    }

    pub fn pin(&mut self, form: Option<SidebarForm>, cx: &mut Context<Self>) {
        if self.pinned != form {
            self.pinned = form;
            cx.notify();
        }
        self.settle(cx);
    }

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
                let name = workspace_name(workspace);
                rows.push(SidebarRow::Workspace {
                    agent: self
                        .agents
                        .as_ref()
                        .and_then(|agents| agents.read(cx).shown(&workspace.id)),
                    id: workspace.id.clone(),
                    name,
                    selected: on_screen == Some(&workspace.id),
                    gone: workspace.head.is_none(),
                });
            }
        }
        rows
    }

    pub(crate) fn pin_other_form(&mut self, cx: &mut Context<Self>) {
        let other = match self.form {
            SidebarForm::Full => SidebarForm::Rail,
            SidebarForm::Rail => SidebarForm::Full,
        };
        self.pin(Some(other), cx);
    }

    pub(crate) fn set_window_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.window_width = Some(width);
        self.settle(cx);
    }

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

    fn row(&self, row: SidebarRow, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let rail = self.form == SidebarForm::Rail;
        match row {
            SidebarRow::Repository { id, name } => {
                let row = div()
                    .debug_selector(move || format!("sidebar-repository-{id}"))
                    .flex_none();
                if rail {
                    // In the rail a repository is the line between two groups of marks.
                    row.mx_2()
                        .my_1()
                        .h(px(1.))
                        .bg(colors.border)
                        .into_any_element()
                } else {
                    row.px_3()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .text_color(colors.muted)
                        .truncate()
                        .child(name)
                        .into_any_element()
                }
            }
            SidebarRow::Workspace {
                id,
                name,
                selected,
                gone,
                agent,
            } => {
                let selector = format!("sidebar-workspace-{id}");
                let agent_mark = agent.and_then(|state| agent_mark(&id, state, rail, colors));
                // After the name, so that a reader sees which row it is and then that it is
                // gone; in the rail, which has room for no sentence, a cross.
                let gone_mark = gone.then(|| {
                    let selector = format!("sidebar-gone-{id}");
                    div()
                        .debug_selector(move || selector)
                        .flex_none()
                        .text_xs()
                        .text_color(colors.muted)
                        .child(if rail { "×" } else { "worktree gone" })
                });
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
                        .relative()
                        .child(mark(&name))
                        .children(gone_mark)
                        .children(agent_mark)
                        .into_any_element()
                } else {
                    row.pl_5()
                        .pr_3()
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().min_w_0().truncate().child(name))
                        .children(gone_mark)
                        .children(agent_mark)
                        .into_any_element()
                }
            }
        }
    }
}

struct Colors {
    border: Hsla,
    muted: Hsla,
    waiting: Hsla,
    unreviewed: Hsla,
    working: Hsla,
    hover: Hsla,
    selected: Hsla,
    selected_text: Hsla,
}

pub(crate) fn workspace_name(workspace: &Workspace) -> String {
    match &workspace.head {
        Some(head) => head.label(),
        None => workspace
            .path
            .file_name()
            .unwrap_or(workspace.path.as_os_str())
            .to_string_lossy()
            .into_owned(),
    }
}

fn agent_mark(
    workspace: &WorkspaceId,
    state: SessionState,
    rail: bool,
    colors: &Colors,
) -> Option<Div> {
    let (word, words, colour) = match state {
        SessionState::WaitsForPermission => ("waiting", "waiting", colors.waiting),
        SessionState::Unreviewed => ("unreviewed", "to review", colors.unreviewed),
        SessionState::Works => ("working", "working", colors.working),
        SessionState::NotRunning => ("not-running", "not running", colors.muted),
        SessionState::Idle => return None,
    };
    let selector = format!("sidebar-agent-{workspace}-{word}");
    let dot = div().flex_none().size_2().rounded_full().bg(colour);
    let mark = div()
        .debug_selector(move || selector)
        .flex_none()
        .flex()
        .items_center();
    Some(if rail {
        // Over the corner of the workspace's letter, where the rail has room for no word.
        mark.absolute().top_0().right_0().child(dot)
    } else {
        mark.gap_1()
            .text_xs()
            .text_color(colors.muted)
            .child(dot)
            .child(words)
    })
}

/// A workspace's mark in the rail: the first character, uppercased, of the last part of its name,
/// so that `feature/login` and `feature/search` do not share a mark.
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
            waiting: theme.warning,
            unreviewed: theme.info,
            working: theme.success,
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
            .when(self.pinned.is_none(), |pin| pin.text_color(colors.muted))
            .hover(|pin| pin.bg(colors.hover))
            .on_click(cx.listener(|sidebar, _, _, cx| sidebar.pin_other_form(cx)))
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
