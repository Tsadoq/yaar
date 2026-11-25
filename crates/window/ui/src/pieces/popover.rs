use gpui_kit::{
    Anchor, AnyElement, App, Bounds, Edges, InteractiveElement, IntoElement, ParentElement, Pixels,
    RenderOnce, SharedString, Styled, Window, anchored, deferred, div, point, px,
};

use super::marked;
use crate::metrics::{RADIUS_CARD, SPACE_4, shadow_card};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

const WIDTH: Pixels = px(460.);

/// A card that opens six pixels under `anchor` and floats over everything else. With no room
/// below it opens above, near a side of the window it is moved back inside, and in a window
/// narrower than itself it narrows.
#[derive(IntoElement)]
pub struct Popover {
    anchor: Bounds<Pixels>,
    children: Vec<AnyElement>,
    selector: Option<SharedString>,
}

impl Popover {
    pub fn new(anchor: Bounds<Pixels>) -> Self {
        Popover {
            anchor,
            children: Vec::new(),
            selector: None,
        }
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let card = div()
            .occlude()
            .flex()
            .flex_col()
            .w(WIDTH)
            .max_w(window.viewport_size().width - SPACE_4 * 2.)
            .overflow_hidden()
            .bg(palette.pane)
            .border_1()
            .border_color(palette.edge)
            .rounded(RADIUS_CARD)
            .shadow(shadow_card())
            .typed(TypeRole::Body)
            .text_color(palette.text)
            .children(self.children);
        deferred(
            anchored()
                .attach(self.anchor, Anchor::BottomLeft)
                .anchor(Anchor::TopLeft)
                .offset(point(px(0.), px(6.)))
                .snap_to_window_with_margin(Edges::all(SPACE_4))
                .child(marked(card, self.selector)),
        )
    }
}
