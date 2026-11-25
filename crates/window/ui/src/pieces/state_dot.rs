use gpui_kit::{
    App, Div, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Styled, Window, div, px,
};

use super::{State, marked};
use crate::themes::palette;

/// A filled circle. The state dot, the tab dot and the toggle's lamp are drawn by this.
pub(super) fn dot(colour: Hsla, size: Pixels) -> Div {
    div().flex_none().size(size).rounded_full().bg(colour)
}

/// Static: it does not pulse.
#[derive(IntoElement)]
pub struct StateDot {
    state: State,
    large: bool,
    selector: Option<SharedString>,
}

impl StateDot {
    pub fn new(state: State) -> Self {
        StateDot {
            state,
            large: false,
            selector: None,
        }
    }

    /// The size the collapsed rail draws.
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for StateDot {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = if self.large { px(10.) } else { px(8.) };
        marked(dot(self.state.colour(&palette(cx)), size), self.selector)
    }
}
