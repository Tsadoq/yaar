use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, Styled, Window, div, px,
};

use super::{Button, ButtonKind, OnDismiss, marked, part};
use crate::metrics::{RADIUS_CONTROL, SPACE_4, SPACE_6, shadow_float};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// A short message that floats over the window. It has no timer: it stays until its owner stops
/// drawing it, which the dismiss button asks for through `on_dismiss`. That button's selector
/// is `<selector>-dismiss`.
#[derive(IntoElement)]
pub struct Toast {
    id: ElementId,
    text: SharedString,
    actions: Vec<AnyElement>,
    selector: Option<SharedString>,
    on_dismiss: Option<OnDismiss>,
}

impl Toast {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Toast {
            id: id.into(),
            text: text.into(),
            actions: Vec::new(),
            selector: None,
            on_dismiss: None,
        }
    }

    /// Each call adds a button after the ones before.
    pub fn action(mut self, button: Button) -> Self {
        self.actions.push(button.into_any_element());
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for Toast {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let cap = (window.viewport_size().width - px(40.)).clamp(px(160.), px(380.));
        let on_dismiss = self.on_dismiss;
        let dismiss = Button::new(
            ElementId::NamedChild(Arc::new(self.id.clone()), "dismiss".into()),
            "\u{d7}",
        )
        .kind(ButtonKind::Ghost)
        .small()
        .when_some(part(&self.selector, "dismiss"), |button, selector| {
            button.selector(selector)
        })
        .when_some(on_dismiss, |button, handler| {
            button.on_click(move |_, window, cx| handler(window, cx))
        });
        let toast = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(SPACE_4)
            .max_w(cap)
            .px(SPACE_6)
            .py(SPACE_4)
            .bg(palette.raised)
            .border_1()
            .border_color(palette.edge)
            .rounded(RADIUS_CONTROL)
            .shadow(shadow_float())
            .typed(TypeRole::Body)
            .text_color(palette.text)
            .child(div().flex_1().min_w(px(0.)).child(self.text))
            .children(self.actions)
            .child(dismiss);
        marked(toast, self.selector)
    }
}
