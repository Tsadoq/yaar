//! The shared models of the Window: what the Keeper told, kept where every pane can
//! observe it.
//!
//! A pane is handed the [`Models`] when it is built. It observes the models it shows,
//! and asks the Keeper through them; it holds no other pane and no connection of its own.
//!
//! An answer of the Keeper and a fact it tells are heard in no fixed order, and neither
//! are two answers. Each model therefore keeps one rule: of what it asked, only the
//! answer to its last question counts; and a fact that makes an answer on its way
//! doubtful has the question asked again.

use std::path::PathBuf;

use gpui_kit::*;
use yaar_link::{
    ChangedFile, Changes, Fact, FileSides, FileStatus, Refusal, Reply, Repository, Request,
    Workspace, WorkspaceId,
};

use crate::agents::{AgentList, TerminalList};
use crate::consents::ConsentList;
use crate::link_model::{AskError, KeeperLink, LinkStatus};
use crate::looks::Looks;
use crate::navigation::Navigation;
use crate::routes::Routes;
use crate::settings::SettingList;
use crate::states::RecordedStates;

/// The shared models, as one handle. Cheap to clone.
#[derive(Clone)]
pub struct Models {
    /// How the Window stands with the Keeper, and the way to ask it.
    pub link: Entity<KeeperLink>,
    /// The repositories and workspaces, and the workspace on screen.
    pub workspaces: Entity<WorkspaceList>,
    /// The changes of the workspace on screen.
    pub changes: Entity<WorkspaceChanges>,
    /// The agent sessions of every workspace, and the state each workspace shows.
    pub agents: Entity<AgentList>,
    /// The terminals the Keeper holds that this Window shows or may show.
    pub terminals: Entity<TerminalList>,
    /// The settings the user set, the form of Flow of each workspace among them.
    pub settings: Entity<SettingList>,
    /// The questions for leave of the Adapters that write into a CLI's own settings.
    pub consents: Entity<ConsentList>,
    /// The Window's way to the Navigator: no Navigator is connected until the Window
    /// program connects one.
    pub navigation: Entity<Navigation>,
    /// The theme the Window draws in, from the user's choice and the system's setting.
    pub looks: Entity<Looks>,
    /// The room the recorded states take, and their clean-up.
    pub states: Entity<RecordedStates>,
    /// Where a pane puts the actions it takes from wherever the focus is in the window.
    pub routes: Routes,
}

impl Models {
    /// The models of one window, fed by `link`.
    pub fn new(link: Entity<KeeperLink>, cx: &mut App) -> Models {
        let workspaces = cx.new(|cx| WorkspaceList::new(link.clone(), cx));
        let changes = cx.new(|cx| WorkspaceChanges::new(link.clone(), workspaces.clone(), cx));
        let agents = cx.new(|cx| AgentList::new(link.clone(), cx));
        let terminals = cx.new(|cx| TerminalList::new(link.clone(), cx));
        let settings = cx.new(|cx| SettingList::new(link.clone(), cx));
        let consents = cx.new(|cx| ConsentList::new(link.clone(), cx));
        let navigation = cx.new(|cx| {
            Navigation::new(
                link.clone(),
                workspaces.clone(),
                changes.clone(),
                settings.clone(),
                cx,
            )
        });
        let looks = cx.new(|cx| Looks::new(settings.clone(), cx));
        let states = cx.new(|cx| RecordedStates::new(link.clone(), cx));
        Models {
            link,
            workspaces,
            changes,
            agents,
            terminals,
            settings,
            consents,
            navigation,
            looks,
            states,
            routes: Routes::default(),
        }
    }
}

/// What the user is shown when something asked of the Keeper brought no answer: the
/// Keeper's own refusal, or the reason there was none, as a failure.
fn as_refusal(error: AskError) -> Refusal {
    match error {
        AskError::Refused(refusal) => refusal,
        other => Refusal::Failed {
            reason: other.to_string(),
        },
    }
}

/// The repositories the user opened, their worktrees, and which workspace is on screen.
///
/// It asks the Keeper for the list when the link is connected and whenever
/// [`WorkspaceList::refresh`] is called, and takes every list the Keeper tells in
/// between. The workspace on screen is the Window's own choice: the first of the list
/// until the user selects another, and a workspace the user just opened. When that
/// workspace leaves the list, the first of the list takes its place.
pub struct WorkspaceList {
    link: Entity<KeeperLink>,
    repositories: Vec<Repository>,
    selected: Option<WorkspaceId>,
    refusal: Option<Refusal>,
    /// A workspace the user opened that no list has held yet: it comes on screen with
    /// the list that does.
    wanted: Option<WorkspaceId>,
    /// The last question for the list. A new one takes its place, which drops the
    /// answer to the old one.
    listing: Option<Task<()>>,
    /// Whether the answer to that question is still awaited.
    awaits_list: bool,
    /// Counts what the user asked here, so that a refusal can tell whether it still
    /// answers the last of it.
    asked_by_user: u64,
    /// What the Keeper was last told is on screen; none while it was told nothing.
    told_shown: Option<Option<WorkspaceId>>,
}

impl WorkspaceList {
    /// Puts `workspace` on screen with the first list that holds it, as a workspace the user
    /// just opened comes on screen: at once when the list holds it already. What a worktree
    /// made with an agent needs, whose workspace the Keeper names in its reply before it tells
    /// the list.
    pub fn want_on_screen(&mut self, workspace: WorkspaceId, cx: &mut Context<Self>) {
        if self.workspace(&workspace).is_some() {
            self.select(&workspace, cx);
        } else {
            self.wanted = Some(workspace);
        }
    }

    /// An empty list, fed by `link`.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        // The link tells of a change only when the outcome of reaching a Keeper is
        // recorded, so being connected then means a connection that was not asked yet.
        cx.observe(&link, |list, link, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Connected(_)) {
                list.refresh(cx);
                // A Keeper just reached knows nothing of what is on screen.
                list.tell_shown(true, cx);
            }
        })
        .detach();
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::Workspaces(repositories) = fact {
                list.take(repositories.clone(), cx);
                // An answer still on its way may have been made before this list was.
                // It is not waited for; the question is asked again, and what answers
                // it is no older than this list.
                if list.awaits_list {
                    list.refresh(cx);
                }
            }
        })
        .detach();

        let connected = matches!(link.read(cx).status(), LinkStatus::Connected(_));
        let mut list = Self {
            link,
            repositories: Vec::new(),
            selected: None,
            refusal: None,
            wanted: None,
            listing: None,
            awaits_list: false,
            asked_by_user: 0,
            told_shown: None,
        };
        if connected {
            list.refresh(cx);
            list.tell_shown(true, cx);
        }
        list
    }

    /// The repositories, each with its worktrees, as the Keeper last told them.
    pub fn repositories(&self) -> &[Repository] {
        &self.repositories
    }

    /// The workspace with this id, if the list holds it.
    pub fn workspace(&self, id: &WorkspaceId) -> Option<&Workspace> {
        workspaces(&self.repositories).find(|workspace| workspace.id == *id)
    }

    /// The workspace on screen.
    pub fn selected(&self) -> Option<&WorkspaceId> {
        self.selected.as_ref()
    }

    /// Puts a workspace of the list on screen. Does nothing for an id the list does not
    /// hold.
    pub fn select(&mut self, workspace: &WorkspaceId, cx: &mut Context<Self>) {
        if self.workspace(workspace).is_none() {
            return;
        }
        // What the user selects now outranks a folder opened before.
        self.wanted = None;
        if self.selected.as_ref() != Some(workspace) {
            self.selected = Some(workspace.clone());
            self.tell_shown(false, cx);
            cx.notify();
        }
    }

    /// Puts the next workspace of the list on screen, in the order the sidebar shows
    /// them; after the last comes the first.
    pub fn select_next(&mut self, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    /// Puts the previous workspace of the list on screen; before the first comes the
    /// last.
    pub fn select_previous(&mut self, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    /// Asks the Keeper again for the list. The Frame calls it when the window comes to
    /// the front: a worktree made or a branch switched outside the app is in no fact.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
                // A refusal has no place to show here: the list stays as it was, and
                // what the user asks next is refused with the same reason.
                if let Ok(Reply::Workspaces(repositories)) = answer {
                    list.take(repositories, cx);
                }
            })
            .ok();
        }));
    }

    /// Asks the Keeper to open a folder. When it is opened, its workspace is put on
    /// screen; when it is refused, [`WorkspaceList::refusal`] says why.
    ///
    /// The Keeper answers with the id of the workspace before it tells the list that
    /// holds it. The workspace comes on screen when both are here, whichever is first.
    pub fn open_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.ask_for_user(
            Request::OpenFolder { path: folder },
            |list, reply, cx| match reply {
                Reply::Opened { workspace, .. } => {
                    if list.workspace(&workspace).is_some() {
                        list.select(&workspace, cx);
                    } else {
                        list.wanted = Some(workspace);
                    }
                    true
                }
                _ => false,
            },
            cx,
        );
    }

    /// Asks the Keeper to count the changes of the workspace on screen from this branch
    /// or commit. When it is refused, [`WorkspaceList::refusal`] says why. Does nothing
    /// while no workspace is on screen.
    pub fn set_base(&mut self, base: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected.clone() else {
            return;
        };
        // The Keeper tells every Window that the changes changed; nothing is left to
        // do here when it is done.
        self.ask_for_user(
            Request::SetBase { workspace, base },
            |_, reply, _| matches!(reply, Reply::Done),
            cx,
        );
    }

    /// Why the Keeper refused the last thing the user asked of it here; none once the
    /// user asks again.
    ///
    /// Something that was asked and brought no answer at all, because the Keeper is not
    /// reached or answered with something else, is told the same way: as
    /// [`Refusal::Failed`] with the reason.
    pub fn refusal(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

    /// Takes a list of the Keeper, asked for or told, and settles which workspace is on
    /// screen with it.
    fn take(&mut self, repositories: Vec<Repository>, cx: &mut Context<Self>) {
        let held = |id: &&WorkspaceId| workspaces(&repositories).any(|held| held.id == **id);
        let selected = self
            .wanted
            .as_ref()
            .filter(held)
            .or(self.selected.as_ref().filter(held))
            .cloned()
            .or_else(|| {
                workspaces(&repositories)
                    .next()
                    .map(|first| first.id.clone())
            });
        if self.wanted.is_some() && self.wanted == selected {
            self.wanted = None;
        }
        if repositories != self.repositories || selected != self.selected {
            self.repositories = repositories;
            self.selected = selected;
            self.tell_shown(false, cx);
            cx.notify();
        }
    }

    /// Tells the Keeper which workspace is on screen, so that it watches that worktree:
    /// when it is not what the Keeper was last told, or always with `force`, as to a
    /// Keeper just reached. The answer is not waited for. A Keeper that refuses, as the
    /// one of the contract does, is told again only at the next change.
    fn tell_shown(&mut self, force: bool, cx: &mut Context<Self>) {
        if !matches!(self.link.read(cx).status(), LinkStatus::Connected(_)) {
            return;
        }
        if !force && self.told_shown.as_ref() == Some(&self.selected) {
            return;
        }
        self.told_shown = Some(self.selected.clone());
        let workspace = self.selected.clone();
        self.link
            .update(cx, |link, cx| {
                link.ask(Request::ShowWorkspace { workspace }, cx)
            })
            .detach();
    }

    /// Moves the workspace on screen by `by` places in the order of the list, around
    /// its ends.
    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let ids: Vec<&WorkspaceId> = workspaces(&self.repositories)
            .map(|workspace| &workspace.id)
            .collect();
        let Some(count) = isize::try_from(ids.len()).ok().filter(|count| *count > 0) else {
            return;
        };
        let now = ids
            .iter()
            .position(|id| Some(*id) == self.selected.as_ref());
        let next = match now {
            Some(now) => (now as isize + by).rem_euclid(count) as usize,
            None => 0,
        };
        let next = ids[next].clone();
        self.select(&next, cx);
    }

    /// Asks the Keeper for something the user asked for. `done` takes the reply, and
    /// says whether it was of the kind the request has.
    fn ask_for_user(
        &mut self,
        request: Request,
        done: impl FnOnce(&mut Self, Reply, &mut Context<Self>) -> bool + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.refusal.take().is_some() {
            cx.notify();
        }
        self.asked_by_user += 1;
        let number = self.asked_by_user;
        let asked = self.link.update(cx, |link, cx| link.ask(request, cx));
        // Not kept: the user may ask for the next thing before this one is answered,
        // and both answers count.
        cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                let error = match answer {
                    Ok(reply) => {
                        if done(list, reply, cx) {
                            return;
                        }
                        AskError::Unexpected
                    }
                    Err(error) => error,
                };
                // A refusal of something the user has asked past is no longer news.
                if list.asked_by_user == number {
                    list.refusal = Some(as_refusal(error));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

/// The workspaces of a list, in the order the sidebar shows them.
fn workspaces(repositories: &[Repository]) -> impl Iterator<Item = &Workspace> {
    repositories
        .iter()
        .flat_map(|repository| &repository.workspaces)
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
    /// The last question for the changes. A new one takes its place, which drops the
    /// answer to the old one: of two answers, only the one asked for last is shown.
    asking: Option<Task<()>>,
}

impl WorkspaceChanges {
    /// No changes yet; follows the workspace on screen of `workspaces`.
    pub fn new(
        link: Entity<KeeperLink>,
        workspaces: Entity<WorkspaceList>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&workspaces, |changes, _, cx| changes.follow(cx))
            .detach();
        cx.subscribe(&link, |changes, _, fact: &Fact, cx| {
            if let Fact::ChangesChanged { workspace } = fact
                && changes.workspace.as_ref() == Some(workspace)
            {
                changes.refresh(cx);
            }
        })
        .detach();

        let mut changes = Self {
            link,
            workspaces,
            workspace: None,
            changes: None,
            loading: false,
            refusal: None,
            asking: None,
        };
        changes.follow(cx);
        changes
    }

    /// The workspace these are the changes of: the one on screen.
    pub fn workspace(&self) -> Option<&WorkspaceId> {
        self.workspace.as_ref()
    }

    /// The changes, as the Keeper last answered for this workspace; none before the
    /// first answer.
    pub fn changes(&self) -> Option<&Changes> {
        self.changes.as_ref()
    }

    /// Whether an answer of the Keeper is still awaited.
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Why the Keeper refused the last time it was asked; none once it answers.
    ///
    /// A question that brought no answer at all, because the connection ended or the
    /// Keeper answered with something else, is told the same way: as
    /// [`Refusal::Failed`] with the reason.
    pub fn refusal(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

    /// Asks the Keeper again for the changes of the workspace on screen. The Frame calls
    /// it when the window comes to the front, since files change while nobody looks.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let asked = self.link.update(cx, |link, cx| {
            link.ask(
                Request::GetChanges {
                    workspace: workspace.clone(),
                },
                cx,
            )
        });
        if !self.loading {
            self.loading = true;
            cx.notify();
        }
        self.asking = Some(cx.spawn(async move |changes, cx| {
            let answer = asked.await;
            changes
                .update(cx, |changes, cx| {
                    changes.loading = false;
                    match answer {
                        Ok(Reply::Changes(answered)) if answered.workspace == workspace => {
                            changes.changes = Some(answered);
                            changes.refusal = None;
                        }
                        Ok(_) => changes.refusal = Some(as_refusal(AskError::Unexpected)),
                        Err(error) => changes.refusal = Some(as_refusal(error)),
                    }
                    cx.notify();
                })
                .ok();
        }));
    }

    /// Asks the Keeper for the two versions of one changed file of the workspace on
    /// screen: how a pane gets what it diffs. `file` is a file of
    /// [`WorkspaceChanges::changes`].
    ///
    /// The old version is asked for in the commit those changes are counted from, at the
    /// path the file had there, so the two versions belong to the list the pane holds.
    /// A new file has no old version, and none is asked for.
    ///
    /// While there are no changes there is no file of them: nothing is asked, and the
    /// task resolves to a refusal that says so.
    pub fn file(
        &self,
        file: &ChangedFile,
        cx: &mut Context<Self>,
    ) -> Task<Result<FileSides, AskError>> {
        let (Some(workspace), Some(changes)) = (&self.workspace, &self.changes) else {
            return Task::ready(Err(AskError::Refused(Refusal::Failed {
                reason: "no changes are shown that the file could be one of".to_owned(),
            })));
        };
        let (since, old_path) = match &file.status {
            FileStatus::Added => (None, file.path.clone()),
            FileStatus::Renamed { from } => (changes.counted_from.clone(), from.clone()),
            FileStatus::Modified | FileStatus::Deleted => {
                (changes.counted_from.clone(), file.path.clone())
            }
        };
        let request = Request::ReadFile {
            workspace: workspace.clone(),
            since,
            old_path,
            path: file.path.clone(),
        };
        let asked = self.link.update(cx, |link, cx| link.ask(request, cx));
        cx.spawn(async move |_, _| match asked.await {
            Ok(Reply::File(sides)) => Ok(sides),
            Ok(_) => Err(AskError::Unexpected),
            Err(error) => Err(error),
        })
    }

    /// Takes up the workspace on screen, when it is another than before.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let on_screen = self.workspaces.read(cx).selected().cloned();
        if on_screen == self.workspace {
            return;
        }
        // Nothing of the workspace that left the screen is shown under the one that
        // came, and no answer about it is waited for.
        self.workspace = on_screen;
        self.changes = None;
        self.refusal = None;
        self.asking = None;
        self.loading = false;
        self.refresh(cx);
        cx.notify();
    }
}
