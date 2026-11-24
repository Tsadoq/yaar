//! The theme the Window draws in: the user's choice, else the system's (told by `system_is`). A
//! chosen theme forces the window's appearance, so system changes are ignored; once let go, the
//! Window program rereads `App::window_appearance`, since the window's own keeps the forced value.

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;
use yaar_link::ThemeChoice;

use crate::settings::SettingList;
use crate::theme::{Appearance, ThemeDefinition, Themes, set_applied};

pub struct Looks {
    settings: Entity<SettingList>,
    themes: Themes,
    system: Option<Appearance>,
    applied: ThemeDefinition,
    forced: bool,
    done: bool,
}

impl Looks {
    /// Applies nothing until first told the system's setting.
    pub fn new(settings: Entity<SettingList>, cx: &mut Context<Self>) -> Looks {
        // Other changes of the list land here too and apply nothing new.
        cx.observe(&settings, |looks, _, cx| looks.apply(cx))
            .detach();
        let themes = Themes::built_in();
        let applied = themes
            .resolve(&ThemeChoice::System, Appearance::Light)
            .clone();
        Looks {
            settings,
            themes,
            system: None,
            applied,
            forced: false,
            done: false,
        }
    }

    pub fn system_is(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        self.system = Some(appearance);
        self.apply(cx);
    }

    pub fn system(&self) -> Option<Appearance> {
        self.system
    }

    pub fn chosen(&self, cx: &App) -> ThemeChoice {
        self.settings.read(cx).theme()
    }

    pub fn applied(&self) -> &ThemeDefinition {
        &self.applied
    }

    pub fn themes(&self) -> &Themes {
        &self.themes
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(system) = self.system else {
            return;
        };
        let choice = self.chosen(cx);
        let definition = self.themes.resolve(&choice, system).clone();
        let forced =
            matches!(&choice, ThemeChoice::Named(name) if self.themes.named(name).is_some());
        let same_definition = self.done && definition == self.applied;
        if same_definition && forced == self.forced {
            return;
        }
        if !same_definition {
            let mode = match definition.appearance {
                Appearance::Light => ThemeMode::Light,
                Appearance::Dark => ThemeMode::Dark,
            };
            Theme::change(mode, None, cx);
            // `change` loaded the toolkit's own colours over everything: the definition goes on
            // top, in a second update.
            Theme::update(cx, |theme| definition.onto(&mut theme.colors));
            set_applied(definition.clone(), cx);
        }
        cx.set_window_appearance(forced.then_some(match definition.appearance {
            Appearance::Light => WindowAppearance::Light,
            Appearance::Dark => WindowAppearance::Dark,
        }));
        self.applied = definition;
        self.forced = forced;
        self.done = true;
        cx.notify();
    }
}
