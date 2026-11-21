//! The settings screen: what the user changes inside the app (FR-124, FR-125, FR-126 and
//! the settings earlier slices left for it), drawn by the Frame in the place of the panes.
//!
//! # Where, and how it opens
//!
//! [`crate::OpenSettings`] (Cmd-comma on macOS, Ctrl-comma elsewhere; and the menu's
//! "Settings…" on macOS) opens it in the place of the panes, below the notices, the sidebar
//! still beside it: no dialog, no window of its own (`cx.has_pending_prompt()` stays false
//! and the app holds one window). The same key, or Escape ([`crate::CloseSettings`]), closes
//! it and gives the focus back to where it was. While it is open it has the focus and sets
//! the key context [`SETTINGS_CONTEXT`]; the Frame's keys still work. It is drawn inside
//! an element with the selector `settings-screen`, which scrolls with the wheel when the
//! rows are taller than the room.
//!
//! The screen holds no setting of its own: it reads the shared models and changes them,
//! and what it changes is in force as soon as the model tells its readers. No pane is
//! called: a pane reads what the screen set from the same models.
//!
//! # The rows, in order
//!
//! Each row is one [`SettingRow`], with a title ([`TITLES`]) and its values, each drawn as a
//! control under the selector `settings-value-<row>-<value>:<label>`, the chosen one also
//! under `settings-chosen-<row>`: `<row>` is the row's index in [`SettingsScreen::rows`] as
//! they are drawn (so it depends on how many rows of extensions and of repositories come
//! before; a test finds a row's index in `rows()`, never writes it), `<value>` the value's
//! index in [`RowShown::values`], both from 0, `<label>` the words drawn. A click on a value chooses it; with the screen focused, `j` and
//! `k` move the cursor (`settings-cursor` on its row) to the next and the previous row, `l`
//! and `h` choose the next and the previous value of the row under the cursor, stopping at
//! the last and the first, `enter` acts on a row that acts ([`SettingRow::Base`]), and
//! `shift-c` cleans up the recorded states ([`crate::CleanUpStates`]).
//!
//! | Row | Values (labels) | Model read | What a choice does |
//! | --- | --- | --- | --- |
//! | [`SettingRow::TurnEnd`] | [`WHAT_CHANGED`], [`KEEP_SELECTION`] | `SettingList::turn_end` | `SettingList::set(Setting::TurnEnd)`; the review pane reads it at the next turn's end |
//! | [`SettingRow::StopAfter`] | each of [`STOP_AFTER_MINUTES`], "N min" | `SettingList::stop_after` | `SettingList::set(Setting::StopAfter)`; the model `Navigation` tells the Navigator `Ask::StopAfter` at once |
//! | [`SettingRow::Theme`] | [`SYSTEM`], then the name of every theme of `Looks::themes` | `Looks::chosen` | `SettingList::set(Setting::Theme)`; `Looks` applies it at once |
//! | [`SettingRow::Base`] | the base of the workspace on screen (`WorkspaceChanges`'s comparison), one value, or [`NO_WORKSPACE`] | `WorkspaceChanges` | `enter` or a click: the Frame's field for the base opens, as [`crate::SetBase`] opens it; Enter there sends `SetBase` |
//! | [`SettingRow::KeepStates`] | each of [`KEEP_STATES_DAYS`], "N days" ("1 day") | `SettingList::keep_states` | `SettingList::set(Setting::KeepStates)` |
//! | [`SettingRow::TerminalPlace`] | [`RIGHT`], [`BELOW`], [`LEFT`] | `SettingList::terminal_place` | `SettingList::set(Setting::TerminalPlace)`; the Frame lays the panes out at once |
//! | [`SettingRow::OptionKey`] | [`BY_LAYOUT`], [`META`], [`CHARACTERS`] | `SettingList::option_key` | `SettingList::set(Setting::OptionKey)`; the terminal pane reads it at the next key |
//! | [`SettingRow::Extension`], one for each question of `ConsentList` | [`INSTALL`], [`DONT_INSTALL`] | `ConsentList` | `ConsentList::answer`: `AnswerConsent` of that Adapter |
//! | [`SettingRow::Servers`], one for each repository answered (`SettingList::servers_answers`) | [`RUN_SERVERS`], [`KEEP_SERVERS_OUT`] | `SettingList` | `SettingList::set(Setting::ServersInRepository)`; the model `Navigation` hands the Navigator the answer, and a yes searches again |
//! | [`SettingRow::States`] | the size of each workspace's states and the total, and the control [`CLEAN_UP`] | `RecordedStates` | the control, `shift-c` or a click: `RecordedStates::clean_up`; while it runs the control reads [`CLEANING`] |
//!
//! A value that is not one of a row's values (a stop time of 7 minutes another Window set)
//! is shown as the row's value with no value chosen, and `l`, `h` move from the value
//! above, or below, it.
//!
//! The sizes are asked for when the screen opens (`RecordedStates::refresh`), and shown as
//! `<workspace's name>: <size>` (selector `settings-size-<workspace id>`) for each workspace
//! of the answer that the Window lists; then, when the total is larger than the sum of
//! those, `<OTHER_STATES>: <the difference>` (selector `settings-size-other`,
//! [`OTHER_STATES`]'s words): the stores of worktrees that are gone or that this Window does
//! not list; then `Total: <size>` (`settings-size-total`); each size in the words of
//! [`size_words`].
//!
//! The labels are the constants of this module, which the drawing and the tests read from
//! this one place: a control's drawn words and its selector are made from the same constant.

use gpui_kit::*;
use yaar_link::RepositoryId;

use crate::models::Models;

/// The key context the screen sets while it has the focus.
pub const SETTINGS_CONTEXT: &str = "Settings";

/// The title of each row, in the order of [`SettingRow`]'s first seven kinds, then the
/// title of the rows of an extension, of a repository's servers and of the states.
pub const TITLES: [&str; 10] = [
    "When a turn ends",
    "Language servers off screen stop after",
    "Theme",
    "Base of this worktree",
    "Keep recorded states nothing refers to for",
    "Terminal pane",
    "Option key in a terminal (macOS)",
    "Extension of the agent",
    "Language servers inside the repository",
    "Recorded states",
];

/// A turn's end selects what changed since the last review.
pub const WHAT_CHANGED: &str = "Select what changed";
/// A turn's end keeps the selection and offers what changed.
pub const KEEP_SELECTION: &str = "Keep the selection";
/// The theme follows the system.
pub const SYSTEM: &str = "As the system";
/// The base row when no workspace is on screen.
pub const NO_WORKSPACE: &str = "No worktree on screen";
/// The terminal pane to the right.
pub const RIGHT: &str = "Right";
/// Below.
pub const BELOW: &str = "Below";
/// To the left.
pub const LEFT: &str = "Left";
/// Option by the keyboard layout.
pub const BY_LAYOUT: &str = "By the keyboard layout";
/// Option as Meta.
pub const META: &str = "Meta";
/// Option types characters.
pub const CHARACTERS: &str = "Characters";
/// Install an agent's extension.
pub const INSTALL: &str = "Install";
/// Do not install it.
pub const DONT_INSTALL: &str = "Don't install";
/// Run the language servers inside a repository.
pub const RUN_SERVERS: &str = "Run them";
/// Keep them out.
pub const KEEP_SERVERS_OUT: &str = "Keep them out";
/// The clean-up's control.
pub const CLEAN_UP: &str = "Clean up now";
/// The clean-up's control while it runs.
pub const CLEANING: &str = "Cleaning up…";
/// The line of the states of workspaces the Window does not list.
pub const OTHER_STATES: &str = "Other recorded states";

/// The stop times offered, in minutes.
pub const STOP_AFTER_MINUTES: [u32; 7] = [1, 2, 5, 10, 15, 30, 60];

/// The times recorded states are kept, in days.
pub const KEEP_STATES_DAYS: [u32; 8] = [1, 7, 14, 30, 60, 90, 180, 365];

/// One row of the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingRow {
    /// What a turn end selects.
    TurnEnd,
    /// The time before the servers of a workspace off screen stop.
    StopAfter,
    /// The theme.
    Theme,
    /// The base of the workspace on screen.
    Base,
    /// The time a state nothing refers to is kept.
    KeepStates,
    /// The place of the terminal pane.
    TerminalPlace,
    /// What Option does in a terminal.
    OptionKey,
    /// The question for leave of an Adapter, by its name.
    Extension(String),
    /// The answer on the servers inside a repository.
    Servers(RepositoryId),
    /// The sizes of the recorded states, and the clean-up.
    States,
}

/// A row as the screen draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowShown {
    /// The row.
    pub row: SettingRow,
    /// Its title, as drawn.
    pub title: String,
    /// Its values' labels, in order.
    pub values: Vec<String>,
    /// The index of the chosen value; none when the setting's value is none of them.
    pub chosen: Option<usize>,
}

/// A size of the disk as the screen writes it. Under 1,024 bytes: "N bytes". Otherwise the
/// largest of KB, MB, GB and TB (1,024 bytes, then each 1,024 of the one before) of which
/// there is at least one; the number in it rounded to one decimal, half away from zero;
/// when that is below 10, written with its decimal, a decimal of 0 dropped; else rounded to
/// a whole number. So: 0 "0 bytes", 1,023 "1023 bytes", 1,024 "1 KB", 2,560 "2.5 KB", 4,096
/// "4 KB", 2,621,440 "2.5 MB", 2,629,632 "2.5 MB", 10,485,760 "10 MB", 3,221,225,472
/// "3 GB".
pub fn size_words(bytes: u64) -> String {
    let _ = bytes;
    todo!("a size in words, as the doc says")
}

/// The keys of the screen, a table of their own as a pane's: the Window program puts them
/// after the Frame's. Each works in [`SETTINGS_CONTEXT`] only: `escape` closes
/// ([`crate::CloseSettings`]), `j` and `k` move the cursor ([`crate::NextSetting`],
/// [`crate::PreviousSetting`]), `l` and `h` choose a value and send `SetSetting`
/// ([`crate::NextValue`], [`crate::PreviousValue`]), `enter` acts on the row
/// ([`crate::OpenSetting`]), `shift-c` cleans up and sends `CleanUpStates`
/// ([`crate::CleanUpStates`]); `PLAN.md`'s key table of slice 9 holds the titles. Empty
/// until the screen handles them.
pub fn controls() -> Vec<crate::Control> {
    Vec::new()
}

/// The settings screen.
pub struct SettingsScreen {
    models: Models,
    focus: FocusHandle,
}

impl SettingsScreen {
    /// The screen of one window, over its shared models. It asks for the sizes of the states
    /// when it is made, which is when it opens.
    pub fn new(models: Models, cx: &mut Context<Self>) -> SettingsScreen {
        SettingsScreen {
            models,
            focus: cx.focus_handle(),
        }
    }

    /// Where the keys go while it is open.
    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// The rows, as the module doc's table says, from the models as they are now.
    pub fn rows(&self, cx: &App) -> Vec<RowShown> {
        let _ = (&self.models, cx);
        todo!("the rows, as the module doc says")
    }

    /// The index of the row under the cursor.
    pub fn cursor(&self) -> usize {
        todo!("the cursor")
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().debug_selector(|| "settings-screen".to_owned())
    }
}
