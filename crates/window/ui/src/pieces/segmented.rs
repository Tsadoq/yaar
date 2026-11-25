use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use super::{OnSelect, marked, part};
use crate::metrics::{RADIUS_CONTROL, RADIUS_INLINE, SPACE_1};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// A segment keyed `k` carries the selector `<selector>-<k>`.
#[derive(IntoElement)]
pub struct Segmented {
    id: ElementId,
    segments: Vec<(SharedString, SharedString)>,
    selected: Option<usize>,
    selector: Option<SharedString>,
    on_select: Option<OnSelect>,
}

impl Segmented {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Segmented {
            id: id.into(),
            segments: Vec::new(),
            selected: None,
            selector: None,
            on_select: None,
        }
    }

    pub fn segment(mut self, key: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        self.segments.push((key.into(), label.into()));
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
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
}

impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let parent = Arc::new(self.id.clone());
        let selector = self.selector;
        let segments = self
            .segments
            .into_iter()
            .enumerate()
            .map(|(index, (key, label))| {
                let on = self.selected == Some(index);
                let handler = self.on_select.clone();
                let segment = div()
                    .id(ElementId::NamedChild(parent.clone(), key.clone()))
                    .px(px(7.))
                    .py(SPACE_1)
                    .rounded(RADIUS_INLINE)
                    .typed(TypeRole::Body)
                    .text_color(if on { palette.text } else { palette.text_muted })
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when(on, |segment| segment.bg(palette.raised))
                    .when_some(handler, |segment, handler| {
                        segment.on_click(move |_, window, cx| handler(index, window, cx))
                    })
                    .child(label);
                marked(segment, part(&selector, &key))
            });
        let control = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(SPACE_1)
            .p(px(1.))
            .border_1()
            .border_color(palette.edge)
            .rounded(RADIUS_CONTROL)
            .children(segments);
        marked(control, selector)
    }
}
