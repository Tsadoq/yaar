//! Settings the user set. One never set is not kept and not listed: its default is the app's to
//! know. The base of a workspace is kept with the workspace (`Store::set_base`).
//! Names and values are text: a setting of the app is a row of `setting`, one of a workspace
//! a row of `workspace_setting`, one of a repository a row of `repository_setting`.
//!
//! | Setting | Table | Name | Values |
//! | --- | --- | --- | --- |
//! | `TurnEnd` | `setting` | `turn_end` | `what_changed`, `keep_selection` |
//! | `StopAfter` | `setting` | `stop_after` | the seconds, in decimal digits |
//! | `Theme` | `setting` | `theme` | `system`, `light`, `dark`, `named:<name>` |
//! | `TerminalPlace` | `setting` | `terminal_place` | `right`, `below`, `left` |
//! | `KeepStates` | `setting` | `keep_states` | the days, in decimal digits |
//! | `OptionKey` | `setting` | `option_key` | `by_layout`, `meta`, `characters` |
//! | `FlowForm` | `workspace_setting` | `flow_form` | `timeline`, `live_diff` |
//! | `ServersInRepository` | `repository_setting` | `servers_inside` | `yes`, `no` |
//! | `WorktreesFolder` | `repository_setting` | `worktrees_folder` | absolute path, or empty |
//!
//! A row unknown to this build, of a workspace or repository the Store does not hold, or breaking
//! these rules is not listed. A theme name is 1 to 64 characters, none a control character;
//! `stop_after` and `keep_states` are plain digits, never 0; a worktrees folder is an absolute
//! UTF-8 path, and that it lies outside the repository's worktrees is the Workspaces part's to
//! check.

use std::path::{Path, PathBuf};

use rusqlite::params;

use crate::reviews::{exists, refused};
use crate::{Failed, RepositoryId, Store, StoreError, WorkspaceId};

const THEME_NAME_MOST: usize = 64;
const NAMED: &str = "named:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnEndSelects {
    WhatChanged,
    KeepSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Theme {
    System,
    Light,
    Dark,
    Named(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionKey {
    /// Meta on the U.S. layout, characters on every other.
    ByLayout,
    Meta,
    Characters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalPlace {
    Right,
    Below,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowForm {
    Timeline,
    LiveDiff,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Setting {
    TurnEnd(TurnEndSelects),
    StopAfter {
        seconds: u32,
    },
    Theme(Theme),
    TerminalPlace(TerminalPlace),
    KeepStates {
        days: u32,
    },
    OptionKey(OptionKey),
    FlowForm {
        workspace: WorkspaceId,
        form: FlowForm,
    },
    ServersInRepository {
        repository: RepositoryId,
        allowed: bool,
    },
    WorktreesFolder {
        repository: RepositoryId,
        /// An absolute folder; None for the rule the app follows when the user named none.
        folder: Option<PathBuf>,
    },
}

fn is_theme_name(name: &str) -> bool {
    (1..=THEME_NAME_MOST).contains(&name.chars().count()) && !name.chars().any(char::is_control)
}

/// Only plain digits (no sign, no space), and not 0: a time of 0 is no time anyone set.
fn whole_number(text: &str) -> Option<u32> {
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|number| *number > 0)
}

enum Row {
    App(&'static str, String),
    Workspace(String, &'static str, &'static str),
    Repository(String, &'static str, String),
}

impl Setting {
    fn row(&self) -> Result<Row, Failed> {
        Ok(match self {
            Self::TurnEnd(selects) => Row::App(
                "turn_end",
                match selects {
                    TurnEndSelects::WhatChanged => "what_changed",
                    TurnEndSelects::KeepSelection => "keep_selection",
                }
                .to_owned(),
            ),
            Self::StopAfter { seconds } => {
                if *seconds == 0 {
                    return refused(
                        "the setting stop_after is at least 1 second, not 0".to_owned(),
                    );
                }
                Row::App("stop_after", seconds.to_string())
            }
            Self::Theme(theme) => Row::App(
                "theme",
                match theme {
                    Theme::System => "system".to_owned(),
                    Theme::Light => "light".to_owned(),
                    Theme::Dark => "dark".to_owned(),
                    // The two built-in themes keep the rows `light` and `dark`, however they are
                    // asked for.
                    Theme::Named(name) if name == "Light" => "light".to_owned(),
                    Theme::Named(name) if name == "Dark" => "dark".to_owned(),
                    Theme::Named(name) => {
                        if !is_theme_name(name) {
                            return refused(format!(
                                "the setting theme needs a name of 1 to {THEME_NAME_MOST} \
                                 characters with no control character, not {name:?}"
                            ));
                        }
                        format!("{NAMED}{name}")
                    }
                },
            ),
            Self::KeepStates { days } => {
                if *days == 0 {
                    return refused("the setting keep_states is at least 1 day, not 0".to_owned());
                }
                Row::App("keep_states", days.to_string())
            }
            Self::OptionKey(key) => Row::App(
                "option_key",
                match key {
                    OptionKey::ByLayout => "by_layout",
                    OptionKey::Meta => "meta",
                    OptionKey::Characters => "characters",
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
            Self::ServersInRepository {
                repository,
                allowed,
            } => Row::Repository(
                repository.as_str().to_owned(),
                "servers_inside",
                if *allowed { "yes" } else { "no" }.to_owned(),
            ),
            Self::WorktreesFolder { repository, folder } => {
                let text = match folder {
                    None => "",
                    Some(folder) if folder.is_absolute() => match folder.to_str() {
                        Some(text) => text,
                        None => {
                            return refused(format!(
                                "the setting worktrees_folder of repository {repository} \
                                 needs a folder whose path is text"
                            ));
                        }
                    },
                    Some(folder) => {
                        return refused(format!(
                            "the setting worktrees_folder of repository {repository} \
                             needs an absolute folder, not {}",
                            folder.display()
                        ));
                    }
                };
                Row::Repository(
                    repository.as_str().to_owned(),
                    "worktrees_folder",
                    text.to_owned(),
                )
            }
        })
    }

    fn of_app_row(name: &str, value: &str) -> Option<Self> {
        Some(match (name, value) {
            ("turn_end", "what_changed") => Self::TurnEnd(TurnEndSelects::WhatChanged),
            ("turn_end", "keep_selection") => Self::TurnEnd(TurnEndSelects::KeepSelection),
            ("stop_after", seconds) => Self::StopAfter {
                seconds: whole_number(seconds)?,
            },
            ("keep_states", days) => Self::KeepStates {
                days: whole_number(days)?,
            },
            ("option_key", "by_layout") => Self::OptionKey(OptionKey::ByLayout),
            ("option_key", "meta") => Self::OptionKey(OptionKey::Meta),
            ("option_key", "characters") => Self::OptionKey(OptionKey::Characters),
            ("theme", "system") => Self::Theme(Theme::System),
            ("theme", "light") => Self::Theme(Theme::Light),
            ("theme", "dark") => Self::Theme(Theme::Dark),
            ("theme", named) if named.starts_with(NAMED) => {
                let name = &named[NAMED.len()..];
                is_theme_name(name).then(|| Self::Theme(Theme::Named(name.to_owned())))?
            }
            ("terminal_place", "right") => Self::TerminalPlace(TerminalPlace::Right),
            ("terminal_place", "below") => Self::TerminalPlace(TerminalPlace::Below),
            ("terminal_place", "left") => Self::TerminalPlace(TerminalPlace::Left),
            _ => return None,
        })
    }

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

    fn of_repository_row(repository: String, name: &str, value: &str) -> Option<Self> {
        let repository = RepositoryId::new(repository);
        Some(match (name, value) {
            ("servers_inside", "yes") => Self::ServersInRepository {
                repository,
                allowed: true,
            },
            ("servers_inside", "no") => Self::ServersInRepository {
                repository,
                allowed: false,
            },
            ("worktrees_folder", "") => Self::WorktreesFolder {
                repository,
                folder: None,
            },
            ("worktrees_folder", folder) if Path::new(folder).is_absolute() => {
                Self::WorktreesFolder {
                    repository,
                    folder: Some(PathBuf::from(folder)),
                }
            }
            _ => return None,
        })
    }

    fn rank(&self) -> u8 {
        match self {
            Self::TurnEnd(_) => 0,
            Self::StopAfter { .. } => 1,
            Self::Theme(_) => 2,
            Self::TerminalPlace(_) => 3,
            Self::KeepStates { .. } => 4,
            Self::OptionKey(_) => 5,
            Self::FlowForm { .. } => 6,
            Self::ServersInRepository { .. } => 7,
            Self::WorktreesFolder { .. } => 8,
        }
    }
}

impl Store {
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
            let of_repositories = connection
                .prepare(
                    "SELECT repository_setting.repository, repository_setting.name,
                            repository_setting.value
                     FROM repository_setting JOIN repository
                         ON repository.id = repository_setting.repository
                     ORDER BY repository.seq, repository_setting.name",
                )?
                .query_map([], |row| {
                    Ok(Setting::of_repository_row(
                        row.get(0)?,
                        &row.get::<_, String>(1)?,
                        &row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|row| row.transpose())
                .collect::<rusqlite::Result<Vec<_>>>()?;
            settings.extend(of_repositories);
            Ok(settings)
        })
    }

    pub fn set_setting(&self, setting: &Setting) -> Result<(), StoreError> {
        let row = setting.row()?;
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
                Row::Repository(repository, name, value) => {
                    if !exists(
                        &keeping,
                        "SELECT 1 FROM repository WHERE id = ?1",
                        &repository,
                    )? {
                        return refused(format!("repository {repository} is not kept"));
                    }
                    keeping.execute(
                        "INSERT INTO repository_setting (repository, name, value)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT (repository, name) DO UPDATE SET value = excluded.value
                         WHERE value IS NOT excluded.value",
                        params![repository, name, value],
                    )?;
                }
            }
            Ok(keeping.commit()?)
        })
    }
}
