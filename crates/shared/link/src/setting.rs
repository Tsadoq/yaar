//! Settings the user set, kept by the Keeper; the defaults are the Window's. Each kind has its own
//! number on the wire; a later kind takes the next and changes none of these.

use std::path::PathBuf;

use crate::LinkError;
use crate::wire::{In, Out};
use crate::workspace::{RepositoryId, WorkspaceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlowForm {
    Timeline,
    /// From the state the worktree was in when the turn started to the files on disk.
    #[default]
    LiveDiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TurnEndSelects {
    #[default]
    WhatChanged,
    KeepSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    /// `Light` and `Dark` are the two built in. A name no theme of the Window has is drawn as
    /// [`ThemeChoice::System`].
    Named(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TerminalPlace {
    #[default]
    Right,
    Below,
    Left,
}

/// What the Option key does in a terminal, on macOS. Elsewhere Alt is always Meta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OptionKey {
    /// Meta on the U.S. keyboard layout, the layout's characters on every other.
    #[default]
    ByLayout,
    Meta,
    Characters,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Setting {
    FlowForm {
        workspace: WorkspaceId,
        form: FlowForm,
    },
    /// Not set while the question waits for an answer.
    ServersInRepository {
        repository: RepositoryId,
        allowed: bool,
    },
    TurnEnd(TurnEndSelects),
    StopAfter {
        /// In seconds; at least 1.
        seconds: u32,
    },
    Theme(ThemeChoice),
    TerminalPlace(TerminalPlace),
    /// How long a recorded state nothing refers to is kept before the clean-up deletes it.
    KeepStates {
        /// In whole days; at least 1.
        days: u32,
    },
    OptionKey(OptionKey),
    /// Where the app creates the worktrees of a repository. With none set, or `folder: None`, a new
    /// worktree lies where the repository keeps its worktrees: the folder its linked worktrees
    /// share, else `<repository>.worktrees` beside it.
    WorktreesFolder {
        repository: RepositoryId,
        /// Absolute, outside every worktree of the repository and its git folder.
        folder: Option<PathBuf>,
    },
}

impl Setting {
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
            (
                Setting::WorktreesFolder { repository, .. },
                Setting::WorktreesFolder {
                    repository: other, ..
                },
            ) => repository == other,
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
        Setting::WorktreesFolder { repository, folder } => {
            out.u8(8);
            out.text(repository.as_str());
            match folder {
                None => out.u8(0),
                Some(folder) => {
                    out.u8(1);
                    out.path(folder);
                }
            }
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
        8 => Setting::WorktreesFolder {
            repository: RepositoryId::new(input.text()?),
            folder: match input.u8()? {
                0 => None,
                1 => Some(input.path()?),
                _ => {
                    return Err(LinkError::Malformed(
                        "a folder for worktrees neither none nor one",
                    ));
                }
            },
        },
        _ => return Err(LinkError::Malformed("an unknown kind of setting")),
    })
}
