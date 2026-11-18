//! The sidebar: every repository with its worktrees under it, as a full list or as a rail.

use gpui_kit::*;
use yaar_link::{RepositoryId, WorkspaceId};

use crate::models::WorkspaceList;

/// The form the sidebar is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarForm {
    /// Every repository by its name, with its worktrees under it by their branches.
    Full,
    /// A narrow strip with one mark for each workspace.
    Rail,
}

/// The width of the window below which the sidebar is a rail while the user has pinned
/// neither form.
pub const RAIL_BELOW: Pixels = px(1000.);

/// The width of the sidebar as the full list.
pub const FULL_WIDTH: Pixels = px(240.);

/// The width of the sidebar as a rail.
pub const RAIL_WIDTH: Pixels = px(48.);

/// The form of the sidebar: the pinned one if the user pinned one, whatever the width;
/// else a rail in a window narrower than [`RAIL_BELOW`] and the full list in any other.
pub fn sidebar_form(window_width: Pixels, pinned: Option<SidebarForm>) -> SidebarForm {
    todo!()
}

/// One row of the sidebar. The rail holds the same rows as the full list, drawn
/// narrower.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarRow {
    /// A repository; its worktrees are the rows that follow, up to the next repository.
    Repository {
        /// Its id.
        id: RepositoryId,
        /// Its name.
        name: String,
    },
    /// A worktree of the repository above.
    Workspace {
        /// Its id.
        id: WorkspaceId,
        /// The name it is listed by: its branch, or the commit it sits on when no branch
        /// is checked out.
        name: String,
        /// Whether it is the workspace on screen.
        selected: bool,
    },
}

/// The sidebar of the window: a view, and the model of what it shows.
///
/// A click on a workspace puts it on screen. A click on the pin control does what
/// [`crate::ToggleSidebar`] does.
///
/// For tests, which cannot read what is drawn, two kinds of element carry a selector
/// that `debug_bounds` finds: each workspace row, `sidebar-workspace-<id>`, and the pin
/// control, `sidebar-pin`.
pub struct Sidebar {
    workspaces: Entity<WorkspaceList>,
    pinned: Option<SidebarForm>,
    form: SidebarForm,
}

impl Sidebar {
    /// A sidebar that lists `workspaces`, with neither form pinned.
    ///
    /// By itself it knows no window. It follows the width of a window only inside a
    /// [`crate::Frame`] made by [`crate::Frame::with_panes`], which watches its window
    /// and tells the sidebar; until it is told a width, the form is the full list.
    pub fn new(workspaces: Entity<WorkspaceList>, cx: &mut Context<Self>) -> Self {
        todo!()
    }

    /// The form the sidebar has now, at the width the window has now.
    pub fn form(&self) -> SidebarForm {
        todo!()
    }

    /// The form the user pinned, if any.
    pub fn pinned(&self) -> Option<SidebarForm> {
        todo!()
    }

    /// Pins a form, which the sidebar then keeps at any width; with none, the form
    /// follows the width of the window again.
    pub fn pin(&mut self, form: Option<SidebarForm>, cx: &mut Context<Self>) {
        todo!()
    }

    /// The rows the sidebar shows, top to bottom. These are the rows it draws from, not
    /// a second account of them.
    pub fn rows(&self, cx: &App) -> Vec<SidebarRow> {
        todo!()
    }
}
