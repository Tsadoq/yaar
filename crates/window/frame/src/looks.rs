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

use gpui_kit::*;
use yaar_link::ThemeChoice;

use crate::settings::SettingList;
use crate::theme::{Appearance, ThemeDefinition, Themes};

/// The theme the Window draws in, and why.
pub struct Looks {
    settings: Entity<SettingList>,
    /// The system's setting, as last told; none until told.
    system: Option<Appearance>,
}

impl Looks {
    /// The model of one window, reading the user's choice from `settings`. It applies
    /// nothing until it is first told the system's setting.
    pub fn new(settings: Entity<SettingList>, cx: &mut Context<Self>) -> Looks {
        let _ = cx;
        Looks {
            settings,
            system: None,
        }
    }

    /// The system's light or dark setting is `appearance` now: applies what the module
    /// doc's table says.
    pub fn system_is(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        let _ = (appearance, cx);
        todo!("note the system's setting and apply, as the module doc says")
    }

    /// The system's setting, as last told; none until [`Looks::system_is`] was first called.
    pub fn system(&self) -> Option<Appearance> {
        self.system
    }

    /// The user's choice, as the settings hold it: [`ThemeChoice::System`] where none is
    /// set.
    pub fn chosen(&self, cx: &App) -> ThemeChoice {
        let _ = (&self.settings, cx);
        todo!("the choice the settings hold")
    }

    /// The definition applied now.
    pub fn applied(&self) -> &ThemeDefinition {
        todo!("the definition applied now")
    }

    /// The themes the Window can apply.
    pub fn themes(&self) -> &Themes {
        todo!("the themes")
    }
}
