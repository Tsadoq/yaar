//! Answers and facts are heard in no fixed order. Each model keeps one rule: of what it asked only
//! the answer to its last question counts, and a fact that makes an answer on its way doubtful has
//! it asked again.

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

#[derive(Clone)]
pub struct Models {
    pub link: Entity<KeeperLink>,
    pub workspaces: Entity<WorkspaceList>,
    pub changes: Entity<WorkspaceChanges>,
    pub agents: Entity<AgentList>,
    pub terminals: Entity<TerminalList>,
    pub settings: Entity<SettingList>,
    pub consents: Entity<ConsentList>,
    pub navigation: Entity<Navigation>,
    pub looks: Entity<Looks>,
    pub states: Entity<RecordedStates>,
    pub routes: Routes,
}

impl Models {
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

fn as_refusal(error: AskError) -> Refusal {
    match error {
        AskError::Refused(refusal) => refusal,
        other => Refusal::Failed {
            reason: other.to_string(),
        },
    }
}

/// The workspace on screen is the Window's own choice: the first of the list until the user selects
/// another, or one just opened. When it leaves the list, the first takes its place.
pub struct WorkspaceList {
    link: Entity<KeeperLink>,
    repositories: Vec<Repository>,
    selected: Option<WorkspaceId>,
    refusal: Option<Refusal>,
    /// A workspace the user opened that no list has held yet: it comes on screen with the list that
    /// does.
    wanted: Option<WorkspaceId>,
    listing: Option<Task<()>>,
    awaits_list: bool,
    asked_by_user: u64,
    told_shown: Option<Option<WorkspaceId>>,
}

impl WorkspaceList {
    pub fn want_on_screen(&mut self, workspace: WorkspaceId, cx: &mut Context<Self>) {
        if self.workspace(&workspace).is_some() {
            self.select(&workspace, cx);
        } else {
            self.wanted = Some(workspace);
        }
    }

    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        // The link notifies only when an outcome is recorded, so connected here means a connection
        // not asked yet.
        cx.observe(&link, |list, link, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Connected(_)) {
                list.refresh(cx);
                list.tell_shown(true, cx);
            }
        })
        .detach();
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::Workspaces(repositories) = fact {
                list.take(repositories.clone(), cx);
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

    pub fn repositories(&self) -> &[Repository] {
        &self.repositories
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Option<&Workspace> {
        workspaces(&self.repositories).find(|workspace| workspace.id == *id)
    }

    pub fn selected(&self) -> Option<&WorkspaceId> {
        self.selected.as_ref()
    }

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

    pub fn select_next(&mut self, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    pub fn select_previous(&mut self, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    /// The Frame calls it when the window comes to the front: a worktree made or a branch switched
    /// outside the app is in no fact.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::ListWorkspaces, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
                // A refusal has no place to show here: the list stays as it was, and what the user
                // asks next is refused with the same reason.
                if let Ok(Reply::Workspaces(repositories)) = answer {
                    list.take(repositories, cx);
                }
            })
            .ok();
        }));
    }

    /// The Keeper answers with the id of the workspace before it tells the list that holds it; the
    /// workspace comes on screen when both are here, whichever is first.
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

    pub fn set_base(&mut self, base: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected.clone() else {
            return;
        };
        self.ask_for_user(
            Request::SetBase { workspace, base },
            |_, reply, _| matches!(reply, Reply::Done),
            cx,
        );
    }

    pub fn refusal(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

    pub fn forget_refusal(&mut self, cx: &mut Context<Self>) {
        if self.refusal.take().is_some() {
            cx.notify();
        }
    }

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

    /// Tells the Keeper the workspace on screen, so it watches that worktree: when it changed, or
    /// always with `force`. The answer is not waited for: after a refusal the Keeper is told again
    /// only at the next change.
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
        // Not kept: the user may ask for the next thing before this one is answered, and both
        // answers count.
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

fn workspaces(repositories: &[Repository]) -> impl Iterator<Item = &Workspace> {
    repositories
        .iter()
        .flat_map(|repository| &repository.workspaces)
}

pub struct WorkspaceChanges {
    link: Entity<KeeperLink>,
    workspaces: Entity<WorkspaceList>,
    workspace: Option<WorkspaceId>,
    changes: Option<Changes>,
    loading: bool,
    refusal: Option<Refusal>,
    asking: Option<Task<()>>,
}

impl WorkspaceChanges {
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

    pub fn workspace(&self) -> Option<&WorkspaceId> {
        self.workspace.as_ref()
    }

    pub fn changes(&self) -> Option<&Changes> {
        self.changes.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn refusal(&self) -> Option<&Refusal> {
        self.refusal.as_ref()
    }

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

    /// The old version is asked for in the commit the changes are counted from, at the path the
    /// file had there; a new file has none. With no changes nothing is asked and the task resolves
    /// to a refusal.
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

    fn follow(&mut self, cx: &mut Context<Self>) {
        let on_screen = self.workspaces.read(cx).selected().cloned();
        if on_screen == self.workspace {
            return;
        }
        self.workspace = on_screen;
        self.changes = None;
        self.refusal = None;
        self.asking = None;
        self.loading = false;
        self.refresh(cx);
        cx.notify();
    }
}
