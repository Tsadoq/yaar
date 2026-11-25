use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement, Refineable as _, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, div, px,
};

use super::{marked, part};
use crate::metrics::{RADIUS_CARD, SPACE_3, SPACE_5, SPACE_6};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// A bordered box on the window ground: a thread, a Flow step, a comment box. The dashed one is
/// an outline for a closing note, with its children in a wrapping row.
#[derive(IntoElement)]
pub struct Card {
    header: Option<AnyElement>,
    children: Vec<AnyElement>,
    dashed: bool,
    selected: bool,
    style: StyleRefinement,
    selector: Option<SharedString>,
}

impl Card {
    pub fn new() -> Self {
        Card {
            header: None,
            children: Vec::new(),
            dashed: false,
            selected: false,
            style: StyleRefinement::default(),
            selector: None,
        }
    }

    /// A row above the children, under a separator. Its selector is `<selector>-header`.
    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    pub fn dashed(mut self) -> Self {
        self.dashed = true;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl Default for Card {
    fn default() -> Self {
        Card::new()
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Card {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Card {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let header = self.header.map(|header| {
            let row = div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .min_h(px(32.))
                .px(SPACE_5)
                .py(SPACE_3)
                .border_b_1()
                .border_color(palette.separator)
                .child(header);
            marked(row, part(&self.selector, "header"))
        });
        let dashed = self.dashed;
        let selected = self.selected;
        let mut card = div()
            .flex()
            .overflow_hidden()
            .border_1()
            .border_color(if selected {
                palette.focus
            } else {
                palette.edge
            })
            .rounded(RADIUS_CARD)
            .typed(TypeRole::Body)
            .text_color(palette.text)
            .when(dashed, |card| {
                card.border_dashed()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(SPACE_5)
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .text_color(palette.text_muted)
            })
            .when(!dashed, |card| {
                card.flex_col().bg(if selected {
                    palette.raised
                } else {
                    palette.window
                })
            })
            .children(header)
            .children(self.children);
        card.style().refine(&self.style);
        marked(card, self.selector)
    }
}
