use gpui_kit::{App, IntoElement, RenderOnce, SharedString, Styled, Window, div, px};

use super::marked;
use crate::themes::palette;

/// A one-pixel rule that runs along its container.
#[derive(IntoElement)]
pub struct Divider {
    vertical: bool,
    strong: bool,
    selector: Option<SharedString>,
}

impl Divider {
    pub fn horizontal() -> Self {
        Divider {
            vertical: false,
            strong: false,
            selector: None,
        }
    }

    pub fn vertical() -> Self {
        Divider {
            vertical: true,
            ..Divider::horizontal()
        }
    }

    /// In the edge colour, where the plain rule is in the fainter separator colour.
    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for Divider {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let colour = if self.strong {
            palette.edge
        } else {
            palette.separator
        };
        let rule = div().flex_none().bg(colour);
        let rule = if self.vertical {
            rule.w(px(1.)).h_full()
        } else {
            rule.h(px(1.)).w_full()
        };
        marked(rule, self.selector)
    }
}
