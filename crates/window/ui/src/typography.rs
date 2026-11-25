use gpui_kit::{DefiniteLength, FontWeight, Styled, px, relative};

use crate::fonts::{CODE, UI};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeRole {
    Body,
    Strong,
    Title,
    Display,
    /// The label piece uppercases its text; the role does not.
    Label,
    Meta,
    MetaStrong,
    Small,
    Tiny,
    Code,
    CodeSmall,
    Path,
    Terminal,
}

struct Spec {
    family: &'static str,
    size: f32,
    weight: FontWeight,
    line_height: DefiniteLength,
}

impl TypeRole {
    fn spec(self) -> Spec {
        let (family, size, weight, line_height) = match self {
            TypeRole::Body => (UI, 13., FontWeight::NORMAL, relative(1.45)),
            TypeRole::Strong => (UI, 13., FontWeight::MEDIUM, relative(1.3)),
            TypeRole::Title => (UI, 15., FontWeight::SEMIBOLD, relative(1.3)),
            TypeRole::Display => (UI, 17., FontWeight::SEMIBOLD, relative(1.3)),
            TypeRole::Label => (UI, 11., FontWeight::MEDIUM, relative(1.3)),
            TypeRole::Meta => (UI, 12., FontWeight::NORMAL, relative(1.3)),
            TypeRole::MetaStrong => (UI, 12., FontWeight::SEMIBOLD, relative(1.3)),
            TypeRole::Small => (UI, 11.5, FontWeight::NORMAL, relative(1.3)),
            TypeRole::Tiny => (UI, 11., FontWeight::NORMAL, relative(1.3)),
            TypeRole::Code => (CODE, 13., FontWeight::NORMAL, px(20.).into()),
            TypeRole::CodeSmall => (CODE, 12., FontWeight::NORMAL, relative(1.3)),
            TypeRole::Path => (CODE, 12.5, FontWeight::NORMAL, relative(1.3)),
            TypeRole::Terminal => (CODE, 12.5, FontWeight::NORMAL, relative(1.5)),
        };
        Spec {
            family,
            size,
            weight,
            line_height,
        }
    }
}

/// The only way a piece outside this crate sets type.
pub trait Typed: Styled + Sized {
    fn typed(self, role: TypeRole) -> Self {
        let spec = role.spec();
        self.font_family(spec.family)
            .text_size(px(spec.size))
            .line_height(spec.line_height)
            .font_weight(spec.weight)
    }
}

impl<T: Styled> Typed for T {}

#[cfg(test)]
mod tests {
    use gpui_kit::{FontWeight, Styled, TextStyleRefinement, div, px, relative};

    use super::{TypeRole, Typed};

    fn text_of(role: TypeRole) -> TextStyleRefinement {
        div().typed(role).text_style().clone()
    }

    #[test]
    fn each_type_role_sets_the_mocks_family_size_and_weight() {
        let code = text_of(TypeRole::Code);
        assert_eq!(code.font_family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(code.font_size, Some(px(13.).into()));
        assert_eq!(code.line_height, Some(px(20.).into()));

        let label = text_of(TypeRole::Label);
        assert_eq!(label.font_family.as_deref(), Some("Inter"));
        assert_eq!(label.font_size, Some(px(11.).into()));
        assert_eq!(label.font_weight, Some(FontWeight::MEDIUM));

        let body = text_of(TypeRole::Body);
        assert_eq!(body.font_family.as_deref(), Some("Inter"));
        assert_eq!(body.font_size, Some(px(13.).into()));
        assert_eq!(body.line_height, Some(relative(1.45)));
    }
}
