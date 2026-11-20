//! The user's settings: the ones the user set, kept so that a Keeper started again, and
//! every Window it serves, finds them.
//!
//! A setting the user never set is not kept, and [`Store::settings`] does not list it:
//! its default is the app's to know, not the Store's. The base of a workspace is a setting
//! too; it is kept with the workspace, by `Store::set_base`, and read with
//! `Store::workspaces`.
//!
//! How each is kept, in the tables of version 4: a setting of the whole app is one row of
//! `setting`, by its name; a setting of one workspace is one row of `workspace_setting`,
//! by the workspace and its name. Names and values are text:
//!
//! | Setting | Table | Name | Values |
//! | --- | --- | --- | --- |
//! | [`Setting::TurnEnd`] | `setting` | `turn_end` | `what_changed`, `keep_selection` |
//! | [`Setting::StopAfter`] | `setting` | `stop_after` | the seconds, in decimal digits |
//! | [`Setting::Theme`] | `setting` | `theme` | `system`, `light`, `dark` |
//! | [`Setting::TerminalPlace`] | `setting` | `terminal_place` | `right`, `below`, `left` |
//! | [`Setting::FlowForm`] | `workspace_setting` | `flow_form` | `timeline`, `live_diff` |
//! | [`Setting::ServersInRepository`] | `repository_setting`, from version 7 | `servers_inside` | `yes`, `no` |
//!
//! A setting of one repository is one row of `repository_setting`, by the repository and
//! its name; one of a repository the Store does not hold is refused, and a row of one it no
//! longer holds is not listed. Settings of repositories come last in [`Store::settings`],
//! after those of the workspaces, in the order the repositories were first kept.
//!
//! A row whose name or value this build does not know, as one a later version wrote into
//! a file it then left, is not listed: the setting holds its default.

use rusqlite::params;

use crate::reviews::{exists, refused};
use crate::{RepositoryId, Store, StoreError, WorkspaceId};

/// What the app selects in Changes when a turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnEndSelects {
    /// What changed since the reviewer's last review: the default.
    WhatChanged,
    /// Nothing: the selection stays as it is, and what changed is offered.
    KeepSelection,
}

/// The theme of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Theme {
    /// The light or the dark one, as the system's setting is: the default.
    System,
    /// The light one.
    Light,
    /// The dark one.
    Dark,
}

/// Where the panes after the first lie beside it, the terminal pane among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalPlace {
    /// To the right: the default.
    Right,
    /// Below.
    Below,
    /// To the left.
    Left,
}

/// The two forms of Flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowForm {
    /// The agent's steps in order: the default.
    Timeline,
    /// The diff of the running turn.
    LiveDiff,
}

/// One setting the user set, with its value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Setting {
    /// What a turn end selects in Changes.
    TurnEnd(TurnEndSelects),
    /// How long the language servers of a workspace off screen run on before they stop,
    /// in seconds.
    StopAfter {
        /// The seconds.
        seconds: u32,
    },
    /// The theme.
    Theme(Theme),
    /// Where the terminal pane lies.
    TerminalPlace(TerminalPlace),
    /// The form of Flow chosen last in a workspace.
    FlowForm {
        /// The workspace.
        workspace: WorkspaceId,
        /// The form.
        form: FlowForm,
    },
    /// The user's answer on the language servers that lie inside a repository: whether
    /// they may run.
    ServersInRepository {
        /// The repository.
        repository: RepositoryId,
        /// True for yes.
        allowed: bool,
    },
}

/// The name and value a setting of the whole app is kept as, or the workspace, name and
/// value of one of a workspace.
enum Row {
    App(&'static str, String),
    Workspace(String, &'static str, &'static str),
}

impl Setting {
    /// The row that keeps this setting.
    fn row(&self) -> Row {
        match self {
            Self::TurnEnd(selects) => Row::App(
                "turn_end",
                match selects {
                    TurnEndSelects::WhatChanged => "what_changed",
                    TurnEndSelects::KeepSelection => "keep_selection",
                }
                .to_owned(),
            ),
            Self::StopAfter { seconds } => Row::App("stop_after", seconds.to_string()),
            Self::Theme(theme) => Row::App(
                "theme",
                match theme {
                    Theme::System => "system",
                    Theme::Light => "light",
                    Theme::Dark => "dark",
                }
                .to_owned(),
            ),
            Self::TerminalPlace(place) => Row::App(
                "terminal_place",
                match place {
                    TerminalPlace::Right => "right",
                    TerminalPlace::Below => "below",
                    TerminalPlace::Left => "left",
                }
                .to_owned(),
            ),
            Self::FlowForm { workspace, form } => Row::Workspace(
                workspace.0.clone(),
                "flow_form",
                match form {
                    FlowForm::Timeline => "timeline",
                    FlowForm::LiveDiff => "live_diff",
                },
            ),
            Self::ServersInRepository { .. } => todo!(),
        }
    }

    /// The setting a row of `setting` holds; none for a name or a value this build does
    /// not know.
    fn of_app_row(name: &str, value: &str) -> Option<Self> {
        Some(match (name, value) {
            ("turn_end", "what_changed") => Self::TurnEnd(TurnEndSelects::WhatChanged),
            ("turn_end", "keep_selection") => Self::TurnEnd(TurnEndSelects::KeepSelection),
            ("stop_after", seconds) => Self::StopAfter {
                // Only the digits the Store wrote: no sign, no space.
                seconds: seconds
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
                    .then(|| seconds.parse().ok())
                    .flatten()?,
            },
            ("theme", "system") => Self::Theme(Theme::System),
            ("theme", "light") => Self::Theme(Theme::Light),
            ("theme", "dark") => Self::Theme(Theme::Dark),
            ("terminal_place", "right") => Self::TerminalPlace(TerminalPlace::Right),
            ("terminal_place", "below") => Self::TerminalPlace(TerminalPlace::Below),
            ("terminal_place", "left") => Self::TerminalPlace(TerminalPlace::Left),
            _ => return None,
        })
    }

    /// The setting a row of `workspace_setting` holds; none for a name or a value this
    /// build does not know.
    fn of_workspace_row(workspace: String, name: &str, value: &str) -> Option<Self> {
        let form = match (name, value) {
            ("flow_form", "timeline") => FlowForm::Timeline,
            ("flow_form", "live_diff") => FlowForm::LiveDiff,
            _ => return None,
        };
        Some(Self::FlowForm {
            workspace: WorkspaceId(workspace),
            form,
        })
    }

    /// Where a setting of the whole app comes among the others: the order its kind is
    /// listed in.
    fn rank(&self) -> u8 {
        match self {
            Self::TurnEnd(_) => 0,
            Self::StopAfter { .. } => 1,
            Self::Theme(_) => 2,
            Self::TerminalPlace(_) => 3,
            Self::FlowForm { .. } => 4,
            Self::ServersInRepository { .. } => 5,
        }
    }
}

impl Store {
    /// Every setting the user set: those of the whole app first, in the order
    /// [`Setting`] lists their kinds, then those of each workspace, in the order the
    /// workspaces were first kept. A setting never set is not listed.
    pub fn settings(&self) -> Result<Vec<Setting>, StoreError> {
        self.ask(|connection| {
            let mut settings = connection
                .prepare("SELECT name, value FROM setting")?
                .query_map([], |row| {
                    Ok(Setting::of_app_row(
                        &row.get::<_, String>(0)?,
                        &row.get::<_, String>(1)?,
                    ))
                })?
                .filter_map(|row| row.transpose())
                .collect::<rusqlite::Result<Vec<_>>>()?;
            settings.sort_by_key(Setting::rank);
            // A row of a workspace the Store does not hold is not listed either.
            let of_workspaces = connection
                .prepare(
                    "SELECT workspace_setting.workspace, workspace_setting.name,
                            workspace_setting.value
                     FROM workspace_setting JOIN workspace
                         ON workspace.id = workspace_setting.workspace
                     ORDER BY workspace.seq, workspace_setting.name",
                )?
                .query_map([], |row| {
                    Ok(Setting::of_workspace_row(
                        row.get(0)?,
                        &row.get::<_, String>(1)?,
                        &row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|row| row.transpose())
                .collect::<rusqlite::Result<Vec<_>>>()?;
            settings.extend(of_workspaces);
            Ok(settings)
        })
    }

    /// Keeps `setting`, in the place of the value the same setting had: one transaction
    /// that returns when the change is on the drive, so that a Keeper killed after the
    /// call finds it. A setting of a workspace the Store does not hold is refused with a
    /// sentence that names it.
    pub fn set_setting(&self, setting: &Setting) -> Result<(), StoreError> {
        let row = setting.row();
        self.try_ask(move |connection| {
            let keeping = connection.transaction()?;
            match row {
                Row::App(name, value) => {
                    keeping.execute(
                        "INSERT INTO setting (name, value) VALUES (?1, ?2)
                         ON CONFLICT (name) DO UPDATE SET value = excluded.value
                         WHERE value IS NOT excluded.value",
                        params![name, value],
                    )?;
                }
                Row::Workspace(workspace, name, value) => {
                    if !exists(
                        &keeping,
                        "SELECT 1 FROM workspace WHERE id = ?1",
                        &workspace,
                    )? {
                        return refused(format!("workspace {workspace} is not kept"));
                    }
                    keeping.execute(
                        "INSERT INTO workspace_setting (workspace, name, value)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT (workspace, name) DO UPDATE SET value = excluded.value
                         WHERE value IS NOT excluded.value",
                        params![workspace, name, value],
                    )?;
                }
            }
            Ok(keeping.commit()?)
        })
    }
}
