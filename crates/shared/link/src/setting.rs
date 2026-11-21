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
//! these.
//!
//! | Setting | Number | Whose | Default, when the user set none |
//! | --- | --- | --- | --- |
//! | [`Setting::FlowForm`] | 0 | a workspace's | [`FlowForm::Timeline`] |
//! | [`Setting::ServersInRepository`] | 1 | a repository's | not answered: the question is asked |
//! | [`Setting::TurnEnd`] | 2 | the app's | [`TurnEndSelects::WhatChanged`] |
//! | [`Setting::StopAfter`] | 3 | the app's | 600 seconds |
//! | [`Setting::Theme`] | 4 | the app's | [`ThemeChoice::System`] |
//! | [`Setting::TerminalPlace`] | 5 | the app's | [`TerminalPlace::Right`] |
//! | [`Setting::KeepStates`] | 6 | the app's | 30 days |
//! | [`Setting::OptionKey`] | 7 | the app's | [`OptionKey::ByLayout`] |
//!
//! A setting of the app's is one of its kind: two settings of one kind are the same thing,
//! whatever their values ([`Setting::same_thing_as`]). The defaults are the Window's to
//! hold, as this table says; the Keeper keeps only what the user set.
//!
//! The user's answer to the question the review pane asks before a language server that
//! lies inside a repository first runs is a setting of that repository,
//! [`Setting::ServersInRepository`]: asked once for the repository, whichever of its
//! worktrees is on screen, and kept across restarts.

use crate::LinkError;
use crate::wire::{In, Out};
use crate::workspace::{RepositoryId, WorkspaceId};

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

/// What the review pane selects in Changes when a turn of an agent ends in the workspace
/// on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TurnEndSelects {
    /// What changed since the reviewer's last review: the default.
    #[default]
    WhatChanged,
    /// Nothing: the selection stays as it is, and what changed is offered.
    KeepSelection,
}

/// Which theme the app draws in.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum ThemeChoice {
    /// The light or the dark theme, as the system's setting is now and as it changes: the
    /// default.
    #[default]
    System,
    /// The theme of this name, whatever the system's setting: `Light` and `Dark` are the
    /// two the app is built with. A name no theme of the Window has is drawn as
    /// [`ThemeChoice::System`]: a later version may have kept a theme this one lacks.
    Named(String),
}

/// Where the panes after the first lie beside it: the terminal pane, in the app as the
/// Window program composes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TerminalPlace {
    /// To the right of the first pane: the default.
    #[default]
    Right,
    /// Below the first pane.
    Below,
    /// To the left of the first pane.
    Left,
}

/// What the Option key does in a terminal, on macOS. Elsewhere Alt is always Meta, and the
/// setting changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OptionKey {
    /// Meta on the U.S. keyboard layout, the characters of the layout on every other: the
    /// default.
    #[default]
    ByLayout,
    /// Meta on every layout: Option with a key sends that key with the Meta prefix.
    Meta,
    /// The characters the layout gives, on every layout.
    Characters,
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
    /// Whether the language servers that lie inside a repository may run: the user's
    /// answer to the question asked before the first of them runs. Not set while the
    /// question waits for an answer.
    ServersInRepository {
        /// The repository.
        repository: RepositoryId,
        /// The user's answer: true for yes.
        allowed: bool,
    },
    /// What a turn end selects in Changes.
    TurnEnd(TurnEndSelects),
    /// How long the language servers of a workspace off screen run on before they stop.
    StopAfter {
        /// The time, in seconds; at least 1.
        seconds: u32,
    },
    /// The theme.
    Theme(ThemeChoice),
    /// Where the terminal pane lies.
    TerminalPlace(TerminalPlace),
    /// How long the recorded state of a turn that nothing refers to is kept before it is
    /// deleted: the set time of the clean-up.
    KeepStates {
        /// The time, in whole days; at least 1.
        days: u32,
    },
    /// What the Option key does in a terminal.
    OptionKey(OptionKey),
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
            (
                Setting::ServersInRepository { repository, .. },
                Setting::ServersInRepository {
                    repository: other, ..
                },
            ) => repository == other,
            // A setting of the app's: one of its kind.
            (Setting::TurnEnd(_), Setting::TurnEnd(_))
            | (Setting::StopAfter { .. }, Setting::StopAfter { .. })
            | (Setting::Theme(_), Setting::Theme(_))
            | (Setting::TerminalPlace(_), Setting::TerminalPlace(_))
            | (Setting::KeepStates { .. }, Setting::KeepStates { .. })
            | (Setting::OptionKey(_), Setting::OptionKey(_)) => true,
            _ => false,
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
        Setting::ServersInRepository {
            repository,
            allowed,
        } => {
            out.u8(1);
            out.text(repository.as_str());
            out.u8(u8::from(*allowed));
        }
        Setting::TurnEnd(selects) => {
            out.u8(2);
            out.u8(match selects {
                TurnEndSelects::WhatChanged => 0,
                TurnEndSelects::KeepSelection => 1,
            });
        }
        Setting::StopAfter { seconds } => {
            out.u8(3);
            out.u32(*seconds);
        }
        Setting::Theme(choice) => {
            out.u8(4);
            match choice {
                ThemeChoice::System => out.u8(0),
                ThemeChoice::Named(name) => {
                    out.u8(1);
                    out.text(name);
                }
            }
        }
        Setting::TerminalPlace(place) => {
            out.u8(5);
            out.u8(match place {
                TerminalPlace::Right => 0,
                TerminalPlace::Below => 1,
                TerminalPlace::Left => 2,
            });
        }
        Setting::KeepStates { days } => {
            out.u8(6);
            out.u32(*days);
        }
        Setting::OptionKey(option) => {
            out.u8(7);
            out.u8(match option {
                OptionKey::ByLayout => 0,
                OptionKey::Meta => 1,
                OptionKey::Characters => 2,
            });
        }
    }
}

pub(crate) fn read_setting(input: &mut In<'_>) -> Result<Setting, LinkError> {
    Ok(match input.u8()? {
        0 => Setting::FlowForm {
            workspace: WorkspaceId::new(input.text()?),
            form: read_form(input)?,
        },
        1 => Setting::ServersInRepository {
            repository: RepositoryId::new(input.text()?),
            allowed: match input.u8()? {
                0 => false,
                1 => true,
                _ => return Err(LinkError::Malformed("an answer is neither 0 nor 1")),
            },
        },
        2 => Setting::TurnEnd(match input.u8()? {
            0 => TurnEndSelects::WhatChanged,
            1 => TurnEndSelects::KeepSelection,
            _ => return Err(LinkError::Malformed("an unknown selection at a turn's end")),
        }),
        3 => Setting::StopAfter {
            seconds: input.u32()?,
        },
        4 => Setting::Theme(match input.u8()? {
            0 => ThemeChoice::System,
            1 => ThemeChoice::Named(input.text()?),
            _ => return Err(LinkError::Malformed("an unknown choice of theme")),
        }),
        5 => Setting::TerminalPlace(match input.u8()? {
            0 => TerminalPlace::Right,
            1 => TerminalPlace::Below,
            2 => TerminalPlace::Left,
            _ => {
                return Err(LinkError::Malformed(
                    "an unknown place of the terminal pane",
                ));
            }
        }),
        6 => Setting::KeepStates { days: input.u32()? },
        7 => Setting::OptionKey(match input.u8()? {
            0 => OptionKey::ByLayout,
            1 => OptionKey::Meta,
            2 => OptionKey::Characters,
            _ => return Err(LinkError::Malformed("an unknown setting of the Option key")),
        }),
        _ => return Err(LinkError::Malformed("an unknown kind of setting")),
    })
}
