//! The user's settings between the Link and the Store.
//! A kept setting is told as `Fact::SettingChanged` after the reply of the request that set it, so
//! a Window never takes an older list for a newer one.

use yaar_link::{
    Fact, FlowForm, OptionKey, Refusal, Reply, Setting, TerminalPlace, ThemeChoice, TurnEndSelects,
};

use crate::wiring::Kept;

pub(crate) fn get(kept: &Kept) -> Result<Reply, Refusal> {
    let kept = kept.store.settings().map_err(failed)?;
    Ok(Reply::Settings(
        kept.iter().filter_map(link_setting).collect(),
    ))
}

/// Refused with `UnknownWorkspace` for an unknown workspace and `Failed` for an unknown repository;
/// nothing is kept or told.
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

/// The answer is the repository's, whichever of its worktrees the Window has on screen.
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

fn set_of_app(kept: &Kept, setting: Setting, facts: &mut Vec<Fact>) -> Result<Reply, Refusal> {
    kept.store
        .set_setting(&store_setting_of_app(&setting)?)
        .map_err(failed)?;
    facts.push(Fact::SettingChanged(setting));
    Ok(Reply::Done)
}

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

/// Any theme but the app's own maps to `Named`, which the Store refuses when empty or too long.
/// `set` routes the other kinds apart: the refusal arm is unreachable by a request.
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

fn store_form(form: FlowForm) -> yaar_store::FlowForm {
    match form {
        FlowForm::Timeline => yaar_store::FlowForm::Timeline,
        FlowForm::LiveDiff => yaar_store::FlowForm::LiveDiff,
    }
}

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
