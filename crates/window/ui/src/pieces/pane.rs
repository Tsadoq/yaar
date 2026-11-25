use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement, Refineable as _, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, div, px,
};

use super::{marked, part};
use crate::metrics::{RADIUS_CARD, shadow_pane};
use crate::themes::{Appearance, applied, palette};

/// One of the window's columns: the pane ground inside an edge border, clipping what it holds.
/// Its place in the layout is the caller's: it takes the layout methods of `Styled`.
#[derive(IntoElement)]
pub struct Pane {
    children: Vec<AnyElement>,
    style: StyleRefinement,
    focused: bool,
    selector: Option<SharedString>,
}

impl Pane {
    pub fn new() -> Self {
        Pane {
            children: Vec::new(),
            style: StyleRefinement::default(),
            focused: false,
            selector: None,
        }
    }

    /// Highlights the pane's border and draws a top focus rule while it holds keyboard focus.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl Default for Pane {
    fn default() -> Self {
        Pane::new()
    }
}

impl ParentElement for Pane {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Pane {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Pane {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let dark = applied(cx).appearance == Appearance::Dark;
        let border = if self.focused {
            palette.focus.opacity(0.55)
        } else {
            palette.edge
        };
        let focus_bar = self.focused.then(|| {
            let bar = div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(2.))
                .bg(palette.focus.opacity(0.85));
            marked(bar, part(&self.selector, "focus"))
        });
        let mut pane = div()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .relative()
            .overflow_hidden()
            .bg(palette.pane)
            .border_1()
            .border_color(border)
            .rounded(RADIUS_CARD)
            .shadow(shadow_pane(dark))
            .children(self.children)
            .when_some(focus_bar, |pane, bar| pane.child(bar));
        pane.style().refine(&self.style);
        marked(pane, self.selector)
    }
}
