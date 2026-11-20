//! Frame (`W-SHELL`): the window around the panes.
//!
//! Owns: the window, the sidebar, the layout of panes, themes, settings screens, key
//! routing between panes, notices. It also holds the Window's connection to the Keeper
//! and the shared models the panes observe; a pane depends on this crate and on no other
//! pane.
//!
//! Surface:
//! - [`Frame`], the root view of the window, and [`open`], which opens it with the panes
//!   it is given: a [`BuildPane`] for each, which makes a [`Pane`]. A pane's crate hands
//!   the Window program one [`PaneEntry`], which holds the builder and the keys;
//! - [`KeeperLink`], the model that says how the Window stands with the Keeper, through
//!   which the Window asks the Keeper and hears its facts;
//! - [`Models`], the shared models a pane observes: [`WorkspaceList`], the repositories
//!   with their worktrees and the workspace on screen, [`WorkspaceChanges`], the changes
//!   of that workspace, [`AgentList`], the agent sessions of every workspace,
//!   [`TerminalList`], the terminals the Keeper holds, [`SettingList`], the settings the
//!   user set, and [`ConsentList`], the questions for leave of the Adapters that write
//!   into a CLI's own settings only with the user's yes;
//! - [`Sidebar`], the list of repositories and worktrees, as the full list or as a rail;
//! - [`controls`], the table of actions with their keys, and the actions of the Frame.
//!   A [`Control`] says in which [`Place`] its key works; [`KEY_CONTEXT`] is the key
//!   context the Frame sets around everything it draws, and [`PROGRAM_CONTEXT`] the one
//!   an element sets that hands every key to a program, which [`release_program_keys`]
//!   gives the keys the toolkit binds for the whole window;
//! - [`Routes`], through which a pane takes actions run anywhere in the window, and
//!   [`Beside`], where the panes after the first lie;
//! - [`keeper`], which finds or starts the Keeper. It uses no interface code;
//! - [`trace`], the marks by which a start is timed from outside.
//!
//! How the parts of the Window talk. What the Keeper tells arrives at the [`KeeperLink`]
//! and changes a shared model; a pane observes the models it shows. A pane that needs
//! more asks the Keeper through the models. What the user wants travels as an action,
//! which the Frame or the pane with the focus handles, or the pane that routed it through
//! [`Routes`]. No pane holds another pane.

mod agents;
mod consents;
mod controls;
pub mod keeper;
mod link_model;
mod models;
mod routes;
mod settings;
mod sidebar;
pub mod trace;
mod view;

pub use agents::{AgentList, TerminalList, Wanted};
pub use consents::ConsentList;
pub use controls::{
    Asks, Control, FocusNextPane, FocusPreviousPane, KEY_CONTEXT, MovePanes, OpenFolder,
    PROGRAM_CONTEXT, PROGRAM_KEYS, Place, Quit, QuitAndStopAgents, SelectNextWorkspace,
    SelectPreviousWorkspace, Sends, SetBase, ShowFinishedTurn, ToggleSidebar, UnpinSidebar, bind,
    controls, release_program_keys,
};
pub use link_model::{AskError, KeeperLink, LinkStatus, TerminalOutput};
pub use models::{Models, WorkspaceChanges, WorkspaceList};
pub use routes::Routes;
pub use settings::SettingList;
pub use sidebar::{
    FULL_WIDTH, RAIL_BELOW, RAIL_WIDTH, Sidebar, SidebarForm, SidebarRow, sidebar_form,
};
pub use view::{Beside, BuildPane, Frame, Opened, Pane, PaneEntry, TurnNotice, open};
