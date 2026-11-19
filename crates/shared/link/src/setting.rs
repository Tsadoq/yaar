//! What the two programs say about the user's settings: the ones the user set, kept by the
//! Keeper so that they outlast a closed Window and a Keeper started again.
//!
//! A Window reads every setting the user set with [`crate::Request::GetSettings`] when it
//! connects, sets one with [`crate::Request::SetSetting`], and is told a setting another
//! Window set as [`crate::Fact::SettingChanged`]. A setting the user never set is not
//! listed: the Window then holds its default, which is the Window's to know.
//!
//! Each kind of setting is one variant of [`Setting`], with a number of its own on the
//! wire. A later kind is one more variant, with the next number, and changes none of
//! these: the settings screen adds what a turn end selects, the time before the language
//! servers of a workspace off screen stop, the theme and the place of the terminal pane.

use crate::LinkError;
use crate::wire::{In, Out};
use crate::workspace::WorkspaceId;

/// The two forms of Flow: the steps of the agent in order, or the diff of the running
/// turn as it grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlowForm {
    /// Each read, edit, new file and command of the agent, in the order it happened.
    /// What Flow shows the first time.
    #[default]
    Timeline,
    /// The diff of the running turn, from the state the worktree was in when the turn
    /// started to the files on disk.
    LiveDiff,
}

/// One setting the user set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Setting {
    /// The form Flow shows in a workspace: the one the user chose there last.
    FlowForm {
        /// The workspace.
        workspace: WorkspaceId,
        /// The form.
        form: FlowForm,
    },
}

impl Setting {
    /// Whether `self` and `other` set the same thing, whatever its value: a setting the
    /// user sets takes the place of the one it is the same thing as.
    pub fn same_thing_as(&self, other: &Setting) -> bool {
        match (self, other) {
            (
                Setting::FlowForm { workspace, .. },
                Setting::FlowForm {
                    workspace: other, ..
                },
            ) => workspace == other,
        }
    }
}

fn write_form(out: &mut Out, form: FlowForm) {
    out.u8(match form {
        FlowForm::Timeline => 0,
        FlowForm::LiveDiff => 1,
    });
}

fn read_form(input: &mut In<'_>) -> Result<FlowForm, LinkError> {
    Ok(match input.u8()? {
        0 => FlowForm::Timeline,
        1 => FlowForm::LiveDiff,
        _ => return Err(LinkError::Malformed("an unknown form of Flow")),
    })
}

pub(crate) fn write_setting(out: &mut Out, setting: &Setting) {
    match setting {
        Setting::FlowForm { workspace, form } => {
            out.u8(0);
            out.text(workspace.as_str());
            write_form(out, *form);
        }
    }
}

pub(crate) fn read_setting(input: &mut In<'_>) -> Result<Setting, LinkError> {
    Ok(match input.u8()? {
        0 => Setting::FlowForm {
            workspace: WorkspaceId::new(input.text()?),
            form: read_form(input)?,
        },
        _ => return Err(LinkError::Malformed("an unknown kind of setting")),
    })
}
