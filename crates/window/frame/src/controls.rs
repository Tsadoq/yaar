//! The actions of the Window that have a key, in one table. `secondary-` is Cmd on macOS, Ctrl
//! elsewhere. A `Place::Anywhere` key binds with no context where it is Cmd, else in `KEY_CONTEXT`
//! outside `PROGRAM_CONTEXT`; any other key binds in its row's context.

use gpui_kit::*;
use yaar_link::RequestKind;

actions!(
    yaar,
    [
        OpenFolder,
        SetBase,
        /// Pins the sidebar in the form it does not have now.
        ToggleSidebar,
        UnpinSidebar,
        SelectNextWorkspace,
        SelectPreviousWorkspace,
        FocusNextPane,
        FocusPreviousPane,
        MovePanes,
        /// The plain quit: the Keeper, the agents and the terminals' programs go on.
        Quit,
        QuitAndStopAgents,
        ShowFinishedTurn,
        OpenSettings,
        CloseSettings,
        NextSetting,
        PreviousSetting,
        NextValue,
        PreviousValue,
        OpenSetting,
        /// Deletes at once the recorded states nothing refers to, with what only they held.
        CleanUpStates,
        /// Asks for a new branch's name and makes a worktree on it with the chosen agent CLI: Enter
        /// in the field wants [`crate::Wanted::Worktree`], which the terminal pane sends as
        /// `CreateWorktree`.
        NewWorktree,
        /// The app quits the plain way when its last window is closed.
        CloseWindow,
        HideApp,
        HideOtherApps,
        Minimize,
    ]
);

pub const KEY_CONTEXT: &str = "Frame";

pub use crate::settings_screen::SETTINGS_CONTEXT;

pub const PROGRAM_CONTEXT: &str = "Program";

const SECONDARY_IS_CMD: bool = cfg!(target_os = "macos");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Wherever the focus is, except, where `secondary` is not Cmd, inside an element that sets
    /// [`PROGRAM_CONTEXT`].
    Anywhere,
    In(&'static str),
    /// Wherever the focus is, inside [`PROGRAM_CONTEXT`] too: a kept key never reaches a program.
    Kept,
}

impl Place {
    fn context(self, secondary_is_cmd: bool) -> Option<&'static str> {
        match self {
            Place::In(context) => Some(context),
            Place::Kept => Some(KEY_CONTEXT),
            Place::Anywhere if secondary_is_cmd => None,
            // The Frame's context holds in a program too, so the key is also asked not to be inside
            // one: GPUI's `!` looks at every context from the root to the focus.
            Place::Anywhere => Some("Frame && !Program"),
        }
    }
}

fn write_key(key: &str, secondary_is_cmd: bool) -> String {
    let modifier = if secondary_is_cmd { "cmd" } else { "ctrl" };
    key.split(' ')
        .map(|keystroke| {
            let mut written = String::new();
            let mut rest = keystroke;
            // A `-` that ends the keystroke is the key, as in `secondary--`.
            while let Some((part, tail)) = rest.split_once('-')
                && !tail.is_empty()
            {
                written.push_str(if part == "secondary" { modifier } else { part });
                written.push('-');
                rest = tail;
            }
            written.push_str(rest);
            written
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asks {
    Nothing,
    Folder,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sends {
    Request(RequestKind),
    StopEverything,
}

#[derive(Debug, Clone, Copy)]
pub struct Control {
    pub title: &'static str,
    pub action: fn() -> Box<dyn Action>,
    pub keystroke: &'static str,
    pub place: Place,
    pub sends: Option<Sends>,
    pub asks: Asks,
}

impl Control {
    pub fn context(&self) -> Option<&'static str> {
        self.place.context(SECONDARY_IS_CMD)
    }

    pub fn key(&self) -> String {
        write_key(self.keystroke, SECONDARY_IS_CMD)
    }
}

/// Keys the toolkit binds for the whole window that a program in a terminal must get: Tab and
/// Shift-Tab (focus moves), and off macOS Ctrl-C (copy) and Ctrl-Shift-I (inspector).
pub const PROGRAM_KEYS: &[&str] = if SECONDARY_IS_CMD {
    &["tab", "shift-tab"]
} else {
    &["tab", "shift-tab", "ctrl-c", "ctrl-shift-i"]
};

/// Binds each to nothing in [`PROGRAM_CONTEXT`]: a binding to nothing in a deeper context hides the
/// same key's outranked bindings, so it goes on to the program. Call it after `gpui_kit::init`: of
/// two bindings at the same depth the later wins.
pub fn release_program_keys(cx: &mut App) {
    cx.bind_keys(
        PROGRAM_KEYS
            .iter()
            .map(|key| KeyBinding::new(key, NoAction, Some(PROGRAM_CONTEXT))),
    );
}

pub fn controls() -> Vec<Control> {
    vec![
        Control {
            title: "Open Folder…",
            action: || Box::new(OpenFolder),
            keystroke: "secondary-o",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::OpenFolder)),
            asks: Asks::Folder,
        },
        Control {
            title: "Set Base…",
            action: || Box::new(SetBase),
            keystroke: "secondary-shift-b",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::SetBase)),
            asks: Asks::Text,
        },
        Control {
            title: "Settings…",
            action: || Box::new(OpenSettings),
            keystroke: "secondary-,",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "New Worktree with Agent…",
            action: || Box::new(NewWorktree),
            keystroke: "secondary-shift-n",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::CreateWorktree)),
            asks: Asks::Text,
        },
        Control {
            title: "Toggle Sidebar",
            action: || Box::new(ToggleSidebar),
            keystroke: "secondary-b",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Sidebar by Window Width",
            action: || Box::new(UnpinSidebar),
            keystroke: "secondary-alt-b",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Next Workspace",
            action: || Box::new(SelectNextWorkspace),
            keystroke: "secondary-alt-down",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Previous Workspace",
            action: || Box::new(SelectPreviousWorkspace),
            keystroke: "secondary-alt-up",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Next Pane",
            action: || Box::new(FocusNextPane),
            keystroke: "ctrl-tab",
            place: Place::Kept,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Previous Pane",
            action: || Box::new(FocusPreviousPane),
            keystroke: "ctrl-shift-tab",
            place: Place::Kept,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Move Panes",
            action: || Box::new(MovePanes),
            keystroke: "secondary-alt-p",
            place: Place::Anywhere,
            sends: Some(Sends::Request(RequestKind::SetSetting)),
            asks: Asks::Nothing,
        },
        Control {
            title: "Show Finished Turn",
            action: || Box::new(ShowFinishedTurn),
            keystroke: "secondary-alt-n",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Quit",
            action: || Box::new(Quit),
            keystroke: "secondary-q",
            place: Place::Anywhere,
            sends: None,
            asks: Asks::Nothing,
        },
        Control {
            title: "Quit and Stop Agents",
            action: || Box::new(QuitAndStopAgents),
            keystroke: "secondary-alt-q",
            place: Place::Anywhere,
            sends: Some(Sends::StopEverything),
            asks: Asks::Nothing,
        },
    ]
}

pub fn bind(controls: &[Control], cx: &mut App) {
    let bindings: Vec<KeyBinding> = controls
        .iter()
        .map(|control| {
            let context = control.context().map(|context| {
                KeyBindingContextPredicate::parse(context)
                    .unwrap_or_else(|error| {
                        panic!(
                            "the context {context:?} of {:?} is not a key context: {error}",
                            control.title
                        )
                    })
                    .into()
            });
            assert!(
                !control.keystroke.trim().is_empty(),
                "the key {:?} of {:?} is not a key: it is empty",
                control.keystroke,
                control.title
            );
            KeyBinding::load(&control.key(), (control.action)(), context, false, None, cx)
                .unwrap_or_else(|error| {
                    panic!(
                        "the key {:?} of {:?} is not a key: {error}",
                        control.keystroke, control.title
                    )
                })
        })
        .collect();
    cx.bind_keys(bindings);
}

#[cfg(test)]
mod tests {
    use super::{KEY_CONTEXT, PROGRAM_CONTEXT, Place, write_key};

    #[test]
    fn a_key_that_works_anywhere_has_no_context_where_it_is_a_cmd_key_and_the_frames_outside_a_program_elsewhere()
     {
        assert_eq!(Place::Anywhere.context(true), None);
        assert_eq!(
            Place::Anywhere.context(false),
            Some(format!("{KEY_CONTEXT} && !{PROGRAM_CONTEXT}").as_str())
        );
    }

    #[test]
    fn a_kept_key_has_the_context_of_the_frame_on_every_system() {
        for secondary_is_cmd in [true, false] {
            assert_eq!(Place::Kept.context(secondary_is_cmd), Some(KEY_CONTEXT));
        }
    }

    #[test]
    fn a_key_of_a_pane_has_the_context_of_the_pane_on_every_system() {
        for secondary_is_cmd in [true, false] {
            assert_eq!(Place::In("Pane").context(secondary_is_cmd), Some("Pane"));
        }
    }

    #[test]
    fn secondary_is_written_out_as_cmd_or_as_ctrl() {
        assert_eq!(write_key("secondary-alt-p", true), "cmd-alt-p");
        assert_eq!(write_key("secondary-alt-p", false), "ctrl-alt-p");
        assert_eq!(write_key("secondary-o", true), "cmd-o");
        assert_eq!(write_key("secondary-shift-b", false), "ctrl-shift-b");
    }

    #[test]
    fn a_key_with_no_secondary_stays_as_written() {
        for secondary_is_cmd in [true, false] {
            assert_eq!(write_key("ctrl-tab", secondary_is_cmd), "ctrl-tab");
            assert_eq!(
                write_key("ctrl-shift-tab", secondary_is_cmd),
                "ctrl-shift-tab"
            );
            assert_eq!(write_key("a", secondary_is_cmd), "a");
        }
    }

    #[test]
    fn every_keystroke_of_a_key_with_several_is_written_out() {
        assert_eq!(
            write_key("secondary-k secondary-shift-c", false),
            "ctrl-k ctrl-shift-c"
        );
        assert_eq!(write_key("secondary-k c", true), "cmd-k c");
    }

    #[test]
    fn the_word_secondary_is_taken_for_the_modifier_only_where_it_is_one() {
        // A key part is the last of its keystroke: it is never a modifier.
        assert_eq!(write_key("secondary--", true), "cmd--");
        assert_eq!(write_key("secondary", true), "secondary");
        assert_eq!(write_key("ctrl-secondary", false), "ctrl-secondary");
    }
}
