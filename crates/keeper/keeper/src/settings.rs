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
//! which only the Workspaces part can do: its set is `wiring::worktrees::set_folder`'s,
//! beside the creation of a worktree that reads it.
//! Every setting the Store lists is sent, each mapped back the same way.
//!
//! `GetSettings` waits the Keeper's turn, and a kept setting is told as
//! `Fact::SettingChanged` to every Window under the same turn, after the reply of the
//! request that set it: a Window that asks for the settings never takes an older list for a
//! newer one.

use yaar_link::{Fact, FlowForm, Refusal, Reply, Setting};

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
    let _ = (kept, setting, facts);
    todo!("keep a setting of the app, as the module doc says")
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
        // Until the settings of the app have their words here, none is sent.
        yaar_store::Setting::TurnEnd(_)
        | yaar_store::Setting::StopAfter { .. }
        | yaar_store::Setting::Theme(_)
        | yaar_store::Setting::TerminalPlace(_)
        | yaar_store::Setting::KeepStates { .. }
        | yaar_store::Setting::OptionKey(_)
        | yaar_store::Setting::WorktreesFolder { .. } => None,
    }
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
