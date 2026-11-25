use gpui_kit::Hsla;

macro_rules! palette {
    ($( $token:ident, )+) => {
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct Palette {
            $( pub $token: Hsla, )+
        }

        pub const TOKEN_NAMES: [&str; palette!(@count $( $token )+)] =
            [ $( stringify!($token), )+ ];

        impl Palette {
            pub fn get(&self, name: &str) -> Option<Hsla> {
                match name {
                    $( stringify!($token) => Some(self.$token), )+
                    _ => None,
                }
            }

            /// Builds a palette from `read`, asked once for each token in order; the first error
            /// ends it.
            pub(crate) fn build<E>(
                mut read: impl FnMut(&'static str) -> Result<Hsla, E>,
            ) -> Result<Palette, E> {
                Ok(Palette { $( $token: read(stringify!($token))?, )+ })
            }
        }
    };
    (@count) => { 0 };
    (@count $head:ident $( $tail:ident )*) => { 1 + palette!(@count $( $tail )*) };
}

palette! {
    window,
    pane,
    raised,
    overlay,
    edge,
    separator,
    text,
    text_muted,
    text_dim,
    text_on_accent,
    text_on_warn,
    accent,
    accent_hover,
    accent_text,
    link,
    focus,
    selection,
    ok,
    warn,
    danger,
    info,
    add_text,
    add_ground,
    add_bar,
    add_word,
    del_text,
    del_ground,
    del_bar,
    del_word,
    fold_ground,
    gap_stripe,
    syn_keyword,
    syn_function,
    syn_string,
    syn_number,
    syn_comment,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalColors {
    pub foreground: Hsla,
    pub background: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub ansi: [Hsla; 16],
}
