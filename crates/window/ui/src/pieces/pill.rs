use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, Styled, Window, div, px,
};

use super::{Button, marked};
use crate::metrics::{SPACE_3, SPACE_5, shadow_float};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// The rounded bar that floats at the foot of the review pane: a line of text and, at its end,
/// the button it offers.
#[derive(IntoElement)]
pub struct Pill {
    id: ElementId,
    text: SharedString,
    actions: Vec<AnyElement>,
    selector: Option<SharedString>,
}

impl Pill {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Pill {
            id: id.into(),
            text: text.into(),
            actions: Vec::new(),
            selector: None,
        }
    }

    /// Each call adds a button after the ones before.
    pub fn action(mut self, button: Button) -> Self {
        self.actions.push(button.into_any_element());
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for Pill {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let pill = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(SPACE_5)
            .max_w(window.viewport_size().width * 0.92)
            .py(px(5.))
            .pr(SPACE_3)
            .pl(px(14.))
            .bg(palette.raised)
            .border_1()
            .border_color(palette.edge)
            .rounded_full()
            .shadow(shadow_float())
            .typed(TypeRole::Body)
            .text_color(palette.text)
            .whitespace_nowrap()
            .child(self.text)
            .children(self.actions);
        marked(pill, self.selector)
    }
}
