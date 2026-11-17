//! Frame (`W-SHELL`): the window around the panes.
//!
//! Owns: the window, the sidebar, the layout of panes, themes, settings screens, key
//! routing between panes, notices. It also holds the Window's connection to the Keeper
//! and the shared models the panes observe; a pane depends on this crate and on no other
//! pane.
//!
//! Surface:
//! - [`Frame`], the root view of the window, and [`open`], which opens it;
//! - [`KeeperLink`], the model that says how the Window stands with the Keeper;
//! - [`keeper`], which finds or starts the Keeper. It uses no interface code;
//! - [`trace`], the marks by which a start is timed from outside.
//!
//! In this first form the Frame holds no pane: it draws an empty window that names the
//! Keeper it is linked to.

pub mod keeper;
mod link_model;
pub mod trace;
mod view;

pub use link_model::{KeeperLink, LinkStatus};
pub use view::{Frame, open};
