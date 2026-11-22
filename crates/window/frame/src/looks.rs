//! The shared model of the theme the Window draws in (NFR-017, NFR-018).
//!
//! [`Looks`] holds the themes the Window can apply (`crate::theme::Themes::built_in`: a theme
//! is added to the app as one more definition there), the system's light or dark setting as
//! the Window last told it ([`Looks::system_is`]), and the user's
//! choice, which it reads from the settings (`SettingList::theme`, `Setting::Theme`, kept
//! by the Keeper and told to every Window). From the two it applies one definition
//! (`Themes::resolve`), and applies it again whenever either changes:
//!
//! | Choice | System changes | What is applied |
//! | --- | --- | --- |
//! | none set, or `ThemeChoice::System`, or a name no definition has | followed | the built-in definition of the system's appearance |
//! | `ThemeChoice::Named(n)` of a definition | not followed | that definition |
//!
//! Applying a definition, in this order:
//! 1. `gpui_kit::component::theme::Theme::change` to the definition's appearance (the
//!    toolkit's light or dark mode, which loads the toolkit's own colours of that mode for
//!    the roles the definition does not hold);
//! 2. `Theme::update`, a second call, writing [`crate::theme::ThemeDefinition::onto`] into
//!    the toolkit's colours (the toolkit's doc: colours edited after a change of mode go in
//!    a second update);
//! 3. the definition kept as the global [`crate::theme::applied`] reads;
//! 4. on macOS, the window's own appearance: `App::set_window_appearance(Some(..))` with the
//!    definition's appearance while a theme is chosen, so that the window's frame matches,
//!    and `None` while the system is followed, so that the system's changes reach the
//!    window again. (A forced appearance stops them: the toolkit's doc of that call.)
//!
//! Every window is drawn again by the toolkit's update. Applying the definition applied
//! already does nothing, and tells nobody.
//!
//! The system's setting comes from the Window program, which tells it at the window's
//! opening (`Window::appearance`) and at every change the window observes
//! (`Window::observe_window_appearance`). While a theme is chosen the observed appearance
//! is the forced one, so it is noted and not acted on; when the choice goes back to the
//! system, the Window program reads `Window::appearance` again once the forced one is let
//! go, and tells it.
//!
//! A test plays the system at [`Looks::system_is`]: GPUI's test platform reports a light
//! system and gives a test no way to change it (its window's `simulate_appearance_change`
//! is not public), so a component test tells the model what the platform would. That the
//! Window program tells it at the opening is shown by the composed Window's test: until
//! told, [`Looks::system`] is none.

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;
use yaar_link::ThemeChoice;

use crate::settings::SettingList;
use crate::theme::{Appearance, ThemeDefinition, Themes, set_applied};

/// The theme the Window draws in, and why.
pub struct Looks {
    settings: Entity<SettingList>,
    themes: Themes,
    /// The system's setting, as last told; none until told.
    system: Option<Appearance>,
    /// The definition applied, and whether it was applied as a chosen theme (the window's
    /// own appearance forced) or as the system's. Before the first application the light
    /// definition, as [`crate::theme::applied`] reads it.
    applied: ThemeDefinition,
    forced: bool,
    /// Whether the first application was made.
    done: bool,
}

impl Looks {
    /// The model of one window, reading the user's choice from `settings`. It applies
    /// nothing until it is first told the system's setting.
    pub fn new(settings: Entity<SettingList>, cx: &mut Context<Self>) -> Looks {
        // A choice that changes is applied at once; every other change of the list lands
        // here too, and applies what is applied already, which does nothing.
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

    /// The system's light or dark setting is `appearance` now: applies what the module
    /// doc's table says.
    pub fn system_is(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        self.system = Some(appearance);
        self.apply(cx);
    }

    /// The system's setting, as last told; none until [`Looks::system_is`] was first called.
    pub fn system(&self) -> Option<Appearance> {
        self.system
    }

    /// The user's choice, as the settings hold it: [`ThemeChoice::System`] where none is
    /// set.
    pub fn chosen(&self, cx: &App) -> ThemeChoice {
        self.settings.read(cx).theme()
    }

    /// The definition applied now.
    pub fn applied(&self) -> &ThemeDefinition {
        &self.applied
    }

    /// The themes the Window can apply.
    pub fn themes(&self) -> &Themes {
        &self.themes
    }

    /// Applies the definition the system's setting and the choice give, the steps of the
    /// module doc's list for what differs from what is applied. Before the system's setting
    /// is known it does nothing.
    fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(system) = self.system else {
            return;
        };
        let choice = self.chosen(cx);
        let definition = self.themes.resolve(&choice, system).clone();
        // A chosen theme is one the choice names and the themes hold; any other choice
        // is the system's.
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
            // `change` loaded the toolkit's own colours of the mode over everything: the
            // definition's go on top, in a second update.
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
