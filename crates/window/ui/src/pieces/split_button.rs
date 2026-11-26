use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Bounds, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, px,
};

use super::{OnClick, marked};
use crate::metrics::{RADIUS_CONTROL, SPACE_2, SPACE_3, tooltip_element};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// A compact two-part control: a primary action button on the left and a menu chevron on the right,
/// joined inside one bordered frame.
#[derive(IntoElement)]
pub struct SplitButton {
    id: ElementId,
    primary_label: SharedString,
    open: bool,
    primary_tooltip: Option<SharedString>,
    menu_tooltip: Option<SharedString>,
    selector: Option<SharedString>,
    primary_selector: Option<SharedString>,
    menu_selector: Option<SharedString>,
    track_bounds: Option<Rc<Cell<Bounds<Pixels>>>>,
    on_primary: Option<OnClick>,
    on_menu: Option<OnClick>,
}

impl SplitButton {
    pub fn new(id: impl Into<ElementId>, primary_label: impl Into<SharedString>) -> Self {
        SplitButton {
            id: id.into(),
            primary_label: primary_label.into(),
            open: false,
            primary_tooltip: None,
            menu_tooltip: None,
            selector: None,
            primary_selector: None,
            menu_selector: None,
            track_bounds: None,
            on_primary: None,
            on_menu: None,
        }
    }

    /// Highlights the menu half while its popover is open.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn primary_tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.primary_tooltip = Some(text.into());
        self
    }

    pub fn menu_tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.menu_tooltip = Some(text.into());
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    pub fn primary_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.primary_selector = Some(selector.into());
        self
    }

    pub fn menu_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.menu_selector = Some(selector.into());
        self
    }

    /// Records the rendered bounds of the control each frame so a popover can anchor to it.
    pub fn track_bounds(mut self, slot: Rc<Cell<Bounds<Pixels>>>) -> Self {
        self.track_bounds = Some(slot);
        self
    }

    pub fn on_primary(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_primary = Some(Box::new(handler));
        self
    }

    pub fn on_menu(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_menu = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SplitButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let parent = std::sync::Arc::new(self.id);
        let on_primary = self.on_primary;
        let on_menu = self.on_menu;

        let primary = div()
            .id(ElementId::NamedChild(
                parent.clone(),
                SharedString::from("primary"),
            ))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .px(SPACE_3)
            .py(px(1.))
            .typed(TypeRole::Meta)
            .text_color(palette.text)
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(move |style| style.bg(palette.raised))
            .when_some(on_primary, |el, handler| {
                el.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when_some(self.primary_tooltip, |el, tip| {
                el.tooltip(move |_, cx| tooltip_element(tip.clone(), cx))
            })
            .child(self.primary_label);

        let menu = div()
            .id(ElementId::NamedChild(parent, SharedString::from("menu")))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .px(SPACE_2)
            .py(px(1.))
            // JetBrains Mono ships U+25BE natively so the chevron needs no fallback font.
            .typed(TypeRole::CodeSmall)
            .text_color(if self.open {
                palette.text
            } else {
                palette.text_muted
            })
            .whitespace_nowrap()
            .cursor_pointer()
            .when(self.open, |el| el.bg(palette.raised))
            .hover(move |style| style.bg(palette.raised).text_color(palette.text))
            .when_some(on_menu, |el, handler| {
                el.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when_some(self.menu_tooltip, |el, tip| {
                el.tooltip(move |_, cx| tooltip_element(tip.clone(), cx))
            })
            .child("\u{25be}");

        let measure = self.track_bounds.map(|slot| {
            canvas(
                |bounds, _, _| bounds,
                move |_, bounds, _, _| {
                    if bounds.size.width > Pixels::ZERO && bounds.size.height > Pixels::ZERO {
                        slot.set(bounds);
                    }
                },
            )
            .absolute()
            .inset_0()
        });

        let border = if self.open {
            palette.focus
        } else {
            palette.edge
        };

        let container = div()
            .relative()
            .flex_none()
            .flex()
            .items_stretch()
            .overflow_hidden()
            .border_1()
            .border_color(border)
            .rounded(RADIUS_CONTROL)
            .child(marked(primary, self.primary_selector))
            .child(div().flex_none().w(px(1.)).bg(palette.separator))
            .child(marked(menu, self.menu_selector));

        match measure {
            Some(measure) => div()
                .relative()
                .flex_none()
                .child(marked(container, self.selector))
                .child(measure)
                .into_any_element(),
            None => marked(container, self.selector).into_any_element(),
        }
    }
}
