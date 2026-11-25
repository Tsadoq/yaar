use std::f32::consts::TAU;

use gpui_kit::{
    Animation, AnimationExt as _, App, Div, ElementId, Hsla, IntoElement, Pixels, RenderOnce,
    SharedString, Styled, Window, div, px,
};

use super::{State, marked};
use crate::metrics::PULSE_SIDEBAR;
use crate::themes::palette;

/// A filled circle. The state dot, the tab dot and the toggle's lamp are drawn by this.
pub(super) fn dot(colour: Hsla, size: Pixels) -> Div {
    div().flex_none().size(size).rounded_full().bg(colour)
}

#[derive(IntoElement)]
pub struct StateDot {
    state: State,
    large: bool,
    pulse: Option<ElementId>,
    selector: Option<SharedString>,
}

impl StateDot {
    pub fn new(state: State) -> Self {
        StateDot {
            state,
            large: false,
            pulse: None,
            selector: None,
        }
    }

    /// The size the collapsed rail draws.
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// Pulses while `State::Working`, stopping automatically under reduced motion.
    pub fn pulsing(mut self, id: impl Into<ElementId>) -> Self {
        self.pulse = Some(id.into());
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
        let base = marked(dot(self.state.colour(&palette(cx)), size), self.selector);
        if self.state == State::Working
            && let Some(id) = self.pulse
        {
            base.with_animation(id, Animation::new(PULSE_SIDEBAR).repeat(), |el, delta| {
                let wave = 0.5 + 0.5 * (delta * TAU).cos();
                el.opacity(0.35 + 0.65 * wave)
            })
            .into_any_element()
        } else {
            base.into_any_element()
        }
    }
}
