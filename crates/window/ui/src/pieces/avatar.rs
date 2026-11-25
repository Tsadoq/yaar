use gpui_kit::{
    App, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
    div, px,
};

use super::marked;
use crate::themes::palette;
use crate::tokens::Palette;
use crate::typography::{TypeRole, Typed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    You,
    Agent,
}

impl Who {
    /// The letter, its colour and the colour of the disc.
    fn look(self, palette: &Palette) -> (&'static str, Hsla, Hsla) {
        match self {
            Who::You => ("Y", palette.text_on_accent, palette.accent),
            Who::Agent => ("A", palette.warn, palette.raised),
        }
    }
}

/// The round mark beside a comment: who wrote it.
#[derive(IntoElement)]
pub struct Avatar {
    who: Who,
    selector: Option<SharedString>,
}

impl Avatar {
    pub fn you() -> Self {
        Avatar {
            who: Who::You,
            selector: None,
        }
    }

    pub fn agent() -> Self {
        Avatar {
            who: Who::Agent,
            selector: None,
        }
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (letter, text, ground) = self.who.look(&palette(cx));
        let avatar = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(22.))
            .rounded_full()
            .bg(ground)
            .typed(TypeRole::Label)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(text)
            .child(letter);
        marked(avatar, self.selector)
    }
}
