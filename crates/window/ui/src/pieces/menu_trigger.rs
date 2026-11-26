use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Bounds, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, px,
};

use super::state_dot::dot;
use super::{OnClick, State, Tone, marked};
use crate::metrics::{RADIUS_CONTROL, SPACE_2, SPACE_3, tooltip_element};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// A compact trigger for a dropdown menu or a non-interactive status badge (when no click handler
/// or chevron is set).
#[derive(IntoElement)]
pub struct MenuTrigger {
    id: ElementId,
    label: SharedString,
    leading_dot: Option<State>,
    leading_glyph: Option<SharedString>,
    chevron: bool,
    open: bool,
    tone: Tone,
    mono: bool,
    max_width: Option<Pixels>,
    tooltip: Option<SharedString>,
    selector: Option<SharedString>,
    label_selector: Option<SharedString>,
    dot_selector: Option<SharedString>,
    track_bounds: Option<Rc<Cell<Bounds<Pixels>>>>,
    on_click: Option<OnClick>,
}

impl MenuTrigger {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        MenuTrigger {
            id: id.into(),
            label: label.into(),
            leading_dot: None,
            leading_glyph: None,
            chevron: true,
            open: false,
            tone: Tone::Plain,
            mono: false,
            max_width: None,
            tooltip: None,
            selector: None,
            label_selector: None,
            dot_selector: None,
            track_bounds: None,
            on_click: None,
        }
    }

    pub fn leading_dot(mut self, state: State) -> Self {
        self.leading_dot = Some(state);
        self
    }

    pub fn leading_glyph(mut self, glyph: impl Into<SharedString>) -> Self {
        self.leading_glyph = Some(glyph.into());
        self
    }

    /// Shows or hides the trailing dropdown chevron (`▾`).
    pub fn chevron(mut self, show: bool) -> Self {
        self.chevron = show;
        self
    }

    /// Drawn raised with a focus border while its popover is open.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn max_width(mut self, width: Pixels) -> Self {
        self.max_width = Some(width);
        self
    }

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

    pub fn dot_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.dot_selector = Some(selector.into());
        self
    }

    /// Records the rendered bounds of the trigger each frame so a popover can anchor to it.
    pub fn track_bounds(mut self, slot: Rc<Cell<Bounds<Pixels>>>) -> Self {
        self.track_bounds = Some(slot);
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

impl RenderOnce for MenuTrigger {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let interactive = self.on_click.is_some();
        let tone_colour = self.tone.colour(&palette);
        let text_colour = if self.tone != Tone::Plain {
            tone_colour
        } else if self.open || !interactive {
            palette.text
        } else {
            palette.text_muted
        };
        let border = if self.open {
            palette.focus
        } else if self.tone != Tone::Plain {
            tone_colour.opacity(0.35)
        } else {
            palette.edge
        };
        let ground = if self.open {
            Some(palette.raised)
        } else if self.tone != Tone::Plain {
            Some(tone_colour.opacity(0.10))
        } else {
            None
        };
        let role = if self.mono {
            TypeRole::CodeSmall
        } else {
            TypeRole::Meta
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
        let leading_dot = self.leading_dot.map(|state| {
            let lamp = dot(state.colour(&palette), px(7.));
            marked(lamp, self.dot_selector)
        });
        let chevron = self.chevron.then(|| {
            div()
                .flex_none()
                // JetBrains Mono ships U+25BE natively so the chevron needs no fallback font.
                .typed(TypeRole::CodeSmall)
                .text_color(if self.tone != Tone::Plain {
                    tone_colour
                } else {
                    palette.text_muted
                })
                .child("\u{25be}")
        });
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
        let hover_text = if self.tone != Tone::Plain {
            tone_colour
        } else {
            palette.text
        };
        let on_click = self.on_click;
        let trigger = div()
            .id(self.id)
            .relative()
            .flex_none()
            .when_some(self.max_width, |el, width| el.max_w(width))
            .flex()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_3)
            .py(px(1.))
            .border_1()
            .border_color(border)
            .rounded(RADIUS_CONTROL)
            .when_some(ground, |el, ground| el.bg(ground))
            .typed(role)
            .text_color(text_colour)
            .whitespace_nowrap()
            .when(interactive, |el| {
                el.cursor_pointer()
                    .hover(move |style| style.bg(palette.raised).text_color(hover_text))
                    .when_some(on_click, |el, handler| {
                        el.on_click(move |event, window, cx| handler(event, window, cx))
                    })
            })
            .when_some(self.tooltip, |el, tip| {
                el.tooltip(move |_, cx| tooltip_element(tip.clone(), cx))
            })
            .children(leading_dot)
            .children(self.leading_glyph)
            .child(label)
            .children(chevron);
        match measure {
            Some(measure) => div()
                .relative()
                .flex_none()
                .child(marked(trigger, self.selector))
                .child(measure)
                .into_any_element(),
            None => marked(trigger, self.selector).into_any_element(),
        }
    }
}
