use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::state_dot::dot;
use super::{CountBadge, OnSelect, State, marked, part};
use crate::metrics::{RADIUS_CONTROL, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

struct Tab {
    key: SharedString,
    label: SharedString,
    count: Option<usize>,
    dot: Option<State>,
}

/// A row of tabs. A tab keyed `k` carries the selector `<selector>-<k>`, its count
/// `<selector>-<k>-count`, its dot `<selector>-<k>-dot`, and its close button
/// `<selector>-<k>-close`.
#[derive(IntoElement)]
pub struct Tabs {
    id: ElementId,
    tabs: Vec<Tab>,
    selected: Option<usize>,
    mono: bool,
    trailing: Vec<AnyElement>,
    selector: Option<SharedString>,
    on_select: Option<OnSelect>,
    on_close: Option<OnSelect>,
}

impl Tabs {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Tabs {
            id: id.into(),
            tabs: Vec::new(),
            selected: None,
            mono: false,
            trailing: Vec::new(),
            selector: None,
            on_select: None,
            on_close: None,
        }
    }

    pub fn tab(
        mut self,
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        count: Option<usize>,
        dot: Option<State>,
    ) -> Self {
        self.tabs.push(Tab {
            key: key.into(),
            label: label.into(),
            count,
            dot,
        });
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// Labels in the code font, as the terminal's tabs have them.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Placed after the last tab, after those given before it.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing.push(element.into_any_element());
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    pub fn on_close(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Tabs {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let (role, across, down) = if self.mono {
            (TypeRole::CodeSmall, SPACE_4, px(3.))
        } else {
            (TypeRole::Body, SPACE_5, SPACE_2)
        };
        let parent = Arc::new(self.id.clone());
        let selector = self.selector;
        let tabs = self.tabs.into_iter().enumerate().map(|(index, tab)| {
            let on = self.selected == Some(index);
            let own = part(&selector, &tab.key);
            let handler = self.on_select.clone();
            let close = self.on_close.clone().map(|close_handler| {
                let close_button = div()
                    .id(ElementId::NamedChild(
                        parent.clone(),
                        SharedString::from(format!("{}-close", tab.key)),
                    ))
                    .px(SPACE_1)
                    .rounded(RADIUS_CONTROL)
                    .text_color(palette.text_muted)
                    .cursor_pointer()
                    .hover(move |style| style.bg(palette.edge).text_color(palette.text))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        close_handler(index, window, cx);
                    })
                    .child("\u{d7}");
                marked(close_button, part(&own, "close"))
            });
            let button = div()
                .id(ElementId::NamedChild(parent.clone(), tab.key))
                .flex()
                .items_center()
                .gap(SPACE_3)
                .px(across)
                .py(down)
                .rounded(RADIUS_CONTROL)
                .typed(role)
                .text_color(if on { palette.text } else { palette.text_muted })
                .whitespace_nowrap()
                .cursor_pointer()
                .when(on, |button| button.bg(palette.raised))
                .when_some(handler, |button, handler| {
                    button.on_click(move |_, window, cx| handler(index, window, cx))
                })
                .when_some(tab.dot, |button, state| {
                    let lamp = dot(state.colour(&palette), px(7.));
                    button.child(marked(lamp, part(&own, "dot")))
                })
                .child(tab.label)
                .when_some(tab.count, |button, count| {
                    button.child(CountBadge::new(count).selector_of(part(&own, "count")))
                })
                .children(close);
            marked(button, own)
        });
        let row = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(SPACE_1)
            .children(tabs)
            // The gap of two and these six make the eight between a row and its buttons.
            .children(
                self.trailing
                    .into_iter()
                    .map(|trailing| div().ml(SPACE_3).child(trailing)),
            );
        marked(row, selector)
    }
}
