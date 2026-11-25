mod button;
mod chip;
mod count_badge;
mod segmented;
mod state_dot;
mod tabs;
mod toggle;

use std::rc::Rc;

use gpui_kit::{App, ClickEvent, Hsla, InteractiveElement, SharedString, Window};

pub use button::{Button, ButtonKind};
pub use chip::Chip;
pub use count_badge::CountBadge;
pub use segmented::Segmented;
pub use state_dot::StateDot;
pub use tabs::Tabs;
pub use toggle::Toggle;

use crate::tokens::Palette;

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type OnSelect = Rc<dyn Fn(usize, &mut Window, &mut App)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Plain,
    Ok,
    Warn,
    Info,
    Danger,
}

impl Tone {
    pub(crate) fn colour(self, palette: &Palette) -> Hsla {
        match self {
            Tone::Plain => palette.text_muted,
            Tone::Ok => palette.ok,
            Tone::Warn => palette.warn,
            Tone::Info => palette.info,
            Tone::Danger => palette.danger,
        }
    }
}

/// What a workspace or a terminal tab is doing, as the sidebar dots and the terminal tabs show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    NeedsYou,
    Ok,
    Permission,
}

impl State {
    pub(crate) fn colour(self, palette: &Palette) -> Hsla {
        match self {
            State::Idle => palette.text_dim,
            State::Working => palette.link,
            State::NeedsYou => palette.warn,
            State::Ok => palette.ok,
            State::Permission => palette.danger,
        }
    }
}

/// Gives the element its selector, when the caller named one.
pub(crate) fn marked<E: InteractiveElement>(element: E, selector: Option<SharedString>) -> E {
    match selector {
        Some(selector) => element.debug_selector(move || selector.to_string()),
        None => element,
    }
}

/// The debug selector of a piece's part: `<selector>-<part>`.
pub(crate) fn part(
    selector: &Option<SharedString>,
    part: impl std::fmt::Display,
) -> Option<SharedString> {
    selector
        .as_ref()
        .map(|selector| format!("{selector}-{part}").into())
}
