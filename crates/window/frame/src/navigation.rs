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
//!   workspace on screen as `WorkspaceChanges` holds them (the old path of a renamed file
//!   too), each time it takes an answer of the Keeper (a file may change again with its
//!   path in the list already), together with the paths the last such answer for that
//!   workspace held, so that a file that left the list because it was put back is told
//!   too; nothing when there are no paths, nothing after a refusal, and nothing at the
//!   connect (a server starts from the files on disk, so a new Navigator holds nothing
//!   stale), though the list the changes model holds then, when it holds one and the last
//!   question was not refused, is the last answer the next one is set against; a path may
//!   be told again and may no longer exist;
//! - for each [`yaar_navigator::Told::ExportWanted`], `Request::ExportVersion` to the
//!   Keeper through the link, and the answer handed back as
//!   [`yaar_navigator::Ask::Exported`]: `failed` none for `Reply::Done`, else the refusal
//!   or the loss of the link as a sentence.
//!
//! What it keeps of what the Navigator tells: the languages, and the newest list of servers
//! ([`Navigation::servers`]), each change of which notifies the model's observers. Each
//! answer resolves the [`Asking`] of its id.
//!
//! Facts and answers of the Keeper arrive in no fixed order (see the module doc of
//! `models`); what this model tells the Navigator is read from the other models when they
//! notify, so it is never older than they are.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::*;
use yaar_link::{Changes, FileStatus, Reply, RepositoryId, Request, Setting, WorkspaceId};
use yaar_navigator::{
    Answered, Ask, AskId, ExportAsk, Language, Location, OpenedFile, Place, Port,
    RepositoryConsent, ServerState, Told, WorkspaceRoot,
};

use crate::link_model::{AskError, KeeperLink};
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

/// Where the answer to one ask goes, by the kind of answer the ask takes.
enum Waiter {
    Hover(async_channel::Sender<Answered<Option<String>>>),
    Definition(async_channel::Sender<Answered<Vec<Location>>>),
    References(async_channel::Sender<Answered<Vec<Location>>>),
    Open(async_channel::Sender<Result<OpenedFile, String>>),
}

/// The asks of one connection that wait for their answer. An ask leaves it when it is
/// answered, and when its [`Asking`] is dropped; clearing it drops every sender, so that
/// each waiting task resolves to [`NavError::Gone`].
type Waiting = Rc<RefCell<HashMap<AskId, Waiter>>>;

/// What an [`Asking`] needs to tell the Navigator that nobody waits any more.
struct Withdraw {
    asks: async_channel::Sender<Ask>,
    waiting: Waiting,
}

/// An ask on its way: a future of its answer. Dropped before the answer came, it tells the
/// Navigator [`yaar_navigator::Ask::Cancel`], so that the reviewer who moved on leaves no
/// work behind.
pub struct Asking<T> {
    id: AskId,
    task: Task<Result<T, NavError>>,
    /// None for an ask that was never sent.
    withdraw: Option<Withdraw>,
}

impl<T> Asking<T> {
    /// The ask's id, as the Navigator sees it.
    pub fn id(&self) -> AskId {
        self.id
    }

    /// An ask that went nowhere, resolved to `error`.
    fn failed(id: AskId, error: NavError) -> Self {
        Self {
            id,
            task: Task::ready(Err(error)),
            withdraw: None,
        }
    }
}

impl<T> Drop for Asking<T> {
    fn drop(&mut self) {
        let Some(Withdraw { asks, waiting }) = &self.withdraw else {
            return;
        };
        // An ask that was answered, or whose connection ended, left the table: nothing is
        // waited for, so nothing is cancelled.
        if waiting.borrow_mut().remove(&self.id).is_some() {
            // A Navigator that ended has no work left to cancel.
            let _ = asks.try_send(Ask::Cancel { id: self.id });
        }
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

/// What the model keeps of the Navigator it is connected to, and of what it told it.
struct Connection {
    asks: async_channel::Sender<Ask>,
    waiting: Waiting,
    /// Reads what the Navigator tells. Dropped with the connection.
    _reading: Task<()>,
    /// False once the Navigator's channel closed: it ended.
    alive: bool,
    /// What the Navigator was last told, none before the first telling: a change is told
    /// once, and an observer that fires for another reason tells nothing.
    told_workspaces: Option<Vec<WorkspaceRoot>>,
    told_on_screen: Option<Option<WorkspaceId>>,
    told_consents: Option<Vec<RepositoryConsent>>,
    /// The paths of the last answer for the changes, by workspace.
    told_changes: HashMap<WorkspaceId, BTreeSet<PathBuf>>,
}

/// The Window's one way to the Navigator.
pub struct Navigation {
    link: Entity<KeeperLink>,
    workspaces: Entity<WorkspaceList>,
    changes: Entity<WorkspaceChanges>,
    settings: Entity<SettingList>,
    connection: Option<Connection>,
    /// The id of the last ask made over any connection, so that no id is used twice.
    last_ask: u64,
    languages: Vec<Language>,
    servers: Vec<ServerState>,
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
        cx.observe(&workspaces, |navigation, _, cx| {
            navigation.tell_workspaces(cx);
        })
        .detach();
        cx.observe(&settings, |navigation, _, cx| {
            navigation.tell_consents(cx);
        })
        .detach();
        cx.observe(&changes, |navigation, _, cx| {
            navigation.tell_files_changed(cx);
        })
        .detach();
        Self {
            link,
            workspaces,
            changes,
            settings,
            connection: None,
            last_ask: 0,
            languages: Vec::new(),
            servers: Vec::new(),
        }
    }

    /// Connects the Frame's end of a Navigator's port: from now on the model tells it what
    /// the module doc says, at once and then at each change, and reads what it tells. A
    /// second connect replaces the first, whose asks waiting resolve to
    /// [`NavError::Gone`].
    pub fn connect(&mut self, port: Port, cx: &mut Context<Self>) {
        if let Some(old) = self.connection.take() {
            // Each sender goes with its entry, so each waiting task resolves.
            old.waiting.borrow_mut().clear();
        }
        let Port { asks, told } = port;
        // The list the changes model holds is the last answer of the Keeper this Navigator
        // is owed: it is not told, since a server starts from the files on disk, but the
        // next answer is set against it, so that a file put back since is told changed.
        let mut told_changes = HashMap::new();
        let held = self.changes.read(cx);
        if held.refusal().is_none()
            && let Some(taken) = held.changes()
        {
            told_changes.insert(taken.workspace.clone(), changed_paths(taken));
        }
        let reading = cx.spawn(async move |navigation, cx| {
            while let Ok(told) = told.recv().await {
                if navigation
                    .update(cx, |navigation, cx| navigation.take(told, cx))
                    .is_err()
                {
                    return;
                }
            }
            navigation
                .update(cx, |navigation, cx| navigation.ended(cx))
                .ok();
        });
        self.connection = Some(Connection {
            asks,
            waiting: Rc::default(),
            _reading: reading,
            alive: true,
            told_workspaces: None,
            told_on_screen: None,
            told_consents: None,
            told_changes,
        });
        // What the last Navigator told is no news of this one.
        self.forget(cx);
        self.tell_workspaces(cx);
        self.tell_consents(cx);
        // Whether a Navigator is connected changed, as far as an observer can tell.
        cx.notify();
    }

    /// Whether a Navigator is connected. A change of the answer, at a connect and at the
    /// end of the Navigator, notifies the model's observers.
    pub fn is_connected(&self) -> bool {
        self.alive().is_some()
    }

    /// Every language the Navigator's registry knows, as it told them.
    pub fn languages(&self) -> &[Language] {
        &self.languages
    }

    /// Every server, as the Navigator last told them.
    pub fn servers(&self) -> &[ServerState] {
        &self.servers
    }

    /// The language of a file, by its extension; none for a file no row knows.
    pub fn language_of(&self, path: &Path) -> Option<&Language> {
        let extension = path.extension()?.to_str()?;
        self.languages
            .iter()
            .find(|language| language.extensions.iter().any(|known| known == extension))
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
        let language = self.language_of(path)?;
        self.servers.iter().find(|server| {
            server.workspace == *workspace
                && server.language == language.name
                && server.old_side.as_deref() == old_side
        })
    }

    /// Asks what the server reports for the name at `place`.
    pub fn hover(
        &mut self,
        place: Place,
        cx: &mut Context<Self>,
    ) -> Asking<Answered<Option<String>>> {
        self.begin(|id| Ask::Hover { id, place }, Waiter::Hover, cx)
    }

    /// Asks where the name at `place` is defined.
    pub fn definition(
        &mut self,
        place: Place,
        cx: &mut Context<Self>,
    ) -> Asking<Answered<Vec<Location>>> {
        self.begin(|id| Ask::Definition { id, place }, Waiter::Definition, cx)
    }

    /// Asks for every reference to the name at `place`.
    pub fn references(
        &mut self,
        place: Place,
        cx: &mut Context<Self>,
    ) -> Asking<Answered<Vec<Location>>> {
        self.begin(|id| Ask::References { id, place }, Waiter::References, cx)
    }

    /// Asks for the file of `location`, read, for the file view.
    pub fn open(
        &mut self,
        location: Location,
        cx: &mut Context<Self>,
    ) -> Asking<Result<OpenedFile, String>> {
        self.begin(|id| Ask::Open { id, location }, Waiter::Open, cx)
    }

    /// The user accepted the offer to download the server of `language`.
    pub fn download(&mut self, language: &str) {
        self.say(Ask::Download {
            language: language.to_owned(),
        });
    }

    /// The user asked for the update of the server of `language`.
    pub fn update(&mut self, language: &str) {
        self.say(Ask::Update {
            language: language.to_owned(),
        });
    }

    /// The old side of the change on screen in `workspace` is now `version`, or none.
    pub fn old_side_shown(&mut self, workspace: &WorkspaceId, version: Option<String>) {
        self.say(Ask::OldSide {
            workspace: workspace.clone(),
            version,
        });
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
        self.settings.update(cx, |settings, cx| {
            settings.set(
                Setting::ServersInRepository {
                    repository,
                    allowed,
                },
                cx,
            );
        });
    }

    /// The answer the user gave on the servers inside `repository`, as the settings hold
    /// it; none while the question waits.
    pub fn consent(&self, repository: &RepositoryId, cx: &App) -> Option<bool> {
        self.settings
            .read(cx)
            .all()
            .iter()
            .find_map(|setting| match setting {
                Setting::ServersInRepository {
                    repository: of,
                    allowed,
                } if of == repository => Some(*allowed),
                _ => None,
            })
    }

    /// The connection, while its Navigator is there.
    fn alive(&self) -> Option<&Connection> {
        self.connection.as_ref().filter(|c| c.alive)
    }

    /// Sends the Navigator an ask nobody waits for. Nothing is sent while none is
    /// connected; what the model tells by itself is told in full at the connect.
    fn say(&mut self, ask: Ask) {
        if let Some(connection) = self.alive() {
            // A Navigator that ended has no one to tell; the reader says so.
            let _ = connection.asks.try_send(ask);
        }
    }

    /// Sends an ask that wants an answer, with a fresh id, and gives the [`Asking`] that
    /// waits for it.
    fn begin<T: 'static>(
        &mut self,
        ask: impl FnOnce(AskId) -> Ask,
        waiter: fn(async_channel::Sender<T>) -> Waiter,
        cx: &mut Context<Self>,
    ) -> Asking<T> {
        self.last_ask += 1;
        let id = AskId(self.last_ask);
        let Some(connection) = self.alive() else {
            return Asking::failed(id, NavError::NotConnected);
        };
        let (answer, answered) = async_channel::bounded(1);
        // Entered before the ask leaves: the answer may come at once.
        connection.waiting.borrow_mut().insert(id, waiter(answer));
        if connection.asks.try_send(ask(id)).is_err() {
            connection.waiting.borrow_mut().remove(&id);
            return Asking::failed(id, NavError::Gone);
        }
        let withdraw = Withdraw {
            asks: connection.asks.clone(),
            waiting: connection.waiting.clone(),
        };
        Asking {
            id,
            task: cx.spawn(async move |_, _| answered.recv().await.map_err(|_| NavError::Gone)),
            withdraw: Some(withdraw),
        }
    }

    /// Takes what the Navigator told.
    fn take(&mut self, told: Told, cx: &mut Context<Self>) {
        match told {
            Told::Languages(languages) => {
                if languages != self.languages {
                    self.languages = languages;
                    cx.notify();
                }
            }
            Told::Servers(servers) => {
                if servers != self.servers {
                    self.servers = servers;
                    cx.notify();
                }
            }
            Told::Hover { id, answer } => self.answer(id, |waiter| match waiter {
                Waiter::Hover(to) => to.try_send(answer).is_ok(),
                _ => false,
            }),
            Told::Definition { id, answer } => self.answer(id, |waiter| match waiter {
                Waiter::Definition(to) => to.try_send(answer).is_ok(),
                _ => false,
            }),
            Told::References { id, answer } => self.answer(id, |waiter| match waiter {
                Waiter::References(to) => to.try_send(answer).is_ok(),
                _ => false,
            }),
            Told::Opened { id, file } => self.answer(id, |waiter| match waiter {
                Waiter::Open(to) => to.try_send(file).is_ok(),
                _ => false,
            }),
            Told::ExportWanted(wanted) => self.export(wanted, cx),
        }
    }

    /// Hands an answer to the ask it is for. `give` says whether the waiter takes an
    /// answer of this kind: one that is not the kind the ask takes is no answer to it, and
    /// the ask goes on waiting.
    fn answer(&mut self, id: AskId, give: impl FnOnce(&Waiter) -> bool) {
        let Some(connection) = &self.connection else {
            return;
        };
        let mut waiting = connection.waiting.borrow_mut();
        if waiting.get(&id).is_some_and(give) {
            waiting.remove(&id);
        }
    }

    /// The Navigator's channel closed: it ended. What waits for it is told so, and what it
    /// told is no longer known.
    fn ended(&mut self, cx: &mut Context<Self>) {
        if let Some(connection) = &mut self.connection {
            connection.alive = false;
            connection.waiting.borrow_mut().clear();
        }
        self.forget(cx);
        // `forget` notifies only for what there was to forget, and a Navigator that told
        // no server and no language still was connected.
        cx.notify();
    }

    /// Drops the languages and the servers a Navigator told.
    fn forget(&mut self, cx: &mut Context<Self>) {
        if !self.languages.is_empty() || !self.servers.is_empty() {
            self.languages.clear();
            self.servers.clear();
            cx.notify();
        }
    }

    /// Asks the Keeper to write the version the Navigator wants, and tells the Navigator
    /// how it went.
    fn export(&mut self, wanted: ExportAsk, cx: &mut Context<Self>) {
        let Some(asks) = self.alive().map(|connection| connection.asks.clone()) else {
            return;
        };
        let request = Request::ExportVersion {
            workspace: wanted.workspace.clone(),
            version: wanted.version.clone(),
            into: wanted.into.clone(),
        };
        let asked = self.link.update(cx, |link, cx| link.ask(request, cx));
        // Not kept: the Navigator is owed the answer whatever the panes do next.
        cx.spawn(async move |_, _| {
            let failed = match asked.await {
                Ok(Reply::Done) => None,
                Ok(_) => Some(AskError::Unexpected.to_string()),
                Err(error) => Some(error.to_string()),
            };
            // A Navigator that ended is told nothing.
            let _ = asks.try_send(Ask::Exported {
                asked: wanted,
                failed,
            });
        })
        .detach();
    }

    /// Tells the Navigator the workspaces, then the one on screen, each when it is not
    /// what it was last told.
    fn tell_workspaces(&mut self, cx: &mut Context<Self>) {
        if !self.is_connected() {
            return;
        }
        let list = self.workspaces.read(cx);
        let roots: Vec<WorkspaceRoot> = list
            .repositories()
            .iter()
            .flat_map(|repository| {
                repository.workspaces.iter().map(|workspace| WorkspaceRoot {
                    id: workspace.id.clone(),
                    repository: repository.id.clone(),
                    // A worktree git removed stays listed, with nothing checked out.
                    folder: workspace.head.is_some().then(|| workspace.path.clone()),
                })
            })
            .collect();
        let on_screen = list.selected().cloned();
        let Some(connection) = self.connection.as_mut() else {
            return;
        };
        // The list first: the Navigator knows no workspace it was not told.
        if connection.told_workspaces.as_ref() != Some(&roots) {
            let _ = connection.asks.try_send(Ask::Workspaces(roots.clone()));
            connection.told_workspaces = Some(roots);
        }
        if connection.told_on_screen.as_ref() != Some(&on_screen) {
            let _ = connection.asks.try_send(Ask::OnScreen(on_screen.clone()));
            connection.told_on_screen = Some(on_screen);
        }
    }

    /// Tells the Navigator the answers on the servers inside repositories, when they are
    /// not what it was last told.
    fn tell_consents(&mut self, cx: &mut Context<Self>) {
        if !self.is_connected() {
            return;
        }
        let consents: Vec<RepositoryConsent> = self
            .settings
            .read(cx)
            .all()
            .iter()
            .filter_map(|setting| match setting {
                Setting::ServersInRepository {
                    repository,
                    allowed,
                } => Some(RepositoryConsent {
                    repository: repository.clone(),
                    allowed: *allowed,
                }),
                _ => None,
            })
            .collect();
        let Some(connection) = self.connection.as_mut() else {
            return;
        };
        if connection.told_consents.as_ref() != Some(&consents) {
            let _ = connection.asks.try_send(Ask::Consents(consents.clone()));
            connection.told_consents = Some(consents);
        }
    }

    /// Tells the Navigator which files changed, when the changes of the workspace on
    /// screen were just taken from an answer of the Keeper.
    fn tell_files_changed(&mut self, cx: &mut Context<Self>) {
        if !self.is_connected() {
            return;
        }
        let changes = self.changes.read(cx);
        // While an answer is awaited, or after one that was refused, the model shows the
        // last list again: that is not news.
        if changes.is_loading() || changes.refusal().is_some() {
            return;
        }
        let Some(taken) = changes.changes() else {
            return;
        };
        let workspace = taken.workspace.clone();
        let now = changed_paths(taken);
        let Some(connection) = self.connection.as_mut() else {
            return;
        };
        let before = connection
            .told_changes
            .insert(workspace.clone(), now.clone())
            .unwrap_or_default();
        let paths: Vec<PathBuf> = now.union(&before).cloned().collect();
        if !paths.is_empty() {
            let _ = connection
                .asks
                .try_send(Ask::FilesChanged { workspace, paths });
        }
    }
}

/// The paths a list of changes names: each file's, and the old path of a renamed file as
/// well, which changed on disk too: it is gone, and a server still holds what it had.
fn changed_paths(changes: &Changes) -> BTreeSet<PathBuf> {
    changes
        .files
        .iter()
        .flat_map(|file| {
            let from = match &file.status {
                FileStatus::Renamed { from } => Some(from.clone()),
                _ => None,
            };
            std::iter::once(file.path.clone()).chain(from)
        })
        .collect()
}
