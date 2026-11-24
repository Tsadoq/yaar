//! The window around the panes: sidebar, layout, themes, settings, key routing, the Window's link
//! to the Keeper and the shared models the panes observe. A pane depends on this crate and on no
//! other pane.

mod agents;
mod consents;
mod controls;
pub mod keeper;
mod link_model;
mod looks;
mod models;
mod navigation;
mod routes;
mod settings;
pub mod settings_screen;
mod sidebar;
mod states;
pub mod theme;
pub mod trace;
mod view;

pub use agents::{AgentList, TerminalList, Wanted};
pub use consents::ConsentList;
pub use controls::{
    Asks, CleanUpStates, CloseSettings, CloseWindow, Control, FocusNextPane, FocusPreviousPane,
    HideApp, HideOtherApps, KEY_CONTEXT, Minimize, MovePanes, NewWorktree, NextSetting, NextValue,
    OpenFolder, OpenSetting, OpenSettings, PROGRAM_CONTEXT, PROGRAM_KEYS, Place, PreviousSetting,
    PreviousValue, Quit, QuitAndStopAgents, SETTINGS_CONTEXT, SelectNextWorkspace,
    SelectPreviousWorkspace, Sends, SetBase, ShowFinishedTurn, ToggleSidebar, UnpinSidebar, bind,
    controls, release_program_keys,
};
pub use link_model::{AskError, KeeperLink, LinkStatus, TerminalOutput};
pub use looks::Looks;
pub use models::{Models, WorkspaceChanges, WorkspaceList};
pub use navigation::{Asking, NavError, Navigation};
pub use routes::Routes;
pub use settings::{NOT_KEPT, SettingList};
pub use settings_screen::SettingsScreen;
pub use sidebar::{FULL_WIDTH, RAIL_BELOW, RAIL_WIDTH, Sidebar, SidebarForm, SidebarRow};
pub use states::RecordedStates;
pub use view::{Beside, BuildPane, Frame, Opened, Pane, PaneEntry, TurnNotice, open};
