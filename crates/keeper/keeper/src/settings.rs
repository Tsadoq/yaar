//! The user's settings, between the Link and the Store: what a Window reads when it
//! connects, and what it keeps when the user sets one.
//!
//! The Store keeps every setting the user set, those of the whole app, those of each
//! workspace and those of each repository, and the Link carries every kind:
//!
//! | The Link's | The Store's | Refused, with nothing kept or told |
//! | --- | --- | --- |
//! | `Setting::FlowForm` | `Setting::FlowForm` | `UnknownWorkspace` for a workspace the Store does not hold |
//! | `Setting::ServersInRepository` | `Setting::ServersInRepository` | `Failed` for a repository it does not hold |
//! | `Setting::TurnEnd` | `Setting::TurnEnd` | never |
//! | `Setting::StopAfter` | `Setting::StopAfter` | `Failed` for 0 seconds |
//! | `Setting::Theme`, `System` | `Theme::System` | never |
//! | `Setting::Theme`, `Named("Light")`, `Named("Dark")` | `Theme::Light`, `Theme::Dark`, so that a Store of slice 5 reads them as it wrote them | never |
//! | `Setting::Theme`, any other name | `Theme::Named` | `Failed` for an empty name, or one the Store refuses |
//! | `Setting::TerminalPlace` | `Setting::TerminalPlace` | never |
//! | `Setting::KeepStates` | `Setting::KeepStates` | `Failed` for 0 days |
//! | `Setting::OptionKey` | `Setting::OptionKey` | never |
//! | `Setting::WorktreesFolder` | `Setting::WorktreesFolder` | `Failed` for a repository it does not hold, and for a folder `Workspaces::check_worktrees_folder` refuses, with its sentence |
//!
//! A folder for worktrees is checked against the repository's worktrees before it is kept,
//! which only the Workspaces part can do: its set is `wiring::set_worktrees_folder`'s, beside
//! the creation of a worktree that reads it.
//! Every setting the Store lists is sent, each mapped back the same way.
//!
//! `GetSettings` waits the Keeper's turn, and a kept setting is told as
//! `Fact::SettingChanged` to every Window under the same turn, after the reply of the
//! request that set it: a Window that asks for the settings never takes an older list for a
//! newer one.

use yaar_link::{
    Fact, FlowForm, OptionKey, Refusal, Reply, Setting, TerminalPlace, ThemeChoice, TurnEndSelects,
};

use crate::wiring::Kept;

/// `Request::GetSettings`: every setting the Store keeps that the Link has a word for, as
/// `Reply::Settings`, in the Store's order.
pub(crate) fn get(kept: &Kept) -> Result<Reply, Refusal> {
    let kept = kept.store.settings().map_err(failed)?;
    Ok(Reply::Settings(
        kept.iter().filter_map(link_setting).collect(),
    ))
}

/// `Request::SetSetting`: keeps `setting` in the Store, answers `Reply::Done` once it is
/// there, and puts `Fact::SettingChanged` with it into `facts`, for every Window. A
/// setting of a workspace the Keeper does not know is refused with
/// `Refusal::UnknownWorkspace`, and one of a repository it does not know with
/// `Refusal::Failed`, and nothing is kept or told.
pub(crate) fn set(kept: &Kept, setting: Setting, facts: &mut Vec<Fact>) -> Result<Reply, Refusal> {
    let (workspace, form) = match &setting {
        Setting::FlowForm { workspace, form } => (workspace, form),
        Setting::ServersInRepository {
            repository,
            allowed,
        } => return set_of_repository(kept, repository, *allowed, facts),
        Setting::WorktreesFolder { repository, folder } => {
            return crate::wiring::set_worktrees_folder(kept, repository, folder.as_deref(), facts);
        }
        Setting::TurnEnd(_)
        | Setting::StopAfter { .. }
        | Setting::Theme(_)
        | Setting::TerminalPlace(_)
        | Setting::KeepStates { .. }
        | Setting::OptionKey(_) => return set_of_app(kept, setting, facts),
    };
    let store_workspace = yaar_store::WorkspaceId::new(workspace.as_str());
    let known = kept.store.workspaces().map_err(failed)?;
    if !known.iter().any(|known| known.id == store_workspace) {
        return Err(Refusal::UnknownWorkspace {
            workspace: workspace.clone(),
        });
    }
    kept.store
        .set_setting(&yaar_store::Setting::FlowForm {
            workspace: store_workspace,
            form: store_form(*form),
        })
        .map_err(failed)?;
    facts.push(Fact::SettingChanged(setting));
    Ok(Reply::Done)
}

/// `Request::SetSetting` of a setting of a repository: keeps the user's answer on the
/// language servers inside it in the Store, answers `Reply::Done` once it is there, and puts
/// `Fact::SettingChanged` with it into `facts`. A repository the Keeper does not know is
/// refused with `Refusal::Failed` naming it, and nothing is kept or told. The answer is the
/// repository's, whichever of its worktrees the Window has on screen.
fn set_of_repository(
    kept: &Kept,
    repository: &yaar_link::RepositoryId,
    allowed: bool,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let store_repository = yaar_store::RepositoryId::new(repository.as_str());
    let known = kept.store.repositories().map_err(failed)?;
    if !known.iter().any(|known| known.id == store_repository) {
        return Err(Refusal::Failed {
            reason: format!("the repository {repository} is not known"),
        });
    }
    kept.store
        .set_setting(&yaar_store::Setting::ServersInRepository {
            repository: store_repository,
            allowed,
        })
        .map_err(failed)?;
    facts.push(Fact::SettingChanged(Setting::ServersInRepository {
        repository: repository.clone(),
        allowed,
    }));
    Ok(Reply::Done)
}

/// `Request::SetSetting` of a setting of the whole app: keeps it in the Store, as the module
/// doc's table maps it, answers `Reply::Done` once it is there, and puts
/// `Fact::SettingChanged` with it into `facts`. A value the table refuses is refused with
/// `Refusal::Failed` naming the setting, and nothing is kept or told.
fn set_of_app(kept: &Kept, setting: Setting, facts: &mut Vec<Fact>) -> Result<Reply, Refusal> {
    kept.store
        .set_setting(&store_setting_of_app(&setting)?)
        .map_err(failed)?;
    facts.push(Fact::SettingChanged(setting));
    Ok(Reply::Done)
}

/// A setting of the Store in the Link's words; none for a kind the Link has no word for.
fn link_setting(setting: &yaar_store::Setting) -> Option<Setting> {
    match setting {
        yaar_store::Setting::FlowForm { workspace, form } => Some(Setting::FlowForm {
            workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
            form: match form {
                yaar_store::FlowForm::Timeline => FlowForm::Timeline,
                yaar_store::FlowForm::LiveDiff => FlowForm::LiveDiff,
            },
        }),
        yaar_store::Setting::ServersInRepository {
            repository,
            allowed,
        } => Some(Setting::ServersInRepository {
            repository: yaar_link::RepositoryId::new(repository.as_str()),
            allowed: *allowed,
        }),
        yaar_store::Setting::TurnEnd(selects) => Some(Setting::TurnEnd(match selects {
            yaar_store::TurnEndSelects::WhatChanged => TurnEndSelects::WhatChanged,
            yaar_store::TurnEndSelects::KeepSelection => TurnEndSelects::KeepSelection,
        })),
        yaar_store::Setting::StopAfter { seconds } => {
            Some(Setting::StopAfter { seconds: *seconds })
        }
        yaar_store::Setting::Theme(theme) => Some(Setting::Theme(match theme {
            yaar_store::Theme::System => ThemeChoice::System,
            yaar_store::Theme::Light => ThemeChoice::Named("Light".to_owned()),
            yaar_store::Theme::Dark => ThemeChoice::Named("Dark".to_owned()),
            yaar_store::Theme::Named(name) => ThemeChoice::Named(name.clone()),
        })),
        yaar_store::Setting::TerminalPlace(place) => Some(Setting::TerminalPlace(match place {
            yaar_store::TerminalPlace::Right => TerminalPlace::Right,
            yaar_store::TerminalPlace::Below => TerminalPlace::Below,
            yaar_store::TerminalPlace::Left => TerminalPlace::Left,
        })),
        yaar_store::Setting::KeepStates { days } => Some(Setting::KeepStates { days: *days }),
        yaar_store::Setting::OptionKey(key) => Some(Setting::OptionKey(match key {
            yaar_store::OptionKey::ByLayout => OptionKey::ByLayout,
            yaar_store::OptionKey::Meta => OptionKey::Meta,
            yaar_store::OptionKey::Characters => OptionKey::Characters,
        })),
        yaar_store::Setting::WorktreesFolder { repository, folder } => {
            Some(Setting::WorktreesFolder {
                repository: yaar_link::RepositoryId::new(repository.as_str()),
                folder: folder.clone(),
            })
        }
    }
}

/// A setting of the whole app in the Store's words. The two themes the app is built with
/// are the Store's `Light` and `Dark`, so that the row a Store of slice 5 wrote is read the
/// same; any other name is `Named`, which the Store refuses when it is empty or too long.
/// A setting that is not one of the whole app's is refused with `Refusal::Failed`: `set`
/// keeps those apart, so no request reaches that arm.
fn store_setting_of_app(setting: &Setting) -> Result<yaar_store::Setting, Refusal> {
    Ok(match setting {
        Setting::TurnEnd(selects) => yaar_store::Setting::TurnEnd(match selects {
            TurnEndSelects::WhatChanged => yaar_store::TurnEndSelects::WhatChanged,
            TurnEndSelects::KeepSelection => yaar_store::TurnEndSelects::KeepSelection,
        }),
        Setting::StopAfter { seconds } => yaar_store::Setting::StopAfter { seconds: *seconds },
        Setting::Theme(choice) => yaar_store::Setting::Theme(match choice {
            ThemeChoice::System => yaar_store::Theme::System,
            ThemeChoice::Named(name) if name == "Light" => yaar_store::Theme::Light,
            ThemeChoice::Named(name) if name == "Dark" => yaar_store::Theme::Dark,
            ThemeChoice::Named(name) => yaar_store::Theme::Named(name.clone()),
        }),
        Setting::TerminalPlace(place) => yaar_store::Setting::TerminalPlace(match place {
            TerminalPlace::Right => yaar_store::TerminalPlace::Right,
            TerminalPlace::Below => yaar_store::TerminalPlace::Below,
            TerminalPlace::Left => yaar_store::TerminalPlace::Left,
        }),
        Setting::KeepStates { days } => yaar_store::Setting::KeepStates { days: *days },
        Setting::OptionKey(key) => yaar_store::Setting::OptionKey(match key {
            OptionKey::ByLayout => yaar_store::OptionKey::ByLayout,
            OptionKey::Meta => yaar_store::OptionKey::Meta,
            OptionKey::Characters => yaar_store::OptionKey::Characters,
        }),
        Setting::FlowForm { .. }
        | Setting::ServersInRepository { .. }
        | Setting::WorktreesFolder { .. } => {
            return Err(Refusal::Failed {
                reason: format!("{setting:?} is not a setting of the whole app"),
            });
        }
    })
}

/// A form of Flow in the Store's words.
fn store_form(form: FlowForm) -> yaar_store::FlowForm {
    match form {
        FlowForm::Timeline => yaar_store::FlowForm::Timeline,
        FlowForm::LiveDiff => yaar_store::FlowForm::LiveDiff,
    }
}

/// The refusal of a call the Store failed.
fn failed(error: impl std::fmt::Display) -> Refusal {
    Refusal::Failed {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use yaar_link::{FlowForm, RepositoryId, Setting, WorkspaceId};

    use super::*;

    /// `set` keeps the settings of a workspace and of a repository apart, so that no request
    /// reaches the conversion of the whole app's settings with one of them; the conversion
    /// still answers with the Link's refusal, a sentence naming the setting, and does not
    /// panic.
    #[test]
    fn a_setting_that_is_not_the_whole_apps_is_refused_with_the_links_refusal() {
        let not_the_apps = [
            Setting::FlowForm {
                workspace: WorkspaceId::new("w"),
                form: FlowForm::Timeline,
            },
            Setting::ServersInRepository {
                repository: RepositoryId::new("r"),
                allowed: true,
            },
            Setting::WorktreesFolder {
                repository: RepositoryId::new("r"),
                folder: Some(PathBuf::from("/a")),
            },
        ];
        for setting in not_the_apps {
            match store_setting_of_app(&setting) {
                Err(Refusal::Failed { reason }) => {
                    assert!(
                        reason.contains("not a setting of the whole app"),
                        "{reason}"
                    );
                    assert!(
                        reason.contains(&format!("{setting:?}")),
                        "the refusal names the setting: {reason}"
                    );
                }
                other => panic!("{setting:?} was not refused with Failed: {other:?}"),
            }
        }
    }
}
