use gpui_kit::{
    App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px,
};

use super::marked;
use crate::metrics::{SPACE_3, tabular};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// The share of the text colour the badge is washed with.
pub(super) const WASH: f32 = 0.14;

/// A pill in a wash of the text colour, with the text colour on it.
#[derive(IntoElement)]
pub struct CountBadge {
    count: usize,
    selector: Option<SharedString>,
}

impl CountBadge {
    pub fn new(count: usize) -> Self {
        CountBadge {
            count,
            selector: None,
        }
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }

    pub(super) fn selector_of(mut self, selector: Option<SharedString>) -> Self {
        self.selector = selector;
        self
    }
}

impl RenderOnce for CountBadge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = palette(cx);
        let badge = div()
            .px(SPACE_3)
            .rounded_full()
            .bg(palette.text.opacity(WASH))
            .text_color(palette.text)
            .typed(TypeRole::Tiny)
            .line_height(px(16.))
            .font_features(tabular())
            .child(self.count.to_string());
        marked(badge, self.selector)
    }
}

#[cfg(test)]
mod tests {
    use super::WASH;
    use crate::colour::{composited, contrast};
    use crate::themes::Themes;

    #[test]
    fn the_text_colour_reads_on_the_badges_wash_over_every_ground_in_both_themes() {
        for theme in Themes::built_in().all() {
            let palette = theme.palette;
            let wash = palette.text.opacity(WASH);
            for (name, ground) in [
                ("window", palette.window),
                ("pane", palette.pane),
                ("raised", palette.raised),
            ] {
                let ratio = contrast(palette.text, composited(wash, ground), ground);
                assert!(ratio >= 4.5, "{} on {name}: {ratio:.2}", theme.name);
            }
        }
    }
}
