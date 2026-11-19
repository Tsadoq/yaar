//! The user's settings, between the Link and the Store: what a Window reads when it
//! connects, and what it keeps when the user sets one.
//!
//! The Store keeps every setting the user set, those of the whole app and those of each
//! workspace; the Link carries the kinds a Window sets so far, which in slice 5 is the form
//! of Flow of a workspace. A setting the Store keeps that the Link has no word for yet is
//! not sent: the settings screen of slice 9 adds the words, and nothing here changes for
//! them but one arm of each mapping.
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
/// `Refusal::UnknownWorkspace`, and nothing is kept or told.
pub(crate) fn set(kept: &Kept, setting: Setting, facts: &mut Vec<Fact>) -> Result<Reply, Refusal> {
    let Setting::FlowForm { workspace, form } = &setting;
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
        yaar_store::Setting::TurnEnd(_)
        | yaar_store::Setting::StopAfter { .. }
        | yaar_store::Setting::Theme(_)
        | yaar_store::Setting::TerminalPlace(_) => None,
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
