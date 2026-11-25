use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::{OnClick, marked};
use crate::metrics::{RADIUS_CONTROL, SPACE_2, SPACE_3, SPACE_4, SPACE_5};
use crate::themes::palette;

/// One row of the sidebar, the file list or the history: raised while it is selected or under
/// the pointer. What it shows is its children, laid out in a row.
#[derive(IntoElement)]
pub struct ListRow {
    id: ElementId,
    children: Vec<AnyElement>,
    selected: bool,
    accent_selected: bool,
    nested: bool,
    rail: bool,
    compact: bool,
    selector: Option<SharedString>,
    on_click: Option<OnClick>,
}

impl ListRow {
    pub fn new(id: impl Into<ElementId>) -> Self {
        ListRow {
            id: id.into(),
            children: Vec::new(),
            selected: false,
            accent_selected: false,
            nested: false,
            rail: false,
            compact: false,
            selector: None,
            on_click: None,
        }
    }

    /// Compact centred tile for the collapsed sidebar rail.
    pub fn rail(id: impl Into<ElementId>) -> Self {
        ListRow {
            rail: true,
            ..ListRow::new(id)
        }
    }

    /// Compact single-line or dense row, for the review column's history and file lists.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Uses the theme's accent selection wash rather than `raised` when selected.
    pub fn accent_selected(mut self) -> Self {
        self.accent_selected = true;
        self
    }

    /// Indented under the row above it, as a worktree is under its repository.
    pub fn nested(mut self) -> Self {
        self.nested = true;
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl ParentElement for ListRow {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ListRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let selected_bg = if self.accent_selected {
            palette.selection
        } else {
            palette.raised
        };
        let row = div()
            .id(self.id)
            .flex()
            .w_full()
            .min_w(px(0.))
            .rounded(RADIUS_CONTROL)
            .hover(move |style| style.bg(palette.raised))
            .when(self.selected, |row| row.bg(selected_bg))
            .when(self.rail, |row| {
                row.relative().items_center().justify_center().py(SPACE_4)
            })
            .when(!self.rail && self.compact, |row| {
                row.items_center()
                    .gap(SPACE_3)
                    .py(SPACE_2)
                    .px(SPACE_3)
            })
            .when(!self.rail && !self.compact, |row| {
                let indent = if self.nested { px(22.) } else { SPACE_5 };
                row.items_start()
                    .gap(px(9.))
                    .py(px(8.))
                    .pr(SPACE_5)
                    .pl(indent)
            })
            .when_some(self.on_click, |row, handler| {
                row.cursor_pointer()
                    .on_click(move |event, window, cx| handler(event, window, cx))
            })
            .children(self.children);
        marked(row, self.selector)
    }
}
