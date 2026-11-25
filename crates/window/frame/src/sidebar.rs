use gpui_kit::*;
use yaar_link::{RepositoryId, SessionState, Workspace, WorkspaceId};
use yaar_ui::metrics::{SPACE_1, SPACE_2, SPACE_3, SPACE_5};
use yaar_ui::pieces::{Button, ButtonKind, Card, Divider, ListRow, SectionLabel, State, StateDot};
use yaar_ui::themes::palette;
use yaar_ui::tokens::Palette;
use yaar_ui::typography::{TypeRole, Typed};

use crate::agents::AgentList;
use crate::controls::{NewWorktree, OpenFolder};
use crate::models::WorkspaceList;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarForm {
    Full,
    Rail,
}

/// The width of the window below which the sidebar is a rail while the user has pinned neither
/// form.
pub const RAIL_BELOW: Pixels = px(1000.);

pub const FULL_WIDTH: Pixels = px(218.);

pub const RAIL_WIDTH: Pixels = px(54.);

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
    add_menu_open: bool,
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
            add_menu_open: false,
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
            if form == SidebarForm::Rail {
                self.add_menu_open = false;
            }
            cx.notify();
        }
    }

    fn toggle_add_menu(&mut self, cx: &mut Context<Self>) {
        self.add_menu_open = !self.add_menu_open;
        cx.notify();
    }

    fn row(&self, row: SidebarRow, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let rail = self.form == SidebarForm::Rail;
        match row {
            SidebarRow::Repository { id, name } => {
                if rail {
                    // In the rail a repository is the rule between two groups of marks.
                    div()
                        .flex_none()
                        .px(SPACE_3)
                        .py(SPACE_2)
                        .child(Divider::horizontal().selector(format!("sidebar-repository-{id}")))
                        .into_any_element()
                } else {
                    div()
                        .debug_selector(move || format!("sidebar-repository-{id}"))
                        .flex_none()
                        .px(SPACE_5)
                        .pt(SPACE_5)
                        .pb(SPACE_2)
                        .typed(TypeRole::MetaStrong)
                        .text_color(palette.text_muted)
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
                let select_id = id.clone();
                let on_click = cx.listener(move |sidebar, _: &ClickEvent, _, cx| {
                    sidebar
                        .workspaces
                        .update(cx, |list, cx| list.select(&select_id, cx));
                });
                let state = agent_presentation(agent, palette);
                let dot_selector = state
                    .selector_word
                    .map(|word| format!("sidebar-agent-{id}-{word}"));
                let mut dot = StateDot::new(state.dot);
                if agent == Some(SessionState::Works) {
                    dot = dot.pulsing(SharedString::from(format!("sidebar-pulse-{id}")));
                }
                if let Some(mark_selector) = dot_selector {
                    dot = dot.selector(mark_selector);
                }

                if rail {
                    let rail_dot =
                        agent.map(|_| div().absolute().top(SPACE_1).right(SPACE_1).child(dot));
                    let gone_mark = gone.then(|| {
                        let gone_selector = format!("sidebar-gone-{id}");
                        div()
                            .debug_selector(move || gone_selector)
                            .absolute()
                            .bottom(SPACE_1)
                            .right(SPACE_1)
                            .typed(TypeRole::Tiny)
                            .text_color(palette.text_dim)
                            .child("×")
                    });
                    ListRow::rail(SharedString::from(selector.clone()))
                        .selected(selected)
                        .selector(selector)
                        .on_click(on_click)
                        .child(
                            div()
                                .typed(TypeRole::Strong)
                                .text_color(if selected {
                                    palette.text
                                } else {
                                    palette.text_muted
                                })
                                .child(mark(&name)),
                        )
                        .children(rail_dot)
                        .children(gone_mark)
                        .into_any_element()
                } else {
                    let gone_mark = gone.then(|| {
                        let gone_selector = format!("sidebar-gone-{id}");
                        div()
                            .debug_selector(move || gone_selector)
                            .typed(TypeRole::Tiny)
                            .text_color(palette.danger)
                            .child("worktree gone")
                    });
                    ListRow::new(SharedString::from(selector.clone()))
                        .nested()
                        .selected(selected)
                        .selector(selector)
                        .on_click(on_click)
                        .child(div().flex_none().pt(SPACE_2).child(dot))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(SPACE_1)
                                .child(
                                    div()
                                        .typed(TypeRole::Path)
                                        .text_color(if selected {
                                            palette.text
                                        } else {
                                            palette.text_muted
                                        })
                                        .truncate()
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .typed(TypeRole::Meta)
                                        .text_color(state.word_colour)
                                        .truncate()
                                        .child(state.words),
                                )
                                .children(gone_mark),
                        )
                        .into_any_element()
                }
            }
        }
    }

    fn header(&self, rail: bool, cx: &mut Context<Self>) -> AnyElement {
        let pin = Button::new("sidebar-pin", if rail { "»" } else { "«" })
            .kind(ButtonKind::Ghost)
            .small()
            .selector("sidebar-pin")
            .on_click(cx.listener(|sidebar, _, _, cx| sidebar.pin_other_form(cx)));
        if rail {
            return div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .py(SPACE_2)
                .child(pin)
                .into_any_element();
        }
        let add = Button::new("sidebar-add", "+")
            .kind(ButtonKind::Ghost)
            .small()
            .selector("sidebar-add")
            .on_click(cx.listener(|sidebar, _, _, cx| sidebar.toggle_add_menu(cx)));
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .pr(SPACE_3)
            .py(SPACE_1)
            .child(SectionLabel::new("Workspaces"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_1)
                    .child(add)
                    .child(pin),
            )
            .into_any_element()
    }

    fn add_menu(&self, palette: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.add_menu_open || self.form == SidebarForm::Rail {
            return None;
        }
        let open_folder = ListRow::new("sidebar-add-open")
            .selector("sidebar-add-open")
            .on_click(cx.listener(|sidebar, _, window, cx| {
                sidebar.add_menu_open = false;
                cx.notify();
                window.dispatch_action(Box::new(OpenFolder), cx);
            }))
            .child(
                div()
                    .typed(TypeRole::Body)
                    .text_color(palette.text)
                    .child("Open Folder…"),
            );
        let new_worktree = ListRow::new("sidebar-add-worktree")
            .selector("sidebar-add-worktree")
            .on_click(cx.listener(|sidebar, _, window, cx| {
                sidebar.add_menu_open = false;
                cx.notify();
                window.dispatch_action(Box::new(NewWorktree), cx);
            }))
            .child(
                div()
                    .typed(TypeRole::Body)
                    .text_color(palette.text)
                    .child("New Worktree…"),
            );
        Some(
            div()
                .flex_none()
                .px(SPACE_3)
                .pb(SPACE_2)
                .child(
                    Card::new()
                        .selector("sidebar-add-menu")
                        .p(SPACE_1)
                        .gap(SPACE_1)
                        .child(open_folder)
                        .child(new_worktree),
                )
                .into_any_element(),
        )
    }
}

struct AgentPresentation {
    dot: State,
    words: &'static str,
    word_colour: Hsla,
    selector_word: Option<&'static str>,
}

fn agent_presentation(agent: Option<SessionState>, palette: &Palette) -> AgentPresentation {
    match agent {
        Some(SessionState::Works) => AgentPresentation {
            dot: State::Working,
            words: "working",
            word_colour: palette.link,
            selector_word: Some("working"),
        },
        Some(SessionState::WaitsForPermission) => AgentPresentation {
            dot: State::Permission,
            words: "waiting",
            word_colour: palette.danger,
            selector_word: Some("waiting"),
        },
        Some(SessionState::Unreviewed) => AgentPresentation {
            dot: State::NeedsYou,
            words: "to review",
            word_colour: palette.warn,
            selector_word: Some("unreviewed"),
        },
        Some(SessionState::NotRunning) => AgentPresentation {
            dot: State::Idle,
            words: "not running",
            word_colour: palette.text_dim,
            selector_word: Some("not-running"),
        },
        Some(SessionState::Idle) => AgentPresentation {
            dot: State::Ok,
            words: "idle",
            word_colour: palette.text_dim,
            selector_word: None,
        },
        None => AgentPresentation {
            dot: State::Idle,
            words: "no agent running",
            word_colour: palette.text_dim,
            selector_word: None,
        },
    }
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
        let palette = palette(cx);
        let rows: Vec<AnyElement> = self
            .rows(cx)
            .into_iter()
            .map(|row| self.row(row, &palette, cx))
            .collect();

        div()
            .debug_selector(|| "sidebar".to_owned())
            .flex_none()
            .h_full()
            .w(if rail { RAIL_WIDTH } else { FULL_WIDTH })
            .flex()
            .flex_col()
            .bg(palette.window)
            .text_color(palette.text)
            .child(self.header(rail, cx))
            .children(self.add_menu(&palette, cx))
            .child(
                div()
                    .id("sidebar-rows")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .px(SPACE_2)
                    .gap(SPACE_1)
                    .overflow_y_scroll()
                    .children(rows),
            )
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
