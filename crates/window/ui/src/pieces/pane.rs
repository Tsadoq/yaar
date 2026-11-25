use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement, Refineable as _, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, div, px,
};

use super::marked;
use crate::metrics::RADIUS_CARD;
use crate::themes::palette;

/// One of the window's columns: the pane ground inside an edge border, clipping what it holds.
/// Its place in the layout is the caller's: it takes the layout methods of `Styled`.
#[derive(IntoElement)]
pub struct Pane {
    children: Vec<AnyElement>,
    style: StyleRefinement,
    selector: Option<SharedString>,
}

impl Pane {
    pub fn new() -> Self {
        Pane {
            children: Vec::new(),
            style: StyleRefinement::default(),
            selector: None,
        }
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
        let mut pane = div()
            .flex()
            .flex_col()
            .min_w(px(0.))
            .min_h(px(0.))
            .relative()
            .overflow_hidden()
            .bg(palette.pane)
            .border_1()
            .border_color(palette.edge)
            .rounded(RADIUS_CARD)
            .children(self.children);
        pane.style().refine(&self.style);
        marked(pane, self.selector)
    }
}
