//! The user's settings: the ones the user set, kept so that a Keeper started again, and
//! every Window it serves, finds them.
//!
//! A setting the user never set is not kept, and [`Store::settings`] does not list it:
//! its default is the app's to know, not the Store's. The base of a workspace is a setting
//! too; it is kept with the workspace, by `Store::set_base`, and read with
//! `Store::workspaces`.
//!
//! How each is kept: a setting of the whole app is one row of `setting`, by its name; a
//! setting of one workspace is one row of `workspace_setting`, by the workspace and its
//! name; and, from version 7, a setting of one repository is one row of
//! `repository_setting`, by the repository and its name. Names and values are text:
//!
//! | Setting | Table | Name | Values |
//! | --- | --- | --- | --- |
//! | [`Setting::TurnEnd`] | `setting` | `turn_end` | `what_changed`, `keep_selection` |
//! | [`Setting::StopAfter`] | `setting` | `stop_after` | the seconds, in decimal digits |
//! | [`Setting::Theme`] | `setting` | `theme` | `system`, `light`, `dark`, from slice 9 also `named:` and a theme's name |
//! | [`Setting::TerminalPlace`] | `setting` | `terminal_place` | `right`, `below`, `left` |
//! | [`Setting::KeepStates`], from slice 9 | `setting` | `keep_states` | the days, in decimal digits |
//! | [`Setting::OptionKey`], from slice 9 | `setting` | `option_key` | `by_layout`, `meta`, `characters` |
//! | [`Setting::FlowForm`] | `workspace_setting` | `flow_form` | `timeline`, `live_diff` |
//! | [`Setting::ServersInRepository`] | `repository_setting`, from version 7 | `servers_inside` | `yes`, `no` |
//! | [`Setting::WorktreesFolder`], from slice 9 | `repository_setting` | `worktrees_folder` | the absolute folder as text; the empty text for `folder: None` |
//!
//! A setting of a repository the Store does not hold is refused, and a row of one it no longer
//! holds is not listed. Settings of repositories come last in [`Store::settings`], after
//! those of the workspaces, in the order the repositories were first kept.
//!
//! A row whose name or value this build does not know, as one a later version wrote into
//! a file it then left, is not listed: the setting holds its default.
//!
//! Slice 9 adds two names and one form of value, and no table: the version of the tables
//! stays 7. A theme other than the two built in is kept as `named:<name>`, its name as
//! the user saw it: 1 to 64 characters, none of them a control character; `light` and
//! `dark` stay as they were written before. The seconds of `stop_after` and the days of
//! `keep_states` are read only as plain digits and never 0: a row of 0 is not listed, and
//! [`Store::set_setting`] refuses one, as it refuses an empty or too long theme's name,
//! with a sentence that names the setting.
//!
//! A folder for the worktrees of a repository is kept as the text of its path, which must
//! be absolute and valid UTF-8 (the Store keeps text): [`Store::set_setting`] refuses any
//! other, with a sentence that names the setting. `folder: None`, the user's way back to
//! the rule the app follows when no folder is named, is kept as the empty text and listed
//! as `folder: None`; a row whose text is neither empty nor an absolute path is not listed.
//! That the folder lies outside every worktree of the repository and its git folder is the
//! Workspaces part's to check, which knows the worktrees; the Store knows only text.

use std::path::{Path, PathBuf};

use rusqlite::params;

use crate::reviews::{exists, refused};
use crate::{Failed, RepositoryId, Store, StoreError, WorkspaceId};

/// The longest theme name the Store keeps, in characters.
const THEME_NAME_MOST: usize = 64;
/// What a theme's row starts with when the theme is not one of the two built in.
const NAMED: &str = "named:";

/// What the app selects in Changes when a turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnEndSelects {
    /// What changed since the reviewer's last review: the default.
    WhatChanged,
    /// Nothing: the selection stays as it is, and what changed is offered.
    KeepSelection,
}

/// The theme of the app.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Theme {
    /// The light or the dark one, as the system's setting is: the default.
    System,
    /// The light one.
    Light,
    /// The dark one.
    Dark,
    /// Another theme, by its name.
    Named(String),
}

/// What the Option key does in a terminal, on macOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionKey {
    /// Meta on the U.S. layout, characters on every other: the default.
    ByLayout,
    /// Meta on every layout.
    Meta,
    /// Characters on every layout.
    Characters,
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
    /// How long a recorded state nothing refers to is kept, in whole days.
    KeepStates {
        /// The days; at least 1.
        days: u32,
    },
    /// What the Option key does in a terminal.
    OptionKey(OptionKey),
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
    /// The folder the worktrees the app creates for a repository go in.
    WorktreesFolder {
        /// The repository.
        repository: RepositoryId,
        /// An absolute folder; none for the rule the app follows when the user named none.
        folder: Option<PathBuf>,
    },
}

/// Whether `name` is one the Store keeps for a theme: 1 to 64 characters, none of them a
/// control character.
fn is_theme_name(name: &str) -> bool {
    (1..=THEME_NAME_MOST).contains(&name.chars().count()) && !name.chars().any(char::is_control)
}

/// The number the text is, when it is only the digits the Store writes (no sign, no space)
/// and not 0: a time of 0 is no time anyone set.
fn whole_number(text: &str) -> Option<u32> {
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|number| *number > 0)
}

/// The name and value a setting of the whole app is kept as, or the workspace, name and
/// value of one of a workspace, or the repository, name and value of one of a repository.
enum Row {
    App(&'static str, String),
    Workspace(String, &'static str, &'static str),
    Repository(String, &'static str, String),
}

impl Setting {
    /// The row that keeps this setting, or the Store's refusal of its value.
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
                    // The two built-in themes keep the rows slice 5 wrote, however they
                    // are asked for.
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

    /// The setting a row of `setting` holds; none for a name or a value this build does
    /// not know.
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

    /// The setting a row of `repository_setting` holds; none for a name or a value this
    /// build does not know.
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

    /// Where a setting of the whole app comes among the others: the order its kind is
    /// listed in.
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
    /// Every setting the user set: those of the whole app first, in the order
    /// [`Setting`] lists their kinds, then those of each workspace, in the order the
    /// workspaces were first kept, then those of each repository, in the order the
    /// repositories were first kept. A setting never set is not listed.
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
            // The same for a repository the Store does not hold.
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

    /// Keeps `setting`, in the place of the value the same setting had: one transaction
    /// that returns when the change is on the drive, so that a Keeper killed after the
    /// call finds it. A setting of a workspace or of a repository the Store does not hold
    /// is refused with a sentence that names it, and so is a value the module doc's table
    /// refuses.
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
