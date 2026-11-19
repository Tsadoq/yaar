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
//!
//! A row whose name or value this build does not know, as one a later version wrote into
//! a file it then left, is not listed: the setting holds its default.

use crate::{Store, StoreError, WorkspaceId};

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
}

impl Store {
    /// Every setting the user set: those of the whole app first, in the order
    /// [`Setting`] lists their kinds, then those of each workspace, in the order the
    /// workspaces were first kept. A setting never set is not listed.
    pub fn settings(&self) -> Result<Vec<Setting>, StoreError> {
        todo!("the Store's package of slice 5: read `setting` and `workspace_setting`")
    }

    /// Keeps `setting`, in the place of the value the same setting had: one transaction
    /// that returns when the change is on the drive, so that a Keeper killed after the
    /// call finds it. A setting of a workspace the Store does not hold is refused with a
    /// sentence that names it.
    pub fn set_setting(&self, setting: &Setting) -> Result<(), StoreError> {
        let _ = setting;
        todo!("the Store's package of slice 5: one row of `setting` or `workspace_setting`")
    }
}
