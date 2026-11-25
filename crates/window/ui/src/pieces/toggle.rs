use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::state_dot::dot;
use super::{OnClick, marked, part};
use crate::metrics::{RADIUS_CONTROL, SPACE_3, tooltip_element};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// A lamp and a label: the lamp is lit while it is on. The lamp carries the selector
/// `<selector>-dot`.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    label: SharedString,
    on: bool,
    paused: bool,
    tooltip: Option<SharedString>,
    selector: Option<SharedString>,
    on_click: Option<OnClick>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Toggle {
            id: id.into(),
            label: label.into(),
            on: false,
            paused: false,
            tooltip: None,
            selector: None,
            on_click: None,
        }
    }

    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    /// Shows the lamp in the warning colour while follow is paused.
    pub fn paused(mut self, paused: bool) -> Self {
        self.paused = paused;
        self
    }

    /// Shown in a floating tooltip when the pointer rests over the toggle.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let (text, lamp) = if self.on {
            (palette.text, palette.ok)
        } else if self.paused {
            (palette.text_muted, palette.warn)
        } else {
            (palette.text_dim, palette.text_dim)
        };
        let toggle = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(SPACE_3)
            .px(px(9.))
            .py(px(3.))
            .border_1()
            .border_color(palette.edge)
            .rounded(RADIUS_CONTROL)
            .typed(TypeRole::Body)
            .text_color(text)
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(move |style| style.bg(palette.raised).text_color(palette.text))
            .when_some(self.tooltip, |toggle, tip| {
                toggle.tooltip(move |_, cx| tooltip_element(tip.clone(), cx))
            })
            .when_some(self.on_click, |toggle, handler| {
                toggle.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .child(marked(dot(lamp, px(7.)), part(&self.selector, "dot")))
            .child(self.label);
        marked(toggle, self.selector)
    }
}
