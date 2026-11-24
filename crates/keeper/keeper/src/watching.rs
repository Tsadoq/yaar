//! Which worktrees the Keeper watches, those a Window shows and those an agent runs in, and what it
//! tells of them.
//! The first watch records a state for credited changes and the first turn to count from.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use yaar_link::{Fact, Refusal, Reply, WorkspaceId};
use yaar_recorder::StateId;
use yaar_review_engine::StateWatch;
use yaar_workspaces::{Seen, WorkspaceError};

use crate::wiring::{self, Kept};
use crate::{Shared, Window, lock};

/// The shortest time between two facts of one workspace after the first: a Window reading the
/// changes is not made to start again at every save.
pub(crate) const CALM: Duration = Duration::from_millis(250);

/// How long before a state was taken a change may have been seen and still not be in it: recording
/// a large worktree takes over a second, and the engine tells of the state when it is done.
const RECORDING_MARGIN: Duration = Duration::from_secs(2);

pub(crate) struct Watching {
    core: Arc<Core>,
}

impl fmt::Debug for Watching {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Watching").finish_non_exhaustive()
    }
}

struct Core {
    shared: Weak<Shared>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    needs: Needs,
    watchers: HashMap<WorkspaceId, Calm>,
    first_state: HashSet<WorkspaceId>,
    freshness: Freshness,
}

impl Watching {
    pub(crate) fn new(shared: Weak<Shared>) -> Watching {
        Watching {
            core: Arc::new(Core {
                shared,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub(crate) fn show(
        &self,
        kept: &Kept,
        window: &Arc<Window>,
        workspace: Option<&WorkspaceId>,
    ) -> Result<Reply, Refusal> {
        // One the Keeper does not hold is refused; one whose worktree is gone is shown and has
        // nothing to watch.
        if let Some(workspace) = workspace {
            kept.workspaces
                .worktree(&store_id(workspace))
                .map_err(wiring::refusal)?;
        }
        let before = self
            .core
            .lock()
            .needs
            .show(key_of(window), workspace.cloned());
        for changed in before.iter().chain(workspace) {
            self.core.reconcile(kept, changed);
        }
        Ok(Reply::Done)
    }

    pub(crate) fn window_gone(&self, kept: &Kept, window: &Arc<Window>) {
        let before = self.core.lock().needs.window_gone(key_of(window));
        if let Some(workspace) = before {
            self.core.reconcile(kept, &workspace);
        }
    }

    pub(crate) fn agent_runs(&self, kept: &Kept, workspace: &WorkspaceId, runs: bool) {
        self.core.lock().needs.agent_runs(workspace, runs);
        self.core.reconcile(kept, workspace);
    }
}

impl StateWatch for Watching {
    fn changed_since_state(&self, workspace: &yaar_workspaces::WorkspaceId) -> bool {
        self.core
            .lock()
            .freshness
            .changed_since_state(&link_id(workspace))
    }

    fn state_taken(&self, workspace: &yaar_workspaces::WorkspaceId, _state: &StateId) {
        self.core
            .lock()
            .freshness
            .state_taken(&link_id(workspace), Instant::now());
    }
}

impl Core {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Watchers start on a thread of their own: the system may take long to set one up on a large
    /// worktree, and the request that needs it must not wait.
    fn reconcile(self: &Arc<Core>, kept: &Kept, workspace: &WorkspaceId) {
        let mut state = self.lock();
        let needed = state.needs.is_needed(workspace);
        let watched = state.watchers.contains_key(workspace);
        match (needed, watched) {
            (true, false) => {
                state.watchers.insert(workspace.clone(), Calm::default());
                drop(state);
                let (core, workspace) = (Arc::clone(self), workspace.clone());
                std::thread::spawn(move || core.start(&workspace));
            }
            (false, true) => {
                state.watchers.remove(workspace);
                // Changes made while nobody watches are not seen: a state taken before
                // says nothing of the files now.
                state.freshness.forget(workspace);
                drop(state);
                kept.workspaces.unwatch(&store_id(workspace));
            }
            _ => {}
        }
    }

    fn start(self: &Arc<Core>, workspace: &WorkspaceId) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        // The parts are held for as long as this runs, so that none ends under it: the
        // Keeper that is leaving waits for a recording to finish.
        let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
        let Some(kept) = parts.as_ref().and_then(|parts| parts.kept()) else {
            return;
        };
        if !self.lock().watchers.contains_key(workspace) {
            return;
        }
        let id = store_id(workspace);
        let told: Box<dyn Fn(Seen) + Send + Sync> = {
            let (core, workspace) = (Arc::clone(self), workspace.clone());
            Box::new(move |seen| core.seen(&workspace, seen))
        };
        if let Err(error) = kept.workspaces.watch(&id, told) {
            // A worktree that is gone has nothing to watch; what else went wrong is said.
            if !matches!(error, WorkspaceError::Refused(_)) {
                eprintln!("yaar-keeper: {workspace} cannot be watched: {error}");
            }
            self.lock().watchers.remove(workspace);
            return;
        }
        let first = {
            let mut state = self.lock();
            // The need went while the watcher was set up without the lock: let it go again.
            if !state.watchers.contains_key(workspace) {
                drop(state);
                kept.workspaces.unwatch(&id);
                return;
            }
            state.freshness.watched(workspace);
            state.first_state.insert(workspace.clone())
        };
        if first {
            self.record(kept, workspace);
        }
    }

    fn record(&self, kept: &Kept, workspace: &WorkspaceId) {
        let id = store_id(workspace);
        let Ok(Some(worktree)) = kept.workspaces.worktree(&id) else {
            return;
        };
        let taken = kept
            .reviews
            .states_of(&id)
            .map_err(|error| error.to_string())
            .and_then(|states| states.record(&worktree).map_err(|error| error.to_string()));
        match taken {
            Ok(_) => self.lock().freshness.state_taken(workspace, Instant::now()),
            Err(error) => eprintln!("yaar-keeper: no state of {workspace} was taken: {error}"),
        }
    }

    fn seen(self: &Arc<Core>, workspace: &WorkspaceId, seen: Seen) {
        let mut state = self.lock();
        state.freshness.changed(workspace, Instant::now());
        let Some(calm) = state.watchers.get_mut(workspace) else {
            return;
        };
        if calm.notice(seen) {
            drop(state);
            let (core, workspace) = (Arc::clone(self), workspace.clone());
            std::thread::spawn(move || core.tell_calmly(&workspace));
        }
    }

    fn tell_calmly(&self, workspace: &WorkspaceId) {
        loop {
            let step = match self.lock().watchers.get_mut(workspace) {
                Some(calm) => calm.step(Instant::now()),
                None => return,
            };
            match step {
                Step::Idle => return,
                Step::Wait(time) => std::thread::sleep(time),
                Step::Tell(seen) => self.tell(workspace, seen),
            }
        }
    }

    fn tell(&self, workspace: &WorkspaceId, seen: Seen) {
        let Some(shared) = self.shared.upgrade() else {
            return;
        };
        let changed = Fact::ChangesChanged {
            workspace: workspace.clone(),
        };
        match seen {
            Seen::Files => shared.tell(&[changed]),
            Seen::Head => {
                // The list is read and told under the turn every request that carries it waits for:
                // one read between two changes and told after the second would be taken for the
                // newer list.
                let _turn = lock(&shared.turn);
                let parts = shared.parts.read().unwrap_or_else(PoisonError::into_inner);
                let Some(kept) = parts.as_ref().and_then(|parts| parts.kept()) else {
                    return;
                };
                let mut facts = vec![changed];
                if let Ok(list) = wiring::list(&kept.workspaces) {
                    facts.push(Fact::Workspaces(list));
                }
                shared.tell(&facts);
            }
        }
    }
}

/// A pointer address: unique only while the Window lives, so its key must be dropped at
/// `Watching::window_gone`.
fn key_of(window: &Arc<Window>) -> usize {
    Arc::as_ptr(window) as usize
}

fn store_id(workspace: &WorkspaceId) -> yaar_workspaces::WorkspaceId {
    yaar_workspaces::WorkspaceId::new(workspace.as_str())
}

fn link_id(workspace: &yaar_workspaces::WorkspaceId) -> WorkspaceId {
    WorkspaceId::new(workspace.as_str())
}

#[derive(Default)]
struct Needs {
    shown: HashMap<usize, WorkspaceId>,
    running: HashSet<WorkspaceId>,
}

impl Needs {
    fn show(&mut self, window: usize, workspace: Option<WorkspaceId>) -> Option<WorkspaceId> {
        match workspace {
            Some(workspace) => self.shown.insert(window, workspace),
            None => self.shown.remove(&window),
        }
    }

    fn window_gone(&mut self, window: usize) -> Option<WorkspaceId> {
        self.shown.remove(&window)
    }

    fn agent_runs(&mut self, workspace: &WorkspaceId, runs: bool) {
        if runs {
            self.running.insert(workspace.clone());
        } else {
            self.running.remove(workspace);
        }
    }

    fn is_needed(&self, workspace: &WorkspaceId) -> bool {
        self.running.contains(workspace) || self.shown.values().any(|shown| shown == workspace)
    }
}

#[derive(Default)]
struct Calm {
    last: Option<Instant>,
    pending: Option<Seen>,
    telling: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Idle,
    Wait(Duration),
    Tell(Seen),
}

impl Calm {
    fn notice(&mut self, seen: Seen) -> bool {
        self.pending = Some(match (self.pending, seen) {
            (Some(Seen::Head), _) | (_, Seen::Head) => Seen::Head,
            _ => Seen::Files,
        });
        !std::mem::replace(&mut self.telling, true)
    }

    fn step(&mut self, now: Instant) -> Step {
        let Some(seen) = self.pending else {
            self.telling = false;
            return Step::Idle;
        };
        match self.last {
            Some(last) if now < last + CALM => Step::Wait(last + CALM - now),
            _ => {
                self.last = Some(now);
                self.pending = None;
                Step::Tell(seen)
            }
        }
    }
}

#[derive(Default)]
struct Freshness {
    watched: HashMap<WorkspaceId, Sight>,
}

#[derive(Default)]
struct Sight {
    changed: Option<Instant>,
    taken: Option<Instant>,
}

impl Freshness {
    fn watched(&mut self, workspace: &WorkspaceId) {
        self.watched.insert(workspace.clone(), Sight::default());
    }

    fn forget(&mut self, workspace: &WorkspaceId) {
        self.watched.remove(workspace);
    }

    fn changed(&mut self, workspace: &WorkspaceId, now: Instant) {
        if let Some(sight) = self.watched.get_mut(workspace) {
            sight.changed = Some(now);
        }
    }

    fn state_taken(&mut self, workspace: &WorkspaceId, now: Instant) {
        if let Some(sight) = self.watched.get_mut(workspace) {
            sight.taken = Some(now);
        }
    }

    /// True unless the watcher runs, a state was taken since it began, and the last change it saw
    /// was well before that state was taken.
    fn changed_since_state(&self, workspace: &WorkspaceId) -> bool {
        let Some(sight) = self.watched.get(workspace) else {
            return true;
        };
        match (sight.taken, sight.changed) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(taken), Some(changed)) => changed + RECORDING_MARGIN >= taken,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> WorkspaceId {
        WorkspaceId::new(name)
    }

    #[test]
    fn a_workspace_is_needed_while_a_window_shows_it_or_an_agent_runs_in_it() {
        let (a, b) = (workspace("a"), workspace("b"));
        let mut needs = Needs::default();
        assert!(!needs.is_needed(&a));

        assert_eq!(needs.show(1, Some(a.clone())), None);
        assert!(needs.is_needed(&a));
        assert!(!needs.is_needed(&b));

        needs.show(2, Some(a.clone()));
        assert_eq!(needs.show(1, Some(b.clone())), Some(a.clone()));
        assert!(needs.is_needed(&a), "the second Window still shows it");
        assert!(needs.is_needed(&b));
        assert_eq!(needs.window_gone(2), Some(a.clone()));
        assert!(!needs.is_needed(&a));

        needs.agent_runs(&a, true);
        assert!(needs.is_needed(&a));
        needs.agent_runs(&a, false);
        assert!(!needs.is_needed(&a));

        assert_eq!(needs.show(1, None), Some(b.clone()));
        assert!(!needs.is_needed(&b));
        assert_eq!(needs.window_gone(1), None);
    }

    #[test]
    fn a_workspace_that_an_agent_runs_in_stays_needed_when_the_window_that_showed_it_goes() {
        let a = workspace("a");
        let mut needs = Needs::default();
        needs.show(1, Some(a.clone()));
        needs.agent_runs(&a, true);
        assert_eq!(needs.window_gone(1), Some(a.clone()));
        assert!(needs.is_needed(&a));
    }

    #[test]
    fn the_first_change_is_told_at_once_and_those_after_it_once_in_each_calm_time() {
        let mut calm = Calm::default();
        let start = Instant::now();

        assert!(calm.notice(Seen::Files), "a thread has to tell the first");
        assert_eq!(calm.step(start), Step::Tell(Seen::Files));

        let soon = start + Duration::from_millis(40);
        assert!(!calm.notice(Seen::Files), "the thread that tells goes on");
        assert!(!calm.notice(Seen::Files));
        assert_eq!(
            calm.step(soon),
            Step::Wait(CALM - Duration::from_millis(40))
        );
        assert_eq!(calm.step(start + CALM), Step::Tell(Seen::Files));

        assert_eq!(calm.step(start + CALM * 2), Step::Idle);
        assert!(calm.notice(Seen::Files));
    }

    #[test]
    fn a_change_seen_after_a_quiet_time_is_told_at_once() {
        let mut calm = Calm::default();
        let start = Instant::now();
        calm.notice(Seen::Files);
        assert_eq!(calm.step(start), Step::Tell(Seen::Files));
        assert_eq!(calm.step(start), Step::Idle);

        let later = start + CALM * 4;
        assert!(calm.notice(Seen::Files));
        assert_eq!(calm.step(later), Step::Tell(Seen::Files));
    }

    #[test]
    fn a_change_of_what_is_checked_out_is_told_as_such_whatever_else_was_seen_with_it() {
        let start = Instant::now();
        for seen in [
            [Seen::Head, Seen::Files],
            [Seen::Files, Seen::Head],
            [Seen::Head, Seen::Head],
        ] {
            let mut calm = Calm::default();
            for one in seen {
                calm.notice(one);
            }
            assert_eq!(calm.step(start), Step::Tell(Seen::Head), "{seen:?}");
        }
        let mut calm = Calm::default();
        calm.notice(Seen::Files);
        calm.notice(Seen::Files);
        assert_eq!(calm.step(start), Step::Tell(Seen::Files));
    }

    #[test]
    fn files_may_have_changed_unless_the_watcher_runs_and_a_state_was_taken_after_the_last_change()
    {
        let a = workspace("a");
        let start = Instant::now();
        let mut freshness = Freshness::default();

        assert!(freshness.changed_since_state(&a));
        freshness.watched(&a);
        assert!(freshness.changed_since_state(&a));

        freshness.state_taken(&a, start);
        assert!(!freshness.changed_since_state(&a));

        freshness.changed(&a, start + Duration::from_secs(1));
        assert!(freshness.changed_since_state(&a));

        let later = start + RECORDING_MARGIN * 3;
        freshness.state_taken(&a, later);
        assert!(!freshness.changed_since_state(&a));

        // A change seen just before the state was taken may be missing from it: the
        // recording took the time between.
        freshness.changed(&a, later + Duration::from_secs(10));
        freshness.state_taken(&a, later + Duration::from_secs(11));
        assert!(freshness.changed_since_state(&a));

        assert!(freshness.changed_since_state(&workspace("b")));
        freshness.forget(&a);
        assert!(freshness.changed_since_state(&a));
        freshness.state_taken(&a, later);
        assert!(freshness.changed_since_state(&a), "nothing watches it");
    }
}
