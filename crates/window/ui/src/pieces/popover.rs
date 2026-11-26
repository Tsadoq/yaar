use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, Bounds, Edges, ElementId, InteractiveElement, IntoElement,
    KeyDownEvent, MouseDownEvent, ParentElement, Pixels, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px,
};

use super::marked;
use crate::metrics::{RADIUS_CARD, SPACE_4, shadow_card};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

const WIDTH: Pixels = px(460.);

type OnPopoverDismiss = Rc<dyn Fn(&mut Window, &mut App)>;

/// A card that opens six pixels under `anchor` and floats over everything else. With no room
/// below it opens above, near a side of the window it is moved back inside, and in a window
/// narrower than itself it narrows.
#[derive(IntoElement)]
pub struct Popover {
    anchor: Bounds<Pixels>,
    children: Vec<AnyElement>,
    width: Option<Pixels>,
    min_width: Option<Pixels>,
    max_height: Option<Pixels>,
    align_right: bool,
    pass_through: bool,
    selector: Option<SharedString>,
    on_dismiss: Option<OnPopoverDismiss>,
}

impl Popover {
    pub fn new(anchor: Bounds<Pixels>) -> Self {
        Popover {
            anchor,
            children: Vec::new(),
            width: None,
            min_width: None,
            max_height: None,
            align_right: false,
            pass_through: false,
            selector: None,
            on_dismiss: None,
        }
    }

    /// Sets an explicit card width instead of the 460 px default (still clamped to the viewport).
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    /// Sets a minimum card width, allowing content to size naturally when [`Self::width`] is unset.
    pub fn min_width(mut self, width: Pixels) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Caps the card's vertical height and scrolls its contents when taller.
    pub fn max_height(mut self, height: Pixels) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Aligns the card's right edge with the anchor's right edge.
    pub fn align_right(mut self) -> Self {
        self.align_right = true;
        self
    }

    /// Lets pointer clicks pass through to the content underneath (for read-only hover cards).
    pub fn pass_through(mut self) -> Self {
        self.pass_through = true;
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    /// Called when the user clicks outside both the popover and its anchor, or presses Escape
    /// while focus is inside the popover.
    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
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
        let max_w = (window.viewport_size().width - SPACE_4 * 2.).max(px(0.));
        let scroll_id = ElementId::Name(
            self.selector
                .as_ref()
                .map(|s| SharedString::from(format!("{s}-scroll")))
                .unwrap_or_else(|| SharedString::from("popover-scroll")),
        );
        let anchor_bounds = self.anchor;
        let dismiss_out = self.on_dismiss.clone();
        let dismiss_key = self.on_dismiss;

        let mut card = div()
            .when(!self.pass_through, |el| el.occlude())
            .flex()
            .flex_col()
            .max_w(max_w)
            .overflow_hidden()
            .bg(palette.pane)
            .border_1()
            .border_color(palette.edge)
            .rounded(RADIUS_CARD)
            .shadow(shadow_card())
            .typed(TypeRole::Body)
            .text_color(palette.text);

        if let Some(width) = self.width {
            card = card.w(width);
        } else if let Some(min_width) = self.min_width {
            card = card.min_w(min_width);
        } else {
            card = card.w(WIDTH);
        }

        if let Some(dismiss) = dismiss_out {
            card = card.on_mouse_down_out(move |event: &MouseDownEvent, window, cx| {
                // Clicking the trigger itself lets the trigger's own click handler toggle it closed.
                if !anchor_bounds.contains(&event.position) {
                    dismiss(window, cx);
                }
            });
        }

        if let Some(dismiss) = dismiss_key {
            card = card.on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    cx.stop_propagation();
                    dismiss(window, cx);
                }
            });
        }

        card = match self.max_height {
            Some(max_h) => card.child(
                div()
                    .id(scroll_id)
                    .max_h(max_h)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(self.children),
            ),
            None => card.children(self.children),
        };

        let (attach, anchor_corner) = if self.align_right {
            (Anchor::BottomRight, Anchor::TopRight)
        } else {
            (Anchor::BottomLeft, Anchor::TopLeft)
        };

        deferred(
            anchored()
                .attach(self.anchor, attach)
                .anchor(anchor_corner)
                .offset(point(px(0.), px(6.)))
                .snap_to_window_with_margin(Edges::all(SPACE_4))
                .child(marked(card, self.selector)),
        )
    }
}
