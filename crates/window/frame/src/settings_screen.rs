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
//! the key context [`SETTINGS_CONTEXT`], except while a field of the screen is open; the
//! Frame's keys still work. It is drawn inside
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
//! control under the selector `settings-value-<row>-<value>:<label>`, an element around the
//! control with the control's own bounds, and the chosen control itself, the element that
//! has the colours of a chosen value, under `settings-chosen-<row>`: `<row>` is the row's index in [`SettingsScreen::rows`] as
//! they are drawn (so it depends on how many rows of extensions and of repositories come
//! before; a test finds a row's index in `rows()`, never writes it), `<value>` the value's
//! index in [`RowShown::values`], both from 0, `<label>` the words drawn. A click on a value
//! chooses it, and puts the cursor on its row. With the screen focused, `j` and `k` move the
//! cursor (`settings-cursor` on its row) to the next and the previous row, stopping at the
//! last and the first, and scroll the row into the room; `l` and `h` choose the next and the
//! previous value of the row under the cursor, stopping at the last and the first (a value
//! chosen already is not set again by them; a click always acts), `enter` acts on a row that
//! acts ([`SettingRow::Base`], [`SettingRow::WorktreesFolder`]), and `shift-c` cleans up the
//! recorded states ([`crate::CleanUpStates`]). `l` and `h` do nothing on the rows that act on
//! their one value (the base, a folder, the states).
//!
//! | Row | Values (labels) | Model read | What a choice does |
//! | --- | --- | --- | --- |
//! | [`SettingRow::TurnEnd`] | [`WHAT_CHANGED`], [`KEEP_SELECTION`] | `SettingList::turn_end` | `SettingList::set(Setting::TurnEnd)`; the review pane reads it at the next turn's end |
//! | [`SettingRow::StopAfter`] | each of [`STOP_AFTER_MINUTES`], "N min" | `SettingList::stop_after` | `SettingList::set(Setting::StopAfter)`; the model `Navigation` tells the Navigator `Ask::StopAfter` at once |
//! | [`SettingRow::Theme`] | [`SYSTEM`], then the name of every theme of `Looks::themes` | `Looks::chosen` | `SettingList::set(Setting::Theme)`; `Looks` applies it at once |
//! | [`SettingRow::Base`] | the base of the workspace on screen (`WorkspaceChanges`'s comparison), one value; [`NO_BASE`] where its changes name none; [`NO_WORKSPACE`] where none is on screen | `WorkspaceChanges` | `enter` or a click: the Frame's field for the base opens, as [`crate::SetBase`] opens it; Enter there sends `SetBase` |
//! | [`SettingRow::KeepStates`] | each of [`KEEP_STATES_DAYS`], "N days" ("1 day") | `SettingList::keep_states` | `SettingList::set(Setting::KeepStates)` |
//! | [`SettingRow::TerminalPlace`] | [`RIGHT`], [`BELOW`], [`LEFT`] | `SettingList::terminal_place` | `SettingList::set(Setting::TerminalPlace)`; the Frame lays the panes out at once |
//! | [`SettingRow::OptionKey`] | [`BY_LAYOUT`], [`META`], [`CHARACTERS`] | `SettingList::option_key` | `SettingList::set(Setting::OptionKey)`; the terminal pane reads it at the next key |
//! | [`SettingRow::Extension`], one for each Adapter of `ConsentList::all`, answered or not, titled `TITLES[7]`, `": "` and the Adapter's name | [`INSTALL`], [`DONT_INSTALL`]; none chosen until answered | `ConsentList` | `ConsentList::answer`: `AnswerConsent` of that Adapter; the Adapter's own question is drawn under the row (`settings-question-<row>`) and what went wrong in the colour of a failure (`settings-problem-<row>`) |
//! | [`SettingRow::Servers`], one for each repository answered (`SettingList::servers_answers`), titled `TITLES[8]`, `": "` and the repository's name | [`RUN_SERVERS`], [`KEEP_SERVERS_OUT`] | `SettingList` | `SettingList::set(Setting::ServersInRepository)`; the model `Navigation` hands the Navigator the answer, and a yes searches again |
//! | [`SettingRow::WorktreesFolder`], one for each repository the Window lists (`WorkspaceList::repositories`, in its order) | one value: the folder set (`SettingList::worktrees_folder`), written as its path, or [`DEFAULT_FOLDER`] | `SettingList` | `enter` or a click: the screen's own field for the folder opens, as the base row opens the Frame's; Enter there sends `SetSetting(WorktreesFolder)` (below) |
//! | [`SettingRow::States`] | the size of each workspace's states and the total, and the control [`CLEAN_UP`] | `RecordedStates` | the control, `shift-c` or a click: `RecordedStates::clean_up`; while it runs the control reads [`CLEANING`] |
//!
//! A value that is not one of a row's values (a stop time of 7 minutes another Window set)
//! is shown as the row's value with no value chosen, and `l`, `h` move from the value
//! above, or below, it.
//!
//! At each opening the screen asks the Keeper two questions, in this order: `GetStateSizes`
//! (`RecordedStates::refresh`) and `ListConsents` (`ConsentList::refresh`), so that an
//! extension answered before this Window started, of which the Keeper tells a Window nothing,
//! has its row with its answer chosen. A refusal of the second leaves the rows as they were.
//!
//! The sizes are asked for when the screen opens, and shown as
//! `<workspace's name>: <size>` (selector `settings-size-<workspace id>`) for each workspace
//! of the answer that the Window lists; then, when the total is larger than the sum of
//! those, `<OTHER_STATES>: <the difference>` (selector `settings-size-other`,
//! [`OTHER_STATES`]'s words): the stores of worktrees that are gone or that this Window does
//! not list; then `<TOTAL_STATES>: <size>` (`settings-size-total`); each size in the words of
//! [`size_words`]. Each line is drawn as an element with the selector named, which holds an
//! element with the same selector, a colon and the words drawn, so that a test reads the
//! words as it does a control's. They lie under the control of the clean-up. A refusal of the
//! question for the sizes is drawn there in the colour of a failure (`settings-size-problem`)
//! and the control stays. A refusal of an answer to a question for leave is drawn above the
//! rows (`settings-notice`) until the next answer.
//!
//! The screen scrolls with the wheel and keeps the row under the cursor in the room.
//!
//! # The folder for a repository's worktrees
//!
//! A row [`SettingRow::WorktreesFolder`] is titled [`TITLES`]`[10]`, `": "`, and the
//! repository's name as the sidebar shows it (`Repository::name`); its one value is always
//! chosen (`chosen` is `Some(0)`). `enter` on it, or a click on its value, opens a one-line
//! field (the kit's `InputState` and `Input`, as the Frame's field for the base), drawn
//! under the row with the selector `settings-folder-field`, holding the folder set or
//! nothing, with [`FOLDER_PLACEHOLDER`] as its placeholder, and takes the focus; one field
//! at a time, for one row. In it:
//! - Enter with text (trimmed of blanks at both ends) sends
//!   `SettingList::set_answered(Setting::WorktreesFolder { repository, folder: Some(<the
//!   text as a path>) })`; Enter with nothing sends `folder: None`, the way back to
//!   [`DEFAULT_FOLDER`]. The field stays open, its text kept, until the answer: `Ok` closes
//!   it and gives the focus back to the screen; `Err` keeps it open and draws the sentence
//!   under it with the selector `settings-folder-refused` (and its words), until the next
//!   Enter or the field closes. The row's value is what `SettingList` holds meanwhile: the
//!   folder at once, the kept one again after a refusal.
//! - Escape closes it, nothing sent, the focus back to the screen.
//!
//! A click on another row's value while a field is open closes the field first: what was
//! typed is given up, and one field is never open for two rows. While the field is open the
//! screen sets no key context ([`controls`]): the letters of its keys are typed there, and
//! Enter and Escape are the field's.
//!
//! The screen judges no folder: whether one is taken is the Keeper's to say.
//!
//! The labels are the constants of this module, which the drawing and the tests read from
//! this one place: a control's drawn words and its selector are made from the same constant.

use std::path::PathBuf;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::*;
use yaar_link::{
    Changes, Comparison, ConsentAnswer, OptionKey, RepositoryId, RequestKind, Setting,
    TerminalPlace, ThemeChoice, TurnEndSelects,
};

use crate::controls::{
    Asks, CleanUpStates, CloseSettings, Control, NextSetting, NextValue, OpenSetting, Place,
    PreviousSetting, PreviousValue, Sends, SetBase,
};
use crate::link_model::AskError;
use crate::models::Models;
use crate::sidebar::workspace_name;

/// The key context the screen sets while it has the focus.
pub const SETTINGS_CONTEXT: &str = "Settings";

/// The title of each row, in the order of [`SettingRow`]'s first seven kinds, then the
/// title of the rows of an extension, of a repository's servers and of the states, then
/// that of a repository's folder for worktrees (drawn before the states' row: the table of
/// the module doc gives the order rows are drawn in).
pub const TITLES: [&str; 11] = [
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
    "New worktrees go in",
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
/// The folder row's value when the user named no folder for the repository.
pub const DEFAULT_FOLDER: &str = "Beside the repository, where it keeps its worktrees";
/// The placeholder of the field for a repository's folder for worktrees.
pub const FOLDER_PLACEHOLDER: &str = "An absolute folder; empty for beside the repository";

/// The base row's value when a workspace is on screen and no base is known for it.
pub const NO_BASE: &str = "None found: set one";
/// The words that open the line of the total of the recorded states.
pub const TOTAL_STATES: &str = "Total";

/// The stop times offered, in minutes.
pub const STOP_AFTER_MINUTES: [u32; 7] = [1, 2, 5, 10, 15, 30, 60];

/// The times recorded states are kept, in days.
pub const KEEP_STATES_DAYS: [u32; 8] = [1, 7, 14, 30, 60, 90, 180, 365];

/// The words of a stop time on its row: `"N min"`.
pub fn stop_after_label(minutes: u32) -> String {
    format!("{minutes} min")
}

/// The words of a kept time on its row: `"N days"`, and `"1 day"` for one.
pub fn keep_states_label(days: u32) -> String {
    if days == 1 {
        "1 day".to_owned()
    } else {
        format!("{days} days")
    }
}

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
    /// The folder the worktrees the app creates for a repository go in.
    WorktreesFolder(RepositoryId),
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
    const KB: u128 = 1_024;
    if bytes < 1_024 {
        return format!("{bytes} bytes");
    }
    let bytes = u128::from(bytes);
    // The largest unit of which there is one; past the last, the number grows.
    let (unit, name) = [(KB.pow(4), "TB"), (KB.pow(3), "GB"), (KB.pow(2), "MB")]
        .into_iter()
        .find(|(unit, _)| bytes >= *unit)
        .unwrap_or((KB, "KB"));
    // Whole numbers all the way, so that no float decides a half: a unit is even, and a
    // half of a tenth or of a whole is exact.
    let tenths = (bytes * 10 + unit / 2) / unit;
    let number = if tenths < 100 {
        match (tenths / 10, tenths % 10) {
            (whole, 0) => whole.to_string(),
            (whole, tenth) => format!("{whole}.{tenth}"),
        }
    } else {
        ((bytes + unit / 2) / unit).to_string()
    };
    format!("{number} {name}")
}

/// The keys of the screen, a table of their own as a pane's: the Window program puts them
/// after the Frame's. Each works in [`SETTINGS_CONTEXT`] only: `escape` closes
/// ([`crate::CloseSettings`]), `j` and `k` move the cursor ([`crate::NextSetting`],
/// [`crate::PreviousSetting`]), `l` and `h` choose a value and send `SetSetting`
/// ([`crate::NextValue`], [`crate::PreviousValue`]), `enter` acts on the row
/// ([`crate::OpenSetting`]), `shift-c` cleans up and sends `CleanUpStates`
/// ([`crate::CleanUpStates`]); `PLAN.md`'s key table of slice 9 holds the titles.
///
/// While a field of the screen is open the screen sets no key context, so that the field
/// takes every key as text: `j`, `k`, `l`, `h` and `shift-c` type their letters there.
pub fn controls() -> Vec<Control> {
    let row = |title: &'static str,
               action: fn() -> Box<dyn Action>,
               keystroke: &'static str,
               sends: Option<Sends>| Control {
        title,
        action,
        keystroke,
        place: Place::In(SETTINGS_CONTEXT),
        sends,
        asks: Asks::Nothing,
    };
    let sets = Some(Sends::Request(RequestKind::SetSetting));
    vec![
        row("Close Settings", || Box::new(CloseSettings), "escape", None),
        row("Next Setting", || Box::new(NextSetting), "j", None),
        row("Previous Setting", || Box::new(PreviousSetting), "k", None),
        row("Next Value", || Box::new(NextValue), "l", sets),
        row("Previous Value", || Box::new(PreviousValue), "h", sets),
        row("Open Setting", || Box::new(OpenSetting), "enter", None),
        row(
            "Clean Up Recorded States",
            || Box::new(CleanUpStates),
            "shift-c",
            Some(Sends::Request(RequestKind::CleanUpStates)),
        ),
    ]
}

/// What choosing one value of a row does.
#[derive(Debug, Clone)]
enum Act {
    /// Sets a setting of the app.
    Set(Setting),
    /// Answers the question for leave of one Adapter.
    Answer(String, ConsentAnswer),
    /// Opens the Frame's field for the base.
    OpenBase,
    /// Opens the screen's field for a repository's folder.
    OpenFolder(RepositoryId),
    /// Cleans up the recorded states.
    CleanUp,
}

/// A row with what each of its values does.
struct Built {
    shown: RowShown,
    acts: Vec<Act>,
}

/// The one-line field for a repository's folder, from its opening to its closing.
struct FolderField {
    repository: RepositoryId,
    input: Entity<InputState>,
    /// Why the Keeper refused the last folder sent, until the next Enter.
    refused: Option<String>,
    /// The folder sent and not answered yet; dropping it only stops the wait.
    sending: Option<Task<()>>,
    _enter: Subscription,
}

/// The settings screen.
pub struct SettingsScreen {
    models: Models,
    focus: FocusHandle,
    cursor: usize,
    scroll: ScrollHandle,
    field: Option<FolderField>,
    /// Why the Keeper refused the last answer to a question for leave.
    notice: Option<String>,
    /// The question for the list of the extensions, asked when the screen opened.
    _listing: Task<Result<(), AskError>>,
    _watching: Vec<Subscription>,
}

/// Whether a row's values are choices, so that `l` and `h` move among them. The rows that
/// act on their one value (the base, a folder, the states) are not.
fn is_choice(row: &SettingRow) -> bool {
    !matches!(
        row,
        SettingRow::Base | SettingRow::WorktreesFolder(_) | SettingRow::States
    )
}

impl SettingsScreen {
    /// The screen of one window, over its shared models. It asks for the sizes of the states
    /// and then for the list of the extensions when it is made, which is when it opens: the
    /// Keeper tells a Window an Adapter only while a question waits or something went wrong,
    /// so an answer given before this Window started is in the list only because it asks.
    pub fn new(models: Models, cx: &mut Context<Self>) -> SettingsScreen {
        models.states.update(cx, |states, cx| states.refresh(cx));
        let listing = models
            .consents
            .update(cx, |consents, cx| consents.refresh(cx));
        // The screen holds nothing it shows: each model that does, when it changes, has the
        // screen drawn again, with the cursor still on a row there is.
        let keep = |screen: &mut Self, cx: &mut Context<Self>| {
            let rows = screen.built(cx).len();
            screen.cursor = screen.clamped_cursor(rows);
            cx.notify();
        };
        let watching = vec![
            cx.observe(&models.settings, move |screen, _, cx| keep(screen, cx)),
            cx.observe(&models.workspaces, move |screen, _, cx| keep(screen, cx)),
            cx.observe(&models.changes, move |screen, _, cx| keep(screen, cx)),
            cx.observe(&models.states, move |screen, _, cx| keep(screen, cx)),
            cx.observe(&models.consents, move |screen, _, cx| keep(screen, cx)),
        ];
        SettingsScreen {
            models,
            focus: cx.focus_handle(),
            cursor: 0,
            scroll: ScrollHandle::new(),
            field: None,
            notice: None,
            _listing: listing,
            _watching: watching,
        }
    }

    /// Where the keys go while it is open.
    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// The rows, as the module doc's table says, from the models as they are now.
    pub fn rows(&self, cx: &App) -> Vec<RowShown> {
        self.built(cx).into_iter().map(|row| row.shown).collect()
    }

    /// The index of the row under the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The name of a repository as the sidebar shows it; its id where the Window lists none.
    fn repository_name(&self, repository: &RepositoryId, cx: &App) -> String {
        self.models
            .workspaces
            .read(cx)
            .repositories()
            .iter()
            .find(|listed| listed.id == *repository)
            .map_or_else(
                || repository.as_str().to_owned(),
                |listed| listed.name.clone(),
            )
    }

    /// The base of the workspace on screen, by the comparison its changes are counted in.
    fn base_name(changes: &Changes) -> Option<&str> {
        match &changes.comparison {
            Comparison::SinceBase { base }
            | Comparison::NotOnUpstream { base, .. }
            | Comparison::OnNoRemoteBranch { base }
            | Comparison::NothingToCompare {
                base: Some(base), ..
            } => Some(base.name.as_str()),
            Comparison::NothingToCompare { base: None, .. } => None,
        }
    }

    /// Every row with what each of its values does, from the models as they are now.
    fn built(&self, cx: &App) -> Vec<Built> {
        let settings = self.models.settings.read(cx);
        let mut rows = Vec::new();
        let mut add = |row: SettingRow, title: String, values: Vec<(String, Act)>, chosen| {
            let (values, acts) = values.into_iter().unzip();
            rows.push(Built {
                shown: RowShown {
                    row,
                    title,
                    values,
                    chosen,
                },
                acts,
            });
        };
        let choice = |label: &str, setting: Setting| (label.to_owned(), Act::Set(setting));

        add(
            SettingRow::TurnEnd,
            TITLES[0].to_owned(),
            vec![
                choice(WHAT_CHANGED, Setting::TurnEnd(TurnEndSelects::WhatChanged)),
                choice(
                    KEEP_SELECTION,
                    Setting::TurnEnd(TurnEndSelects::KeepSelection),
                ),
            ],
            Some(match settings.turn_end() {
                TurnEndSelects::WhatChanged => 0,
                TurnEndSelects::KeepSelection => 1,
            }),
        );

        let stop = settings.stop_after().as_secs();
        add(
            SettingRow::StopAfter,
            TITLES[1].to_owned(),
            STOP_AFTER_MINUTES
                .iter()
                .map(|minutes| {
                    choice(
                        &stop_after_label(*minutes),
                        Setting::StopAfter {
                            seconds: minutes * 60,
                        },
                    )
                })
                .collect(),
            STOP_AFTER_MINUTES
                .iter()
                .position(|minutes| u64::from(minutes * 60) == stop),
        );

        let themes = self.models.looks.read(cx).themes();
        let mut theme_values = vec![choice(SYSTEM, Setting::Theme(ThemeChoice::System))];
        theme_values.extend(themes.all().iter().map(|definition| {
            choice(
                &definition.name,
                Setting::Theme(ThemeChoice::Named(definition.name.clone())),
            )
        }));
        // A name no theme has is the system's, as `Looks` applies it.
        let theme_chosen = match settings.theme() {
            ThemeChoice::Named(name) => themes
                .all()
                .iter()
                .position(|definition| themes.named(&name).is_some_and(|d| d == definition))
                .map_or(0, |at| at + 1),
            ThemeChoice::System => 0,
        };
        add(
            SettingRow::Theme,
            TITLES[2].to_owned(),
            theme_values,
            Some(theme_chosen),
        );

        let base = if self.models.workspaces.read(cx).selected().is_none() {
            None
        } else {
            Some(
                self.models
                    .changes
                    .read(cx)
                    .changes()
                    .and_then(Self::base_name)
                    .map(str::to_owned),
            )
        };
        let (base_label, base_chosen) = match base {
            None => (NO_WORKSPACE.to_owned(), None),
            Some(Some(name)) => (name, Some(0)),
            Some(None) => (NO_BASE.to_owned(), None),
        };
        add(
            SettingRow::Base,
            TITLES[3].to_owned(),
            vec![(base_label, Act::OpenBase)],
            base_chosen,
        );

        let keep = settings.keep_states();
        add(
            SettingRow::KeepStates,
            TITLES[4].to_owned(),
            KEEP_STATES_DAYS
                .iter()
                .map(|days| {
                    choice(
                        &keep_states_label(*days),
                        Setting::KeepStates { days: *days },
                    )
                })
                .collect(),
            KEEP_STATES_DAYS.iter().position(|days| *days == keep),
        );

        add(
            SettingRow::TerminalPlace,
            TITLES[5].to_owned(),
            vec![
                choice(RIGHT, Setting::TerminalPlace(TerminalPlace::Right)),
                choice(BELOW, Setting::TerminalPlace(TerminalPlace::Below)),
                choice(LEFT, Setting::TerminalPlace(TerminalPlace::Left)),
            ],
            Some(match settings.terminal_place() {
                TerminalPlace::Right => 0,
                TerminalPlace::Below => 1,
                TerminalPlace::Left => 2,
            }),
        );

        add(
            SettingRow::OptionKey,
            TITLES[6].to_owned(),
            vec![
                choice(BY_LAYOUT, Setting::OptionKey(OptionKey::ByLayout)),
                choice(META, Setting::OptionKey(OptionKey::Meta)),
                choice(CHARACTERS, Setting::OptionKey(OptionKey::Characters)),
            ],
            Some(match settings.option_key() {
                OptionKey::ByLayout => 0,
                OptionKey::Meta => 1,
                OptionKey::Characters => 2,
            }),
        );

        for consent in self.models.consents.read(cx).all() {
            let agent = consent.agent.clone();
            add(
                SettingRow::Extension(agent.clone()),
                format!("{}: {agent}", TITLES[7]),
                vec![
                    (
                        INSTALL.to_owned(),
                        Act::Answer(agent.clone(), ConsentAnswer::Yes),
                    ),
                    (
                        DONT_INSTALL.to_owned(),
                        Act::Answer(agent.clone(), ConsentAnswer::No),
                    ),
                ],
                match consent.answer {
                    ConsentAnswer::Yes => Some(0),
                    ConsentAnswer::No => Some(1),
                    ConsentAnswer::NotAsked => None,
                },
            );
        }

        for (repository, allowed) in settings.servers_answers() {
            let setting = |allowed| Setting::ServersInRepository {
                repository: repository.clone(),
                allowed,
            };
            add(
                SettingRow::Servers(repository.clone()),
                format!("{}: {}", TITLES[8], self.repository_name(&repository, cx)),
                vec![
                    choice(RUN_SERVERS, setting(true)),
                    choice(KEEP_SERVERS_OUT, setting(false)),
                ],
                Some(if allowed { 0 } else { 1 }),
            );
        }

        for listed in self.models.workspaces.read(cx).repositories() {
            let folder = settings.worktrees_folder(&listed.id).map_or_else(
                || DEFAULT_FOLDER.to_owned(),
                |f| f.to_string_lossy().into_owned(),
            );
            add(
                SettingRow::WorktreesFolder(listed.id.clone()),
                format!("{}: {}", TITLES[10], listed.name),
                vec![(folder, Act::OpenFolder(listed.id.clone()))],
                Some(0),
            );
        }

        let cleaning = self.models.states.read(cx).cleaning();
        add(
            SettingRow::States,
            TITLES[9].to_owned(),
            vec![(
                if cleaning { CLEANING } else { CLEAN_UP }.to_owned(),
                Act::CleanUp,
            )],
            None,
        );
        rows
    }

    /// The row under the cursor, with the cursor held inside the rows there are now.
    fn clamped_cursor(&self, rows: usize) -> usize {
        self.cursor.min(rows.saturating_sub(1))
    }

    /// Moves the cursor `by` rows, stopping at the first and the last, and scrolls its row
    /// into the room.
    fn move_cursor(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.built(cx).len();
        let at = self.clamped_cursor(rows);
        self.cursor = if forward {
            (at + 1).min(rows.saturating_sub(1))
        } else {
            at.saturating_sub(1)
        };
        self.scroll.scroll_to_item(self.cursor);
        cx.notify();
    }

    fn next_setting(&mut self, _: &NextSetting, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(true, cx);
    }

    fn previous_setting(&mut self, _: &PreviousSetting, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(false, cx);
    }

    /// The value `l` (`forward`) or `h` chooses in a row: the next or the previous of the
    /// chosen one, none at the ends. Where no value is chosen: for a stop time or a kept
    /// time that is none of the listed, the nearest listed above or below it; otherwise the
    /// first value for `l` and the last for `h`.
    fn step(&self, row: &Built, forward: bool, cx: &App) -> Option<usize> {
        if !is_choice(&row.shown.row) {
            return None;
        }
        let count = row.acts.len();
        if let Some(chosen) = row.shown.chosen {
            return if forward {
                (chosen + 1 < count).then_some(chosen + 1)
            } else {
                chosen.checked_sub(1)
            };
        }
        let settings = self.models.settings.read(cx);
        let nearest = |listed: Vec<u64>, now: u64| {
            if forward {
                listed.iter().position(|value| *value > now)
            } else {
                listed.iter().rposition(|value| *value < now)
            }
        };
        match row.shown.row {
            SettingRow::StopAfter => nearest(
                STOP_AFTER_MINUTES
                    .iter()
                    .map(|minutes| u64::from(*minutes) * 60)
                    .collect(),
                settings.stop_after().as_secs(),
            ),
            SettingRow::KeepStates => nearest(
                KEEP_STATES_DAYS
                    .iter()
                    .map(|days| u64::from(*days))
                    .collect(),
                u64::from(settings.keep_states()),
            ),
            _ => Some(if forward { 0 } else { count - 1 }),
        }
    }

    fn move_value(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.built(cx);
        let at = self.clamped_cursor(rows.len());
        let Some(row) = rows.get(at) else {
            return;
        };
        if let Some(value) = self.step(row, forward, cx) {
            self.choose(at, value, window, cx);
        }
    }

    fn next_value(&mut self, _: &NextValue, window: &mut Window, cx: &mut Context<Self>) {
        self.move_value(true, window, cx);
    }

    fn previous_value(&mut self, _: &PreviousValue, window: &mut Window, cx: &mut Context<Self>) {
        self.move_value(false, window, cx);
    }

    /// `enter`: acts on the row under the cursor where the row acts: the base and a folder.
    fn open_setting(&mut self, _: &OpenSetting, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.built(cx);
        let at = self.clamped_cursor(rows.len());
        let Some(row) = rows.get(at) else {
            return;
        };
        if matches!(
            row.shown.row,
            SettingRow::Base | SettingRow::WorktreesFolder(_)
        ) {
            self.choose(at, 0, window, cx);
        }
    }

    fn clean_up(&mut self, _: &CleanUpStates, _: &mut Window, cx: &mut Context<Self>) {
        self.models
            .states
            .update(cx, |states, cx| states.clean_up(cx));
    }

    /// Chooses value `value` of row `row`: what the module doc's table says of the row. The
    /// cursor goes to the row, and the screen has the focus afterwards, unless the choice
    /// opened a field.
    fn choose(&mut self, row: usize, value: usize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.built(cx);
        let Some(act) = rows
            .get(row)
            .and_then(|built| built.acts.get(value))
            .cloned()
        else {
            return;
        };
        self.cursor = row;
        self.scroll.scroll_to_item(row);
        if let Act::OpenFolder(repository) = act {
            self.open_field(repository, window, cx);
            return;
        }
        // Any other choice leaves a field that was open: it keeps nothing.
        self.field = None;
        self.focus.focus(window, cx);
        match act {
            Act::Set(setting) => self
                .models
                .settings
                .update(cx, |settings, cx| settings.set(setting, cx)),
            Act::Answer(agent, answer) => {
                let task = self
                    .models
                    .consents
                    .update(cx, |consents, cx| consents.answer(agent, answer, cx));
                // Detached: the task takes the list the Keeper answers with.
                cx.spawn(async move |screen, cx| {
                    let outcome = task.await;
                    screen
                        .update(cx, |screen, cx| {
                            screen.notice = outcome.err().map(|error| error.to_string());
                            cx.notify();
                        })
                        .ok();
                })
                .detach();
                self.notice = None;
            }
            Act::OpenBase => window.dispatch_action(Box::new(SetBase), cx),
            Act::CleanUp => self
                .models
                .states
                .update(cx, |states, cx| states.clean_up(cx)),
            Act::OpenFolder(_) => {}
        }
        cx.notify();
    }

    /// Opens the field for `repository`'s folder, holding the folder set, with the focus.
    /// One field is open at a time: another repository's is dropped with what was typed.
    fn open_field(
        &mut self,
        repository: RepositoryId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = &self.field
            && open.repository == repository
        {
            open.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        let text = self
            .models
            .settings
            .read(cx)
            .worktrees_folder(&repository)
            .map(|folder| folder.to_string_lossy().into_owned())
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(FOLDER_PLACEHOLDER));
        input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        let enter = cx.subscribe_in(
            &input,
            window,
            |screen, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    screen.send_folder(window, cx);
                }
            },
        );
        self.field = Some(FolderField {
            repository,
            input,
            refused: None,
            sending: None,
            _enter: enter,
        });
        cx.notify();
    }

    /// Closes the field with nothing sent, and gives the focus back to the screen.
    fn close_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.field.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    /// Enter in the field: sends the folder it holds, or none when it holds nothing, and
    /// waits for the Keeper's answer with the field open.
    fn send_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = &mut self.field else {
            return;
        };
        let text = field.input.read(cx).value().trim().to_owned();
        let repository = field.repository.clone();
        field.refused = None;
        let setting = Setting::WorktreesFolder {
            repository: repository.clone(),
            folder: (!text.is_empty()).then(|| PathBuf::from(text)),
        };
        let answered = self
            .models
            .settings
            .update(cx, |settings, cx| settings.set_answered(setting, cx));
        field.sending = Some(cx.spawn_in(window, async move |screen, cx| {
            let outcome = answered.await;
            screen
                .update_in(cx, |screen, window, cx| {
                    screen.sent(&repository, outcome, window, cx)
                })
                .ok();
        }));
        cx.notify();
    }

    /// The Keeper's answer to a folder: `Ok` closes the field, a refusal keeps it open with
    /// the reason. An answer for a field that is no longer open is no news.
    fn sent(
        &mut self,
        repository: &RepositoryId,
        outcome: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = &mut self.field else {
            return;
        };
        if field.repository != *repository {
            return;
        }
        match outcome {
            Ok(()) => self.close_field(window, cx),
            Err(reason) => {
                field.refused = Some(reason);
                cx.notify();
            }
        }
    }

    /// The lines of the sizes: each workspace of the answer that the Window lists, then the
    /// states of those it does not, then the total; each with its selector and its words.
    fn size_lines(&self, cx: &App) -> Vec<(String, String)> {
        let Some(sizes) = self.models.states.read(cx).sizes() else {
            return Vec::new();
        };
        let workspaces = self.models.workspaces.read(cx);
        let mut lines = Vec::new();
        let mut listed = 0u64;
        for size in &sizes.workspaces {
            if let Some(workspace) = workspaces.workspace(&size.workspace) {
                listed += size.bytes;
                lines.push((
                    format!("settings-size-{}", size.workspace),
                    format!("{}: {}", workspace_name(workspace), size_words(size.bytes)),
                ));
            }
        }
        if sizes.total > listed {
            lines.push((
                "settings-size-other".to_owned(),
                format!("{OTHER_STATES}: {}", size_words(sizes.total - listed)),
            ));
        }
        lines.push((
            "settings-size-total".to_owned(),
            format!("{TOTAL_STATES}: {}", size_words(sizes.total)),
        ));
        lines
    }
}

impl Render for SettingsScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.built(cx);
        let cursor = self.clamped_cursor(rows.len());
        let sizes = self.size_lines(cx);
        let problem = self.models.states.read(cx).problem().map(str::to_owned);
        let theme = cx.theme();
        let (background, foreground, muted, border) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
        );
        let (danger, hover) = (theme.danger, theme.list_hover);
        let (primary, primary_text) = (theme.primary, theme.primary_foreground);
        let (secondary, secondary_text) = (theme.secondary, theme.secondary_foreground);
        let (accent, accent_text) = (theme.accent, theme.accent_foreground);
        let words = |selector: String, text: String| {
            let inner = format!("{selector}:{text}");
            div()
                .debug_selector(move || selector.clone())
                .child(div().debug_selector(move || inner.clone()).child(text))
        };

        let mut drawn_rows = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let mut values = div().flex().flex_row().flex_wrap().gap_1();
            for (j, label) in row.shown.values.iter().enumerate() {
                let chosen = row.shown.chosen == Some(j);
                let selector = format!("settings-value-{i}-{j}:{label}");
                let id = SharedString::from(selector.clone());
                let control = div()
                    .id(id)
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_sm()
                    .on_click(cx.listener(move |screen, _: &ClickEvent, window, cx| {
                        screen.choose(i, j, window, cx);
                    }))
                    .child(label.clone());
                // The chosen mark and the colours of a chosen value are set in this one
                // branch, on the control itself: a value cannot be marked chosen and drawn
                // as another.
                let control = if chosen {
                    control
                        .debug_selector(move || format!("settings-chosen-{i}"))
                        .bg(primary)
                        .text_color(primary_text)
                } else {
                    control
                        .bg(secondary)
                        .text_color(secondary_text)
                        .hover(|style| style.bg(accent).text_color(accent_text))
                };
                // One selector to an element: the control has the chosen mark, so the value's
                // own is on an element around it, which is as large as the control is.
                values = values.child(
                    div()
                        .debug_selector(move || selector.clone())
                        .child(control),
                );
            }
            let block = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_md()
                .text_color(foreground)
                .child(
                    div()
                        .debug_selector({
                            let title = format!("settings-title-{i}:{}", row.shown.title);
                            move || title.clone()
                        })
                        .child(row.shown.title.clone()),
                )
                .child(values);
            let block = if i == cursor {
                block
                    .debug_selector(|| "settings-cursor".to_owned())
                    .bg(hover)
            } else {
                block
            };
            let mut notes = div().flex().flex_col().gap_0p5().px_2();
            match &row.shown.row {
                SettingRow::Extension(agent) => {
                    if let Some(consent) = self
                        .models
                        .consents
                        .read(cx)
                        .all()
                        .iter()
                        .find(|consent| consent.agent == *agent)
                    {
                        notes = notes.child(
                            words(format!("settings-question-{i}"), consent.question.clone())
                                .text_sm()
                                .text_color(muted),
                        );
                        if let Some(problem) = &consent.problem {
                            notes = notes.child(
                                words(format!("settings-problem-{i}"), problem.clone())
                                    .text_sm()
                                    .text_color(danger),
                            );
                        }
                    }
                }
                SettingRow::WorktreesFolder(repository) => {
                    if let Some(field) = self
                        .field
                        .as_ref()
                        .filter(|field| field.repository == *repository)
                    {
                        notes = notes.child(
                            div()
                                .debug_selector(|| "settings-folder-field".to_owned())
                                .on_action(cx.listener(|screen, _: &Escape, window, cx| {
                                    screen.close_field(window, cx);
                                }))
                                .child(Input::new(&field.input)),
                        );
                        if let Some(reason) = &field.refused {
                            notes = notes.child(
                                words("settings-folder-refused".to_owned(), reason.clone())
                                    .text_sm()
                                    .text_color(danger),
                            );
                        }
                    }
                }
                SettingRow::States => {
                    for (selector, text) in &sizes {
                        notes = notes.child(
                            words(selector.clone(), text.clone())
                                .text_sm()
                                .text_color(muted),
                        );
                    }
                    if let Some(problem) = &problem {
                        notes = notes.child(
                            words("settings-size-problem".to_owned(), problem.clone())
                                .text_sm()
                                .text_color(danger),
                        );
                    }
                }
                _ => {}
            }
            drawn_rows.push(
                div()
                    .debug_selector(move || format!("settings-row-{i}"))
                    .flex()
                    .flex_col()
                    .px_3()
                    .py_1()
                    .child(block)
                    .child(notes),
            );
        }

        let notice = self.notice.clone().map(|notice| {
            words("settings-notice".to_owned(), notice)
                .px_4()
                .py_1()
                .text_sm()
                .text_color(danger)
                .border_b_1()
                .border_color(border)
        });
        let root = div()
            .debug_selector(|| "settings-screen".to_owned())
            .track_focus(&self.focus)
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .bg(background)
            .text_color(foreground)
            .on_action(cx.listener(Self::next_setting))
            .on_action(cx.listener(Self::previous_setting))
            .on_action(cx.listener(Self::next_value))
            .on_action(cx.listener(Self::previous_value))
            .on_action(cx.listener(Self::open_setting))
            .on_action(cx.listener(Self::clean_up));
        // With a field open the screen sets no key context, so that every key is the
        // field's: the letters of the screen's keys are typed there.
        let root = if self.field.is_none() {
            root.key_context(SETTINGS_CONTEXT)
        } else {
            root
        };
        root.children(notice).child(
            div()
                .id("settings-rows")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .children(drawn_rows),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::size_words;

    #[test]
    fn a_size_under_a_kilobyte_is_in_bytes_with_no_separator() {
        assert_eq!(size_words(0), "0 bytes");
        assert_eq!(size_words(1), "1 bytes");
        assert_eq!(size_words(1_023), "1023 bytes");
    }

    #[test]
    fn each_value_the_doc_lists_gives_the_words_it_gives() {
        for (bytes, words) in [
            (0, "0 bytes"),
            (1_023, "1023 bytes"),
            (1_024, "1 KB"),
            (2_560, "2.5 KB"),
            (4_096, "4 KB"),
            (2_621_440, "2.5 MB"),
            (2_629_632, "2.5 MB"),
            (10_485_760, "10 MB"),
            (3_221_225_472, "3 GB"),
        ] {
            assert_eq!(size_words(bytes), words, "{bytes} bytes");
        }
    }

    #[test]
    fn the_unit_is_the_largest_of_which_there_is_at_least_one() {
        assert_eq!(size_words(1_048_575), "1024 KB");
        assert_eq!(size_words(1_048_576), "1 MB");
        assert_eq!(size_words(1_073_741_823), "1024 MB");
        assert_eq!(size_words(1_073_741_824), "1 GB");
        assert_eq!(size_words(1_099_511_627_776), "1 TB");
        assert_eq!(size_words(5 * 1_099_511_627_776 / 2), "2.5 TB");
        // Past a TB there is no larger unit: the number grows.
        assert_eq!(size_words(2_048 * 1_099_511_627_776), "2048 TB");
        assert_eq!(size_words(u64::MAX), "16777216 TB");
    }

    #[test]
    fn below_ten_the_number_has_its_decimal_unless_it_is_0_and_from_ten_on_it_is_whole() {
        assert_eq!(size_words(1_536), "1.5 KB");
        assert_eq!(size_words(1_024 * 9), "9 KB");
        // 9.5 KB is as written; 10.4 KB is rounded to a whole number, 10.
        assert_eq!(size_words(1_024 * 19 / 2), "9.5 KB");
        assert_eq!(size_words(1_024 * 104 / 10), "10 KB");
        assert_eq!(size_words(1_024 * 105 / 10 + 1), "11 KB");
        assert_eq!(size_words(1_024 * 99 + 600), "100 KB");
        assert_eq!(size_words(1_024 * 1_000), "1000 KB");
    }

    #[test]
    fn a_decimal_rounds_half_away_from_zero_and_the_rounding_can_reach_ten() {
        // 1.25 KB is 1.3 (a half rounds up); 1.2499 KB is 1.2.
        assert_eq!(size_words(1_280), "1.3 KB");
        assert_eq!(size_words(1_279), "1.2 KB");
        // 9.95 KB is 10 and 9.94 KB is 9.9: after rounding the number is what is written.
        assert_eq!(size_words(10_189), "10 KB");
        assert_eq!(size_words(10_178), "9.9 KB");
        // 9.949 is below the half of a decimal.
        assert_eq!(size_words(10_188), "9.9 KB");
        // A number just under the next whole one is written whole.
        assert_eq!(size_words(1_024 * 10 + 511), "10 KB");
        assert_eq!(size_words(1_024 * 10 + 512), "11 KB");
    }
}
