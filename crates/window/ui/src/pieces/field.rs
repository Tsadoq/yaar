use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::{
    AnyElement, App, Entity, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
    div, px,
};

use super::{marked, part};
use crate::metrics::{RADIUS_CARD, SPACE_3, SPACE_4};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

enum Kind {
    Input(Entity<InputState>),
    Textarea(Entity<TextareaState>),
}

/// The toolkit's text input in a box of its own, as the comment box has it. The input is
/// `<selector>-input` and the row beneath it, when there is one, `<selector>-footer`.
#[derive(IntoElement)]
pub struct Field {
    kind: Kind,
    footer: Option<AnyElement>,
    selector: Option<SharedString>,
}

impl Field {
    pub fn input(state: &Entity<InputState>) -> Self {
        Field::of(Kind::Input(state.clone()))
    }

    pub fn textarea(state: &Entity<TextareaState>) -> Self {
        Field::of(Kind::Textarea(state.clone()))
    }

    fn of(kind: Kind) -> Self {
        Field {
            kind,
            footer: None,
            selector: None,
        }
    }

    /// A row of hints and buttons under the input.
    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for Field {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let (input, height) = match self.kind {
            Kind::Input(state) => (
                Input::new(&state).typed(TypeRole::Body).into_any_element(),
                px(0.),
            ),
            Kind::Textarea(state) => (
                Textarea::new(&state)
                    .typed(TypeRole::Body)
                    .into_any_element(),
                px(62.),
            ),
        };
        let footer = self.footer.map(|footer| {
            marked(
                div().flex().items_center().gap(SPACE_4).child(footer),
                part(&self.selector, "footer"),
            )
        });
        let field = div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .w_full()
            .max_w(px(760.))
            .p(SPACE_4)
            .bg(palette.window)
            .border_1()
            .border_color(palette.edge)
            .rounded(RADIUS_CARD)
            .child(marked(
                div().w_full().min_h(height).child(input),
                part(&self.selector, "input"),
            ))
            .children(footer);
        marked(field, self.selector)
    }
}
