//! The shared model of the settings the user set, as the Keeper keeps them.
//!
//! Filled from the Keeper when the link connects, kept in step by
//! `Fact::SettingChanged`, and changed by a pane that sets one. The review pane reads the
//! form of Flow of the workspace on screen from it, and what a turn's end selects; the
//! settings screen reads and sets every kind through the same model; the Frame reads the
//! place of the terminal pane, `crate::Looks` the theme, the model `crate::Navigation` the
//! stop time, the terminal pane what Option does.
//!
//! Each reader of a setting of the whole app reads it through one call below, which gives
//! the default of `yaar_link`'s table where the user set none.
//!
//! The folder the user named for the worktrees of a repository is read by
//! [`SettingList::worktrees_folder`] and set from the settings screen by
//! [`SettingList::set_answered`], which, unlike [`SettingList::set`], gives the Keeper's
//! answer: the Keeper refuses a folder it cannot take, and the screen shows why.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use yaar_link::{
    Fact, FlowForm, OptionKey, Reply, RepositoryId, Request, Setting, TerminalPlace, ThemeChoice,
    TurnEndSelects, WorkspaceId,
};

use crate::link_model::{AskError, KeeperLink, LinkStatus};

/// The stop time of the Link's table, which holds where the user set none.
const DEFAULT_STOP_AFTER: Duration = Duration::from_secs(600);

/// The days a state is kept by the Link's table, where the user set none.
const DEFAULT_KEEP_STATES: u32 = 30;

/// The settings the user set, as the Keeper told them.
///
/// When the link connects it asks `Request::GetSettings` and takes the answer; each
/// `Fact::SettingChanged` takes the place of the setting it is the same thing as
/// (`Setting::same_thing_as`), or comes last. Of two answers, only that to the last
/// question counts, and a fact heard while a question is on its way has the question
/// asked again, as [`crate::AgentList`] does. A Keeper that refuses leaves the list as it
/// was: every setting then holds its default.
///
/// [`SettingList::set`] takes a setting at once, so that what the user chose shows before
/// the Keeper answered, and asks the Keeper to keep it with `Request::SetSetting`, which is
/// the user's request. A refusal has the list asked again, so that the model shows what
/// the Keeper keeps; so does a set made while an answer for the list is on its way, whose
/// answer is not held, since the Keeper made it before it heard of the set.
///
/// The Keeper tells `SettingChanged` to every Window, this one too, after it answered the
/// set. Each fact is taken as it comes, the echo of this window's own set included: two
/// sets of one setting in quick succession are echoed in the order they were kept, so the
/// first echo can bring back the first value for a moment, until the second echo comes.
/// That is accepted: the list ends as the Keeper keeps it, and no set is lost.
pub struct SettingList {
    link: Entity<KeeperLink>,
    settings: Vec<Setting>,
    /// The last question for the list. A new one takes its place, which drops the answer
    /// to the old one.
    listing: Option<Task<()>>,
    /// Whether the answer to that question is still awaited.
    awaits_list: bool,
}

impl SettingList {
    /// The list of one window, fed by `link`: empty until the Keeper answers.
    ///
    /// It asks when the link connects, and hears the facts, as [`crate::AgentList::new`]
    /// does.
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        // The link tells of a change only when the outcome of reaching a Keeper is
        // recorded, so being connected then means a connection that was not asked yet.
        cx.observe(&link, |list, link, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Connected(_)) {
                list.refresh(cx);
            }
        })
        .detach();
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::SettingChanged(setting) = fact {
                list.take_one(setting.clone(), cx);
                // An answer still on its way may have been made before this fact was.
                // It is not waited for; the question is asked again, and what answers
                // it is no older than this fact.
                if list.awaits_list {
                    list.refresh(cx);
                }
            }
        })
        .detach();

        let connected = matches!(link.read(cx).status(), LinkStatus::Connected(_));
        let mut list = Self {
            link,
            settings: Vec::new(),
            listing: None,
            awaits_list: false,
        };
        if connected {
            list.refresh(cx);
        }
        list
    }

    /// Asks the Keeper for every setting, and takes the answer unless a later question
    /// took its place.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::GetSettings, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
                // A Keeper that refuses, as the one of the contract does, leaves the
                // list as it was: every setting then holds its default.
                if let Ok(Reply::Settings(settings)) = answer
                    && settings != list.settings
                {
                    list.settings = settings;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Takes what the Keeper told of one setting: it replaces the one it is the same
    /// thing as, or comes last.
    fn take_one(&mut self, told: Setting, cx: &mut Context<Self>) {
        match self
            .settings
            .iter_mut()
            .find(|held| held.same_thing_as(&told))
        {
            Some(held) if *held == told => return,
            Some(held) => *held = told,
            None => self.settings.push(told),
        }
        cx.notify();
    }

    /// The link this list is fed by.
    pub fn link(&self) -> &Entity<KeeperLink> {
        &self.link
    }

    /// Every setting the user set, in the order the Keeper listed them, those set since
    /// in the place of the one they are the same thing as, or last.
    pub fn all(&self) -> &[Setting] {
        &self.settings
    }

    /// The form of Flow chosen last in `workspace`: [`FlowForm::Timeline`], the default,
    /// where none was chosen.
    pub fn flow_form(&self, workspace: &WorkspaceId) -> FlowForm {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::FlowForm {
                    workspace: of,
                    form,
                } if of == workspace => Some(*form),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// What a turn's end selects in Changes: [`TurnEndSelects::WhatChanged`] where none is
    /// set.
    pub fn turn_end(&self) -> TurnEndSelects {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::TurnEnd(selects) => Some(*selects),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The time a workspace stays off screen before its servers stop: ten minutes where none
    /// is set.
    pub fn stop_after(&self) -> Duration {
        self.stop_after_set().unwrap_or(DEFAULT_STOP_AFTER)
    }

    /// Whether the user set the stop time: the model `crate::Navigation` tells the Navigator
    /// only a time the user set.
    pub fn stop_after_set(&self) -> Option<Duration> {
        self.settings.iter().find_map(|setting| match setting {
            Setting::StopAfter { seconds } => Some(Duration::from_secs(u64::from(*seconds))),
            _ => None,
        })
    }

    /// The user's choice of theme: [`ThemeChoice::System`] where none is set.
    pub fn theme(&self) -> ThemeChoice {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::Theme(choice) => Some(choice.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Where the terminal pane lies: [`TerminalPlace::Right`] where none is set.
    pub fn terminal_place(&self) -> TerminalPlace {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::TerminalPlace(place) => Some(*place),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// How long a recorded state nothing refers to is kept, in days: 30 where none is set.
    pub fn keep_states(&self) -> u32 {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::KeepStates { days } => Some(*days),
                _ => None,
            })
            .unwrap_or(DEFAULT_KEEP_STATES)
    }

    /// What Option does in a terminal: [`OptionKey::ByLayout`] where none is set.
    pub fn option_key(&self) -> OptionKey {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::OptionKey(key) => Some(*key),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Every repository the user answered on its language servers, with the answer, in the
    /// order the Keeper listed them.
    pub fn servers_answers(&self) -> Vec<(RepositoryId, bool)> {
        self.settings
            .iter()
            .filter_map(|setting| match setting {
                Setting::ServersInRepository {
                    repository,
                    allowed,
                } => Some((repository.clone(), *allowed)),
                _ => None,
            })
            .collect()
    }

    /// The folder the user named for the worktrees the app creates for `repository`; none
    /// where none is set, or where `folder: None` was set last: the app then makes them
    /// beside the repository, where it keeps its worktrees.
    pub fn worktrees_folder(&self, repository: &RepositoryId) -> Option<PathBuf> {
        self.settings.iter().find_map(|setting| match setting {
            Setting::WorktreesFolder {
                repository: of,
                folder,
            } if of == repository => folder.clone(),
            _ => None,
        })
    }

    /// Takes `setting` in the place of the one it is the same thing as, or last, tells the
    /// observers, and asks the Keeper to keep it with `Request::SetSetting`. The answer is
    /// not waited for; a refusal has the list asked again.
    pub fn set(&mut self, setting: Setting, cx: &mut Context<Self>) {
        // Nobody waits for the answer: the receiver is let go.
        drop(self.put(setting, cx));
    }

    /// As [`SettingList::set`]: takes `setting` at once and asks the Keeper to keep it;
    /// and gives the answer. `Ok` once the Keeper answered `Reply::Done`. Otherwise the
    /// list is asked again, as `set` does, and the task gives `Err` with a sentence to show
    /// the user: the reason of a `Refusal::Failed`, the `Display` of any other refusal, or
    /// [`NOT_KEPT`] when the link is gone or the answer is no answer to a set.
    pub fn set_answered(
        &mut self,
        setting: Setting,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), String>> {
        let answered = self.put(setting, cx);
        cx.spawn(async move |_, _| {
            answered
                .recv()
                .await
                .unwrap_or_else(|_| Err(NOT_KEPT.to_owned()))
        })
    }

    /// Takes `setting`, asks the Keeper to keep it, and gives where the outcome will be
    /// sent. The list is asked again after a refusal by a task of its own, so that a caller
    /// that lets go of the outcome does not leave the list showing a setting that was not
    /// kept.
    fn put(
        &mut self,
        setting: Setting,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<Result<(), String>> {
        self.take_one(setting.clone(), cx);
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::SetSetting { setting }, cx));
        // An answer still on its way was made before the Keeper heard of this set, and
        // would take the setting off the list. It is dropped, and the question is asked
        // again, after the set, as for a fact.
        if self.awaits_list {
            self.refresh(cx);
        }
        let (outcome, answered) = async_channel::bounded(1);
        cx.spawn(async move |list, cx| {
            let answer = asked.await;
            // A Keeper that refuses, or answers with anything but `Done`, leaves the list
            // showing a setting it may not keep: ask for what it does keep. A link that is
            // gone has nothing to ask.
            let result = match answer {
                Ok(Reply::Done) => Ok(()),
                Err(AskError::NotLinked) => Err(NOT_KEPT.to_owned()),
                Err(AskError::Refused(refusal)) => {
                    list.update(cx, |list, cx| list.refresh(cx)).ok();
                    Err(refusal.to_string())
                }
                Ok(_) | Err(AskError::Unexpected) => {
                    list.update(cx, |list, cx| list.refresh(cx)).ok();
                    Err(NOT_KEPT.to_owned())
                }
            };
            // A caller that let go of the outcome wants none.
            let _ = outcome.send(result).await;
        })
        .detach();
        answered
    }
}

/// The sentence [`SettingList::set_answered`] gives when the Keeper gave no answer to the
/// set.
pub const NOT_KEPT: &str = "The app could not keep the setting: the Keeper did not answer.";
