use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Hsla, IntoElement, ParentElement, Refineable as _, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, div, px,
};

use super::{marked, part};
use crate::colour::composited;
use crate::metrics::{RADIUS_CARD, SPACE_3, SPACE_5, SPACE_6};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// Which part of a multi-row card this element draws, so adjacent list items join cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardPart {
    #[default]
    Whole,
    Top,
    Middle,
    Bottom,
}

/// A bordered box on the window ground: a thread, a Flow step, a comment box. The dashed one is
/// an outline for a closing note, with its children in a wrapping row.
#[derive(IntoElement)]
pub struct Card {
    header: Option<AnyElement>,
    children: Vec<AnyElement>,
    dashed: bool,
    selected: bool,
    elevated: bool,
    accent: Option<Hsla>,
    part: CardPart,
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
            elevated: false,
            accent: None,
            part: CardPart::Whole,
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

    /// Lifts the card subtly above the pane ground rather than receding to the window ground.
    pub fn elevated(mut self) -> Self {
        self.elevated = true;
        self
    }

    /// Draws a two-pixel left accent bar inside the card without shifting its layout.
    pub fn accent(mut self, colour: Hsla) -> Self {
        self.accent = Some(colour);
        self
    }

    pub fn part(mut self, part: CardPart) -> Self {
        self.part = part;
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
        let accent_bar = self.accent.map(|colour| {
            let bar = div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(2.))
                .bg(colour);
            marked(bar, part(&self.selector, "accent"))
        });
        let dashed = self.dashed;
        let selected = self.selected;
        let elevated = self.elevated;
        let border_colour = if selected {
            palette.focus
        } else {
            palette.edge
        };
        let mut card = div()
            .relative()
            .flex()
            .overflow_hidden()
            .border_color(border_colour)
            .typed(TypeRole::Body)
            .text_color(palette.text)
            .map(|el| match self.part {
                CardPart::Whole => el.border_1().rounded(RADIUS_CARD),
                CardPart::Top => el
                    .border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .rounded_t(RADIUS_CARD),
                CardPart::Middle => el.border_l_1().border_r_1(),
                CardPart::Bottom => el
                    .border_b_1()
                    .border_l_1()
                    .border_r_1()
                    .rounded_b(RADIUS_CARD),
            })
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
                let ground = if selected {
                    palette.raised
                } else if elevated {
                    composited(palette.raised.opacity(0.45), palette.pane)
                } else {
                    palette.window
                };
                card.flex_col().bg(ground)
            })
            .children(header)
            .children(self.children)
            .children(accent_bar);
        card.style().refine(&self.style);
        marked(card, self.selector)
    }
}
