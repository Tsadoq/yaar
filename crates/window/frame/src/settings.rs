//! The settings the user set, as the Keeper keeps them; the readers of app-wide settings give the
//! default of `yaar_link`'s table where none is set.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use yaar_link::{
    Fact, FlowForm, OptionKey, Reply, RepositoryId, Request, Setting, TerminalPlace, ThemeChoice,
    TurnEndSelects, WorkspaceId,
};

use crate::link_model::{AskError, KeeperLink, LinkStatus};

const DEFAULT_STOP_AFTER: Duration = Duration::from_secs(600);

/// In days.
const DEFAULT_KEEP_STATES: u32 = 30;

/// `set` takes a setting at once; a refusal has the list asked again. The Keeper echoes every set
/// to this Window too: two quick sets of one setting can show the first value for a moment, then
/// the list ends as the Keeper keeps it.
pub struct SettingList {
    link: Entity<KeeperLink>,
    settings: Vec<Setting>,
    listing: Option<Task<()>>,
    awaits_list: bool,
}

impl SettingList {
    pub fn new(link: Entity<KeeperLink>, cx: &mut Context<Self>) -> Self {
        // The link notifies only when an outcome is recorded, so connected here means a connection
        // not asked yet.
        cx.observe(&link, |list, link, cx| {
            if matches!(link.read(cx).status(), LinkStatus::Connected(_)) {
                list.refresh(cx);
            }
        })
        .detach();
        cx.subscribe(&link, |list, _, fact: &Fact, cx| {
            if let Fact::SettingChanged(setting) = fact {
                list.take_one(setting.clone(), cx);
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

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::GetSettings, cx));
        self.awaits_list = true;
        self.listing = Some(cx.spawn(async move |list, cx| {
            let answer = asked.await;
            list.update(cx, |list, cx| {
                list.awaits_list = false;
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

    pub fn link(&self) -> &Entity<KeeperLink> {
        &self.link
    }

    pub fn all(&self) -> &[Setting] {
        &self.settings
    }

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

    pub fn turn_end(&self) -> TurnEndSelects {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::TurnEnd(selects) => Some(*selects),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Ten minutes where none is set.
    pub fn stop_after(&self) -> Duration {
        self.stop_after_set().unwrap_or(DEFAULT_STOP_AFTER)
    }

    pub fn stop_after_set(&self) -> Option<Duration> {
        self.settings.iter().find_map(|setting| match setting {
            Setting::StopAfter { seconds } => Some(Duration::from_secs(u64::from(*seconds))),
            _ => None,
        })
    }

    pub fn theme_choice(&self) -> ThemeChoice {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::Theme(choice) => Some(choice.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn terminal_place(&self) -> TerminalPlace {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::TerminalPlace(place) => Some(*place),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn keep_states(&self) -> u32 {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::KeepStates { days } => Some(*days),
                _ => None,
            })
            .unwrap_or(DEFAULT_KEEP_STATES)
    }

    pub fn option_key(&self) -> OptionKey {
        self.settings
            .iter()
            .find_map(|setting| match setting {
                Setting::OptionKey(key) => Some(*key),
                _ => None,
            })
            .unwrap_or_default()
    }

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

    /// None where none is set, or where `folder: None` was set last: the app then makes them beside
    /// the repository.
    pub fn worktrees_folder(&self, repository: &RepositoryId) -> Option<PathBuf> {
        self.settings.iter().find_map(|setting| match setting {
            Setting::WorktreesFolder {
                repository: of,
                folder,
            } if of == repository => folder.clone(),
            _ => None,
        })
    }

    pub fn set(&mut self, setting: Setting, cx: &mut Context<Self>) {
        drop(self.put(setting, cx));
    }

    /// `Ok` once the Keeper answered `Reply::Done`. Otherwise `Err` holds a sentence for the user:
    /// the refusal, or [`NOT_KEPT`] when the link is gone or the answer is no answer to a set.
    /// Every failure but a lost link has the list asked again.
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

    /// After a refusal the list is asked again by a task of its own, so a caller that lets go of
    /// the outcome does not leave the list showing a setting that was not kept.
    fn put(
        &mut self,
        setting: Setting,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<Result<(), String>> {
        self.take_one(setting.clone(), cx);
        let asked = self
            .link
            .update(cx, |link, cx| link.ask(Request::SetSetting { setting }, cx));
        // An answer still on its way was made before the Keeper heard of this set and would take
        // the setting off the list: it is dropped, and the question is asked again after the set.
        if self.awaits_list {
            self.refresh(cx);
        }
        let (outcome, answered) = async_channel::bounded(1);
        cx.spawn(async move |list, cx| {
            let answer = asked.await;
            // Anything but `Done` may leave the list showing a setting the Keeper does not keep:
            // ask for what it keeps. A link that is gone has nothing to ask.
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
            let _ = outcome.send(result).await;
        })
        .detach();
        answered
    }
}

pub const NOT_KEPT: &str = "The app could not keep the setting: the Keeper did not answer.";
