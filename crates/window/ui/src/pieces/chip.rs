use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::{OnClick, Tone, marked};
use crate::metrics::{SPACE_2, SPACE_4};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

#[derive(IntoElement)]
pub struct Chip {
    label: SharedString,
    tone: Tone,
    mono: bool,
    selected: bool,
    selector: Option<SharedString>,
    label_selector: Option<SharedString>,
    click: Option<(ElementId, OnClick)>,
}

impl Chip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Chip {
            label: label.into(),
            tone: Tone::Plain,
            mono: false,
            selected: false,
            selector: None,
            label_selector: None,
            click: None,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// For a branch or a path: the code font.
    pub fn mono(mut self) -> Self {
        self.mono = true;
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

    pub fn label_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.label_selector = Some(selector.into());
        self
    }

    pub fn on_click(
        mut self,
        id: impl Into<ElementId>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some((id.into(), Box::new(handler)));
        self
    }
}

impl RenderOnce for Chip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let colour = if self.selected && self.tone == Tone::Plain {
            palette.text
        } else {
            self.tone.colour(&palette)
        };
        let border = match (self.selected, self.tone) {
            (true, Tone::Plain) => palette.focus,
            (false, Tone::Plain) => palette.edge,
            _ => colour.opacity(0.35),
        };
        let role = if self.mono {
            TypeRole::CodeSmall
        } else {
            TypeRole::Small
        };
        let label = match self.label_selector {
            Some(selector) => div()
                .debug_selector(move || selector.to_string())
                .child(self.label)
                .into_any_element(),
            None => self.label.into_any_element(),
        };
        let base = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_4)
            .border_1()
            .border_color(border)
            .rounded_full()
            .when(self.selected, |chip| chip.bg(palette.raised))
            .typed(role)
            .line_height(px(19.))
            .text_color(colour)
            .whitespace_nowrap()
            .child(label);
        match self.click {
            Some((id, handler)) => {
                let clickable = base
                    .id(id)
                    .cursor_pointer()
                    .hover(move |style| style.bg(palette.raised).text_color(palette.text))
                    .on_click(move |event, window, cx| handler(event, window, cx));
                marked(clickable, self.selector).into_any_element()
            }
            None => marked(base, self.selector).into_any_element(),
        }
    }
}
