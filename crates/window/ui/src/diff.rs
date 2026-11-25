use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Div, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
    transparent_black,
};

use crate::metrics::{RADIUS_INLINE, RADIUS_SMALL, SPACE_2, SPACE_3, tabular};
use crate::themes::palette;
use crate::tokens::Palette;
use crate::typography::{TypeRole, Typed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Added,
    Removed,
}

pub fn ground(kind: DiffKind, palette: &Palette) -> Option<Hsla> {
    match kind {
        DiffKind::Context => None,
        DiffKind::Added => Some(palette.add_ground),
        DiffKind::Removed => Some(palette.del_ground),
    }
}

pub fn word(kind: DiffKind, palette: &Palette) -> Option<Hsla> {
    match kind {
        DiffKind::Context => None,
        DiffKind::Added => Some(palette.add_word),
        DiffKind::Removed => Some(palette.del_word),
    }
}

pub fn sign_colour(kind: DiffKind, palette: &Palette) -> Hsla {
    match kind {
        DiffKind::Context => palette.text_dim,
        DiffKind::Added => palette.add_text,
        DiffKind::Removed => palette.del_text,
    }
}

/// The three-pixel marker between the line numbers and the code: solid for additions, dashed for
/// removals, empty for unchanged lines.
pub fn bar(kind: DiffKind, palette: &Palette) -> Div {
    let base = div().flex_none().w(px(3.)).h_full();
    match kind {
        DiffKind::Context => base,
        DiffKind::Added => base.bg(palette.add_bar),
        DiffKind::Removed => base.border_l_2().border_dashed().border_color(palette.del_bar),
    }
}

/// The empty side of a split diff row, painted with the theme's gap tint.
pub fn gap_fill(palette: &Palette) -> Div {
    div().size_full().bg(palette.gap_stripe)
}

/// Compact `+N −M` diff stat numbers in tabular code figures.
#[derive(IntoElement)]
pub struct DiffStats {
    added: usize,
    removed: usize,
    small: bool,
}

impl DiffStats {
    pub fn new(added: usize, removed: usize) -> Self {
        DiffStats {
            added,
            removed,
            small: false,
        }
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
}

impl RenderOnce for DiffStats {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let role = if self.small {
            TypeRole::Tiny
        } else {
            TypeRole::CodeSmall
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .typed(role)
            .font_features(tabular())
            .whitespace_nowrap()
            .child(
                div()
                    .text_color(palette.add_text)
                    .child(format!("+{}", self.added)),
            )
            .child(
                div()
                    .text_color(palette.del_text)
                    .child(format!("−{}", self.removed)),
            )
    }
}

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The small `+` button in the diff gutter that starts a comment on a line.
#[derive(IntoElement)]
pub struct GutterPlus {
    id: ElementId,
    visible: bool,
    selector: Option<SharedString>,
    on_click: Option<OnClick>,
}

impl GutterPlus {
    pub fn new(id: impl Into<ElementId>) -> Self {
        GutterPlus {
            id: id.into(),
            visible: false,
            selector: None,
            on_click: None,
        }
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
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

impl RenderOnce for GutterPlus {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let button = div()
            .id(self.id)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(16.))
            .rounded(RADIUS_INLINE)
            .typed(TypeRole::MetaStrong)
            .line_height(px(16.))
            .cursor_pointer()
            .when(self.visible, |el| {
                el.bg(palette.accent).text_color(palette.text_on_accent)
            })
            .when(!self.visible, |el| {
                el.bg(transparent_black())
                    .text_color(transparent_black())
                    .hover(move |style| style.bg(palette.accent).text_color(palette.text_on_accent))
            })
            .when_some(self.on_click, |el, handler| {
                el.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .child("+");
        match self.selector {
            Some(selector) => button.debug_selector(move || selector.to_string()),
            None => button,
        }
    }
}

/// The Viewed checkbox control in a file's diff header.
#[derive(IntoElement)]
pub struct ViewedToggle {
    id: ElementId,
    viewed: bool,
    selector: Option<SharedString>,
    on_click: Option<OnClick>,
}

impl ViewedToggle {
    pub fn new(id: impl Into<ElementId>, viewed: bool) -> Self {
        ViewedToggle {
            id: id.into(),
            viewed,
            selector: None,
            on_click: None,
        }
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

impl RenderOnce for ViewedToggle {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let box_el = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(14.))
            .rounded(RADIUS_SMALL)
            .border_1()
            .border_color(if self.viewed {
                palette.ok
            } else {
                palette.edge
            })
            .when(self.viewed, |el| {
                el.bg(palette.ok)
                    .text_color(palette.text_on_accent)
                    .typed(TypeRole::Tiny)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("✓")
            });
        let toggle = div()
            .id(self.id)
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .px(SPACE_3)
            .py(SPACE_2)
            .rounded(RADIUS_INLINE)
            .typed(TypeRole::Meta)
            .text_color(if self.viewed {
                palette.ok
            } else {
                palette.text_muted
            })
            .cursor_pointer()
            .hover(move |style| style.bg(palette.raised).text_color(palette.text))
            .when_some(self.on_click, |el, handler| {
                el.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .child(box_el)
            .child("Viewed");
        match self.selector {
            Some(selector) => toggle.debug_selector(move || selector.to_string()),
            None => toggle,
        }
    }
}
