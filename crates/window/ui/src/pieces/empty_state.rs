use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px,
};

use super::{Button, marked};
use crate::metrics::{SPACE_3, SPACE_5};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// What a pane shows when it has nothing to show: a title, a few lines and perhaps a button,
/// centred.
#[derive(IntoElement)]
pub struct EmptyState {
    title: SharedString,
    display: bool,
    lines: Vec<SharedString>,
    actions: Vec<AnyElement>,
    selector: Option<SharedString>,
}

impl EmptyState {
    pub fn new(title: impl Into<SharedString>) -> Self {
        EmptyState {
            title: title.into(),
            display: false,
            lines: Vec::new(),
            actions: Vec::new(),
            selector: None,
        }
    }

    /// Larger display heading, for a whole-window empty state.
    pub fn display(mut self) -> Self {
        self.display = true;
        self
    }

    pub fn line(mut self, text: impl Into<SharedString>) -> Self {
        self.lines.push(text.into());
        self
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

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let lines = self.lines.into_iter().map(|line| {
            div()
                .w_full()
                .min_w_0()
                .max_w(px(360.))
                .typed(TypeRole::Body)
                .text_color(palette.text_muted)
                .child(line)
        });
        let state = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(SPACE_5)
            .px(px(24.))
            .py(px(40.))
            .text_center()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .typed(if self.display {
                        TypeRole::Display
                    } else {
                        TypeRole::Strong
                    })
                    .text_color(palette.text)
                    .child(self.title),
            )
            .children(lines)
            .when(!self.actions.is_empty(), |state| {
                state.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_center()
                        .gap(SPACE_3)
                        .children(self.actions),
                )
            });
        marked(state, self.selector)
    }
}
