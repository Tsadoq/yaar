use gpui_kit::{
    App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px,
};

use super::{Tone, marked};
use crate::metrics::{SPACE_2, SPACE_4};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

#[derive(IntoElement)]
pub struct Chip {
    label: SharedString,
    tone: Tone,
    mono: bool,
    selector: Option<SharedString>,
}

impl Chip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Chip {
            label: label.into(),
            tone: Tone::Plain,
            mono: false,
            selector: None,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// For a branch or a path: the code font.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for Chip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let colour = self.tone.colour(&palette);
        let border = match self.tone {
            Tone::Plain => palette.edge,
            _ => colour.opacity(0.35),
        };
        let role = if self.mono {
            TypeRole::CodeSmall
        } else {
            TypeRole::Small
        };
        let chip = div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_4)
            .border_1()
            .border_color(border)
            .rounded_full()
            .typed(role)
            .line_height(px(19.))
            .text_color(colour)
            .whitespace_nowrap()
            .child(self.label);
        marked(chip, self.selector)
    }
}
