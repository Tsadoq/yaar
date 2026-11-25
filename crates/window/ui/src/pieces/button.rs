use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
    transparent_black,
};

use super::{CountBadge, OnClick, marked};
use crate::metrics::{RADIUS_CONTROL, SPACE_2, SPACE_3, SPACE_4, SPACE_5};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonKind {
    Primary,
    #[default]
    Secondary,
    Ghost,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    kind: ButtonKind,
    small: bool,
    disabled: bool,
    selected: bool,
    leading: Option<SharedString>,
    count: Option<usize>,
    selector: Option<SharedString>,
    label_selector: Option<SharedString>,
    indicator_selector: Option<SharedString>,
    on_click: Option<OnClick>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Button {
            id: id.into(),
            label: label.into(),
            kind: ButtonKind::default(),
            small: false,
            disabled: false,
            selected: false,
            leading: None,
            count: None,
            selector: None,
            label_selector: None,
            indicator_selector: None,
            on_click: None,
        }
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    /// Drawn faded, with no hover ground, and it calls nothing.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Drawn on the raised ground to show the button is currently chosen.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A Unicode glyph before the label.
    pub fn leading(mut self, glyph: impl Into<SharedString>) -> Self {
        self.leading = Some(glyph.into());
        self
    }

    /// A count badge shown after the label.
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    pub fn label_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.label_selector = Some(selector.into());
        self
    }

    /// Drawn as a two-pixel bottom accent bar inside the button while `selected` is true.
    pub fn indicator_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.indicator_selector = Some(selector.into());
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

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let kind = self.kind;
        let (ground, hover, text, border) = match kind {
            ButtonKind::Primary => (
                Some(palette.accent),
                palette.accent_hover,
                palette.text_on_accent,
                transparent_black(),
            ),
            ButtonKind::Secondary => (
                self.selected.then_some(palette.raised),
                palette.raised,
                palette.text,
                if self.selected {
                    palette.focus
                } else {
                    palette.edge
                },
            ),
            ButtonKind::Ghost => (
                self.selected.then_some(palette.raised),
                palette.raised,
                if self.selected {
                    palette.text
                } else {
                    palette.text_muted
                },
                transparent_black(),
            ),
        };
        let (across, down, role) = if self.small {
            (SPACE_4, px(1.), TypeRole::Meta)
        } else {
            (SPACE_5, SPACE_2, TypeRole::Body)
        };
        let enabled = !self.disabled;
        let on_click = self.on_click;
        let label = match self.label_selector {
            Some(selector) => div()
                .debug_selector(move || selector.to_string())
                .child(self.label)
                .into_any_element(),
            None => self.label.into_any_element(),
        };
        let indicator = self.selected.then(|| {
            let bar = div()
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(px(2.))
                .bg(palette.accent);
            marked(bar, self.indicator_selector)
        });
        let button = div()
            .id(self.id)
            .relative()
            .overflow_hidden()
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .px(across)
            .py(down)
            .border_1()
            .border_color(border)
            .rounded(RADIUS_CONTROL)
            .typed(role)
            .text_color(text)
            .whitespace_nowrap()
            .when(kind == ButtonKind::Primary, |button| {
                button.font_weight(FontWeight::MEDIUM)
            })
            .when_some(ground, |button, ground| button.bg(ground))
            .when(!enabled, |button| button.opacity(0.45))
            .when(enabled, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| {
                        let style = style.bg(hover);
                        match kind {
                            ButtonKind::Ghost => style.text_color(palette.text),
                            _ => style,
                        }
                    })
                    .when_some(on_click, |button, handler| {
                        button.on_click(move |event, window, cx| handler(event, window, cx))
                    })
            })
            .children(self.leading)
            .child(label)
            .when_some(self.count, |button, count| {
                if kind == ButtonKind::Primary {
                    button.child(
                        div()
                            .flex_none()
                            .px(SPACE_2)
                            .rounded_full()
                            .bg(palette.text_on_accent.opacity(0.22))
                            .typed(TypeRole::Small)
                            .font_features(crate::metrics::tabular())
                            .text_color(palette.text_on_accent)
                            .child(count.to_string()),
                    )
                } else {
                    button.child(CountBadge::new(count))
                }
            })
            .children(indicator);
        marked(button, self.selector)
    }
}
