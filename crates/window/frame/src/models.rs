//! The shared models of the Window: what the Keeper told, kept where every pane can
//! observe it.
//!
//! A pane is handed the [`Models`] when it is built. It observes the models it shows,
//! and asks the Keeper through them; it holds no other pane and no connection of its own.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use yaar_link::{Changes, FileSides, Refusal, Repository, Workspace, WorkspaceId};

use crate::link_model::{AskError, KeeperLink};

/// The shared models, as one handle. Cheap to clone.
#[derive(Clone)]
pub struct Models {
    /// How the Window stands with the Keeper, and the way to ask it.
    pub link: Entity<KeeperLink>,
    /// The repositories and workspaces, and the workspace on screen.
    pub workspaces: Entity<WorkspaceList>,
    /// The changes of the workspace on screen.
    pub changes: Entity<WorkspaceChanges>,
}

impl Models {
    /// The models of one window, fed by `link`.
    pub fn new(link: Entity<KeeperLink>, cx: &mut App) -> Models {
        todo!()
    }
}

/// The repositories the user opened, their worktrees, and which workspace is on screen.
///
/// It asks the Keeper for the list when the link is connected, and takes every list the
/// Keeper tells afterwards. The workspace on screen is the Window's own choice: the
/// first of the list until the user selects another, and a workspace the user just
/// opened. When that workspace leaves the list, the first of the list takes its place.
pub struct WorkspaceList {
    link: Entity<KeeperLink>,
    repositories: Vec<Repository>,
    selected: Option<WorkspaceId>,
    refusal: Option<Refusal>,
}

impl WorkspaceList {
    /// An empty list, fed by `link`.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        todo!()
    }

    /// The repositories, each with its worktrees, as the Keeper last told them.
    pub fn repositories(&self) -> &[Repository] {
        todo!()
    }

    /// The workspace with this id, if the list holds it.
    pub fn workspace(&self, id: &WorkspaceId) -> Option<&Workspace> {
        todo!()
    }

    /// The workspace on screen.
    pub fn selected(&self) -> Option<&WorkspaceId> {
        todo!()
    }

    /// Puts a workspace of the list on screen. Does nothing for an id the list does not
    /// hold.
    pub fn select(&mut self, workspace: &WorkspaceId, cx: &mut Context<Self>) {
        todo!()
    }

    /// Puts the next workspace of the list on screen, in the order the sidebar shows
    /// them; after the last comes the first.
    pub fn select_next(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    /// Puts the previous workspace of the list on screen; before the first comes the
    /// last.
    pub fn select_previous(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    /// Asks the Keeper to open a folder. When it is opened, its workspace is put on
    /// screen; when it is refused, [`WorkspaceList::refusal`] says why.
    pub fn open_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        todo!()
    }

    /// Asks the Keeper to count the changes of the workspace on screen from this branch
    /// or commit. When it is refused, [`WorkspaceList::refusal`] says why. Does nothing
    /// while no workspace is on screen.
    pub fn set_base(&mut self, base: String, cx: &mut Context<Self>) {
        todo!()
    }

    /// Why the Keeper refused the last thing the user asked of it here; none once the
    /// user asks again.
    pub fn refusal(&self) -> Option<&Refusal> {
        todo!()
    }
}

/// The changes of the workspace on screen.
///
/// It asks the Keeper whenever another workspace comes on screen, whenever the Keeper
/// tells that the changes of that workspace changed, and whenever
/// [`WorkspaceChanges::refresh`] is called. While it waits for a new answer it keeps
/// showing the last one of the same workspace.
pub struct WorkspaceChanges {
    link: Entity<KeeperLink>,
    workspaces: Entity<WorkspaceList>,
    workspace: Option<WorkspaceId>,
    changes: Option<Changes>,
    loading: bool,
    refusal: Option<Refusal>,
}

impl WorkspaceChanges {
    /// No changes yet; follows the workspace on screen of `workspaces`.
    pub fn new(
        link: Entity<KeeperLink>,
        workspaces: Entity<WorkspaceList>,
        cx: &mut Context<Self>,
    ) -> Self {
        todo!()
    }

    /// The workspace these are the changes of: the one on screen.
    pub fn workspace(&self) -> Option<&WorkspaceId> {
        todo!()
    }

    /// The changes, as the Keeper last answered for this workspace; none before the
    /// first answer.
    pub fn changes(&self) -> Option<&Changes> {
        todo!()
    }

    /// Whether an answer of the Keeper is still awaited.
    pub fn is_loading(&self) -> bool {
        todo!()
    }

    /// Why the Keeper refused the last time it was asked; none once it answers.
    pub fn refusal(&self) -> Option<&Refusal> {
        todo!()
    }

    /// Asks the Keeper again for the changes of the workspace on screen. The Frame calls
    /// it when the window comes to the front, since files change while nobody looks.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        todo!()
    }

    /// Asks the Keeper for the two versions of one file of the workspace on screen:
    /// how a pane gets what it diffs. `path` is the path of a changed file as
    /// [`Changes`] lists it.
    pub fn file(&self, path: &Path, cx: &mut Context<Self>) -> Task<Result<FileSides, AskError>> {
        todo!()
    }
}
