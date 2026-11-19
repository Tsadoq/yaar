//! The shared model of the settings the user set, as the Keeper keeps them.
//!
//! Filled from the Keeper when the link connects, kept in step by
//! `Fact::SettingChanged`, and changed by a pane that sets one. The review pane reads the
//! form of Flow of the workspace on screen from it; the settings screen of slice 9 reads
//! and sets the rest through the same model.

use gpui_kit::*;
use yaar_link::{FlowForm, Setting, WorkspaceId};

use crate::link_model::KeeperLink;

/// The settings the user set, as the Keeper told them.
///
/// When the link connects it asks `Request::GetSettings` and takes the answer; each
/// `Fact::SettingChanged` takes the place of the setting it is the same thing as
/// (`Setting::same_thing_as`), or comes last. Of two answers, only that to the last
/// question counts, and a fact heard while a question is on its way has the question
/// asked again, as [`crate::AgentList`] does. A Keeper that refuses leaves the list as it
/// was: every setting then holds its default.
///
/// [`SettingList::set`] takes a setting at once, so that what the user chose shows before
/// the Keeper answered, and asks the Keeper to keep it with `Request::SetSetting`, which is
/// the user's request. A refusal has the list asked again, so that the model shows what
/// the Keeper keeps.
pub struct SettingList {
    link: Entity<KeeperLink>,
    settings: Vec<Setting>,
}

impl SettingList {
    /// The list of one window, fed by `link`: empty until the Keeper answers.
    ///
    /// The Frame's package of slice 5 makes it ask when the link connects and hear the
    /// facts, as [`crate::AgentList::new`] does.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        let _ = cx;
        Self {
            link,
            settings: Vec::new(),
        }
    }

    /// The link this list is fed by.
    pub fn link(&self) -> &Entity<KeeperLink> {
        &self.link
    }

    /// Every setting the user set, in the order the Keeper listed them, those set since
    /// in the place of the one they are the same thing as, or last.
    pub fn all(&self) -> &[Setting] {
        &self.settings
    }

    /// The form of Flow chosen last in `workspace`: [`FlowForm::Timeline`], the default,
    /// where none was chosen.
    pub fn flow_form(&self, workspace: &WorkspaceId) -> FlowForm {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::FlowForm {
                    workspace: of,
                    form,
                } if of == workspace => Some(*form),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Takes `setting` in the place of the one it is the same thing as, or last, tells the
    /// observers, and asks the Keeper to keep it with `Request::SetSetting`. The answer is
    /// not waited for; a refusal has the list asked again.
    pub fn set(&mut self, setting: Setting, cx: &mut Context<Self>) {
        let _ = (setting, cx);
        todo!("the Frame's package of slice 5: take it, then send SetSetting")
    }
}
