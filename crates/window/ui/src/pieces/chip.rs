use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::{OnClick, Tone, marked};
use crate::metrics::{SPACE_2, SPACE_4, tooltip_element};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

#[derive(IntoElement)]
pub struct Chip {
    label: SharedString,
    tone: Tone,
    mono: bool,
    selected: bool,
    tooltip: Option<SharedString>,
    selector: Option<SharedString>,
    label_selector: Option<SharedString>,
    click: Option<(ElementId, OnClick)>,
    max_width: Option<Pixels>,
}

impl Chip {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Chip {
            label: label.into(),
            tone: Tone::Plain,
            mono: false,
            selected: false,
            tooltip: None,
            selector: None,
            label_selector: None,
            click: None,
            max_width: None,
        }
    }

    pub fn max_width(mut self, width: Pixels) -> Self {
        self.max_width = Some(width);
        self
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

    /// Shown in a floating tooltip when the pointer rests over the chip.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
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
        let border = if self.tone == Tone::Plain {
            palette.edge
        } else {
            colour.opacity(0.35)
        };
        let ground = if self.selected {
            Some(palette.raised)
        } else if self.tone != Tone::Plain {
            Some(colour.opacity(0.10))
        } else {
            None
        };
        let role = if self.mono {
            TypeRole::CodeSmall
        } else {
            TypeRole::Small
        };
        let truncate = self.max_width.is_some();
        let label = match self.label_selector {
            Some(selector) => div()
                .when(truncate, |el| el.min_w_0().truncate())
                .debug_selector(move || selector.to_string())
                .child(self.label)
                .into_any_element(),
            None if truncate => div()
                .min_w_0()
                .truncate()
                .child(self.label)
                .into_any_element(),
            None => self.label.into_any_element(),
        };
        let base = div()
            .flex_none()
            .when_some(self.max_width, |chip, width| chip.max_w(width))
            .flex()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_4)
            .border_1()
            .border_color(border)
            .rounded_full()
            .when_some(ground, |chip, ground| chip.bg(ground))
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
                    .on_click(move |event, window, cx| handler(event, window, cx))
                    .when_some(self.tooltip, |chip, tip| {
                        chip.tooltip(move |_, cx| tooltip_element(tip.clone(), cx))
                    });
                marked(clickable, self.selector).into_any_element()
            }
            None => match self.tooltip {
                Some(tip) => {
                    let id = self
                        .selector
                        .clone()
                        .unwrap_or_else(|| SharedString::from(format!("chip-tip:{tip}")));
                    let with_tip = base
                        .id(ElementId::Name(id))
                        .tooltip(move |_, cx| tooltip_element(tip.clone(), cx));
                    marked(with_tip, self.selector).into_any_element()
                }
                None => marked(base, self.selector).into_any_element(),
            },
        }
    }
}
