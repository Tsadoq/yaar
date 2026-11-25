use gpui_kit::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};

use super::marked;
use crate::metrics::{SPACE_1, SPACE_3, SPACE_5};
use crate::themes::palette;
use crate::typography::{TypeRole, Typed};

/// The small capitals over a group of rows.
#[derive(IntoElement)]
pub struct SectionLabel {
    text: SharedString,
    selector: Option<SharedString>,
}

impl SectionLabel {
    pub fn new(text: impl AsRef<str>) -> Self {
        SectionLabel {
            text: text.as_ref().to_uppercase().into(),
            selector: None,
        }
    }

    pub fn selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.selector = Some(selector.into());
        self
    }
}

impl RenderOnce for SectionLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let label = div()
            .px(SPACE_5)
            .pt(SPACE_1)
            .pb(SPACE_3)
            .typed(TypeRole::Label)
            .text_color(palette(cx).text_dim)
            .child(self.text);
        marked(label, self.selector)
    }
}

#[cfg(test)]
mod tests {
    use super::SectionLabel;

    #[test]
    fn a_section_label_keeps_its_text_in_capitals() {
        assert_eq!(SectionLabel::new("Workspaces").text, "WORKSPACES");
        assert_eq!(SectionLabel::new("Changed files").text, "CHANGED FILES");
        assert_eq!(SectionLabel::new("déjà vu").text, "DÉJÀ VU");
    }
}
