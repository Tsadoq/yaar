use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
    div,
};

use super::{Button, Tone, marked};
use crate::colour::composited;
use crate::metrics::{RADIUS_CARD, SPACE_3, SPACE_4, SPACE_6};
use crate::themes::palette;
use crate::tokens::Palette;
use crate::typography::{TypeRole, Typed};

/// The share of its colour a tinted banner is washed with.
pub(super) const WASH: f32 = 0.08;

/// The faintest wash, for a banner that has no tone.
pub(super) const PLAIN_WASH: f32 = 0.03;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Look {
    Tone(Tone),
    Live,
}

impl Look {
    /// The text colour, and the opaque ground it lies on: the wash laid over the pane, so the
    /// ground is the same wherever the banner is put.
    pub(super) fn colours(self, palette: &Palette) -> (Hsla, Hsla) {
        let plain = palette.text.opacity(PLAIN_WASH);
        let (text, wash) = match self {
            Look::Tone(Tone::Plain) => (palette.text_muted, plain),
            Look::Tone(tone) => {
                let colour = tone.colour(palette);
                (colour, colour.opacity(WASH))
            }
            Look::Live => (palette.link, plain),
        };
        (text, composited(wash, palette.pane))
    }
}

/// A line of notice across the top of a pane, with buttons at its end.
#[derive(IntoElement)]
pub struct Banner {
    text: Option<SharedString>,
    children: Vec<AnyElement>,
    look: Look,
    card: bool,
    actions: Vec<AnyElement>,
    selector: Option<SharedString>,
}

impl Banner {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Banner {
            text: Some(text.into()),
            children: Vec::new(),
            look: Look::Tone(Tone::Plain),
            card: false,
            actions: Vec::new(),
            selector: None,
        }
    }

    /// A banner whose content is supplied as child elements.
    pub fn custom() -> Self {
        Banner {
            text: None,
            children: Vec::new(),
            look: Look::Tone(Tone::Plain),
            card: false,
            actions: Vec::new(),
            selector: None,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.look = Look::Tone(tone);
        self
    }

    /// For what is happening now: the text in the link colour, on the plain wash.
    pub fn live(mut self) -> Self {
        self.look = Look::Live;
        self
    }

    /// Framed as a rounded card for notices that sit on the window ground above the panes.
    pub fn card(mut self) -> Self {
        self.card = true;
        self
    }

    /// Each call adds a button after the ones before.
    pub fn action(mut self, button: Button) -> Self {
        self.actions.push(button.into_any_element());
        self
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl ParentElement for Banner {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Banner {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let (text, ground) = self.look.colours(&palette);
        let banner = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .px(SPACE_6)
            .py(SPACE_3)
            .when(self.card, |banner| {
                banner
                    .border_1()
                    .border_color(palette.edge)
                    .rounded(RADIUS_CARD)
            })
            .when(!self.card, |banner| {
                banner.border_b_1().border_color(palette.separator)
            })
            .bg(ground)
            .typed(TypeRole::Meta)
            .text_color(text)
            .when_some(self.text, |banner, words| {
                banner.child(div().flex_1().min_w_0().child(words))
            })
            .children(self.children)
            .children(self.actions);
        marked(banner, self.selector)
    }
}

#[cfg(test)]
mod tests {
    use super::{Look, Tone};
    use crate::colour::contrast;
    use crate::themes::Themes;

    #[test]
    fn the_text_of_a_banner_and_of_its_buttons_reads_on_its_ground_in_both_themes() {
        let looks = [
            Look::Tone(Tone::Plain),
            Look::Tone(Tone::Ok),
            Look::Tone(Tone::Warn),
            Look::Tone(Tone::Info),
            Look::Tone(Tone::Danger),
            Look::Live,
        ];
        let mut short = Vec::new();
        for theme in Themes::built_in().all() {
            let palette = theme.palette;
            // The ground is opaque, so it is the same wherever the banner is put.
            let buttons = [("ghost", palette.text_muted), ("secondary", palette.text)];
            for look in looks {
                let (text, ground) = look.colours(&palette);
                assert_eq!(ground.a, 1., "{} {look:?}", theme.name);
                for (name, colour) in [("text", text)].into_iter().chain(buttons) {
                    let ratio = contrast(colour, ground, ground);
                    if ratio < 4.5 {
                        short.push(format!("{} {look:?} {name}: {ratio:.2}", theme.name));
                    }
                }
            }
        }
        assert!(short.is_empty(), "{short:#?}");
    }
}
