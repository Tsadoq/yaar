//! The shared model of code intelligence: the Window's one way to the Navigator.
//!
//! A pane asks what a name is, where it is defined and who uses it, through this model, as
//! it asks the Keeper through the others; it holds no Navigator. The model holds the
//! Frame's end of the Navigator's port, `yaar_navigator::Port`, once the Window program
//! connected one ([`Navigation::connect`]); until then every ask resolves to
//! [`NavError::NotConnected`] and no state is known. A pane's test connects a port whose
//! other end the test plays, as it plays the Keeper on a socket.
//!
//! What it tells the Navigator by itself, once connected and from then on:
//! - [`yaar_navigator::Ask::Workspaces`], every workspace of `WorkspaceList` with its
//!   repository and folder (none for one whose worktree is gone), at the connect and at
//!   every change of the list; and [`yaar_navigator::Ask::OnScreen`], the workspace on
//!   screen, at the connect and whenever it changes;
//! - [`yaar_navigator::Ask::Consents`], every `Setting::ServersInRepository` of
//!   `SettingList`, at the connect and whenever the settings change;
//! - [`yaar_navigator::Ask::FilesChanged`], the paths of the changed files of the
//!   workspace on screen as `WorkspaceChanges` holds them, whenever they change;
//! - for each [`yaar_navigator::Told::ExportWanted`], `Request::ExportVersion` to the
//!   Keeper through the link, and the answer handed back as
//!   [`yaar_navigator::Ask::Exported`]: `failed` none for `Reply::Done`, else the refusal
//!   or the loss of the link as a sentence.
//!
//! What it keeps of what the Navigator tells: the languages, and the newest list of servers
//! ([`Navigation::servers`]), each change of which notifies the model's observers. Each
//! answer resolves the [`Asking`] of its id.

use std::path::Path;

use gpui_kit::*;
use yaar_link::{RepositoryId, WorkspaceId};
use yaar_navigator::{Answered, AskId, Language, Location, OpenedFile, Place, Port, ServerState};

use crate::link_model::KeeperLink;
use crate::models::{WorkspaceChanges, WorkspaceList};
use crate::settings::SettingList;

/// Why an ask brought no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavError {
    /// No Navigator is connected.
    NotConnected,
    /// The Navigator ended before it answered.
    Gone,
}

impl std::fmt::Display for NavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => f.write_str("code intelligence is not running"),
            Self::Gone => f.write_str("code intelligence stopped before it answered"),
        }
    }
}

impl std::error::Error for NavError {}

/// An ask on its way: a future of its answer. Dropped before the answer came, it tells the
/// Navigator [`yaar_navigator::Ask::Cancel`], so that the reviewer who moved on leaves no
/// work behind.
pub struct Asking<T> {
    id: AskId,
    task: Task<Result<T, NavError>>,
}

impl<T> Asking<T> {
    /// The ask's id, as the Navigator sees it.
    pub fn id(&self) -> AskId {
        self.id
    }
}

impl<T: Unpin + 'static> std::future::Future for Asking<T> {
    type Output = Result<T, NavError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.task).poll(cx)
    }
}

/// The Window's one way to the Navigator.
pub struct Navigation {
    link: Entity<KeeperLink>,
    port: Option<Port>,
}

impl Navigation {
    /// The model of one window, with no Navigator connected: it observes `workspaces`,
    /// `changes` and `settings`, and asks the Keeper through `link`.
    pub fn new(
        link: Entity<KeeperLink>,
        workspaces: Entity<WorkspaceList>,
        changes: Entity<WorkspaceChanges>,
        settings: Entity<SettingList>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _ = (workspaces, changes, settings, cx);
        Self { link, port: None }
    }

    /// Connects the Frame's end of a Navigator's port: from now on the model tells it what
    /// the module doc says, at once and then at each change, and reads what it tells. A
    /// second connect replaces the first, whose asks waiting resolve to
    /// [`NavError::Gone`].
    pub fn connect(&mut self, port: Port, cx: &mut Context<Self>) {
        let _ = (&self.link, &self.port, port, cx);
        todo!()
    }

    /// Whether a Navigator is connected.
    pub fn is_connected(&self) -> bool {
        todo!()
    }

    /// Every language the Navigator's registry knows, as it told them.
    pub fn languages(&self) -> &[Language] {
        todo!()
    }

    /// Every server, as the Navigator last told them.
    pub fn servers(&self) -> &[ServerState] {
        todo!()
    }

    /// The language of a file, by its extension; none for a file no row knows.
    pub fn language_of(&self, path: &Path) -> Option<&Language> {
        let _ = path;
        todo!()
    }

    /// The state of the server for the file `path` of `workspace`: of its old side of
    /// `old_side`, or of the files on disk for none. None for a file of no known language,
    /// and while the Navigator told no state for that server.
    pub fn state_for(
        &self,
        workspace: &WorkspaceId,
        path: &Path,
        old_side: Option<&str>,
    ) -> Option<&ServerState> {
        let _ = (workspace, path, old_side);
        todo!()
    }

    /// Asks what the server reports for the name at `place`.
    pub fn hover(
        &mut self,
        place: Place,
        cx: &mut Context<Self>,
    ) -> Asking<Answered<Option<String>>> {
        let _ = (place, cx);
        todo!()
    }

    /// Asks where the name at `place` is defined.
    pub fn definition(
        &mut self,
        place: Place,
        cx: &mut Context<Self>,
    ) -> Asking<Answered<Vec<Location>>> {
        let _ = (place, cx);
        todo!()
    }

    /// Asks for every reference to the name at `place`.
    pub fn references(
        &mut self,
        place: Place,
        cx: &mut Context<Self>,
    ) -> Asking<Answered<Vec<Location>>> {
        let _ = (place, cx);
        todo!()
    }

    /// Asks for the file of `location`, read, for the file view.
    pub fn open(
        &mut self,
        location: Location,
        cx: &mut Context<Self>,
    ) -> Asking<Result<OpenedFile, String>> {
        let _ = (location, cx);
        todo!()
    }

    /// The user accepted the offer to download the server of `language`.
    pub fn download(&mut self, language: &str) {
        let _ = language;
        todo!()
    }

    /// The user asked for the update of the server of `language`.
    pub fn update(&mut self, language: &str) {
        let _ = language;
        todo!()
    }

    /// The old side of the change on screen in `workspace` is now `version`, or none.
    pub fn old_side_shown(&mut self, workspace: &WorkspaceId, version: Option<String>) {
        let _ = (workspace, version);
        todo!()
    }

    /// The user's answer on the servers inside `repository`: set as
    /// `Setting::ServersInRepository` through `SettingList::set`, which the Keeper keeps
    /// and tells every Window; the Navigator hears it from the settings.
    pub fn answer_consent(
        &mut self,
        repository: RepositoryId,
        allowed: bool,
        cx: &mut Context<Self>,
    ) {
        let _ = (repository, allowed, cx);
        todo!()
    }

    /// The answer the user gave on the servers inside `repository`, as the settings hold
    /// it; none while the question waits.
    pub fn consent(&self, repository: &RepositoryId, cx: &App) -> Option<bool> {
        let _ = (repository, cx);
        todo!()
    }
}
