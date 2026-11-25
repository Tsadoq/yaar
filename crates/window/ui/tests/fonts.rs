use std::borrow::Cow;
use std::sync::Arc;

use gpui_kit::{
    FontWeight, PlatformTextSystem, TestAppContext, TestDispatcher, TextRun, black, font, px,
};
use yaar_ui::fonts::{self, BUILT_IN, CODE, UI};

/// The text system the platform draws with. GPUI's test platform replaces it by one that lists no
/// fonts and takes any bytes. On Linux no system fonts are loaded, so Inter is named as the font
/// the system falls back to.
fn platform_text_system() -> Arc<dyn PlatformTextSystem> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(gpui_apple::AppleTextSystem::new())
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(UI))
    }
}

fn app_on_the_platforms_text_system() -> TestAppContext {
    TestAppContext::build_with_text_system(TestDispatcher::new(0), None, platform_text_system())
}

fn at_weight(family: &str, weight: FontWeight) -> gpui_kit::Font {
    let mut font = font(family.to_owned());
    font.weight = weight;
    font
}

#[test]
fn the_two_families_are_known_to_the_text_system_after_init() {
    let cx = app_on_the_platforms_text_system();
    cx.update(|cx| {
        yaar_ui::init(cx);
        let names = cx.text_system().all_font_names();
        assert!(names.iter().any(|name| name == UI), "{names:?}");
        assert!(names.iter().any(|name| name == CODE), "{names:?}");

        // `get_font_for_id` names only a font that resolved as itself, never one that stands in
        // for another.
        let text = cx.text_system();
        let resolve = |family: &str, weight| {
            let id = text.resolve_font(&at_weight(family, weight));
            let found = text.get_font_for_id(id).map(|font| font.family);
            assert_eq!(found.as_deref(), Some(family), "{family} at {weight:?}");
            id
        };
        let regular = resolve(UI, FontWeight::NORMAL);
        let medium = resolve(UI, FontWeight::MEDIUM);
        let semibold = resolve(UI, FontWeight::SEMIBOLD);
        assert_ne!(medium, regular);
        assert_ne!(semibold, regular);
        assert_ne!(semibold, medium);
        let mono_regular = resolve(CODE, FontWeight::NORMAL);
        let mono_medium = resolve(CODE, FontWeight::MEDIUM);
        assert_ne!(mono_medium, mono_regular);
    });
    assert_eq!(BUILT_IN.len(), 5);
}

#[test]
fn bytes_that_are_no_font_are_refused_and_the_text_system_still_lays_out_text() {
    let mut cx = app_on_the_platforms_text_system();
    cx.update(|cx| {
        yaar_ui::init(cx);
        let refused = fonts::register(cx, vec![Cow::Borrowed(b"not a font")]);
        assert!(refused.is_err());

        // A file signature followed by garbage passes `register`'s own check, so what happens
        // next is the platform's.
        let corrupt = fonts::register(cx, vec![Cow::Borrowed(b"\0\x01\0\0 garbage, no tables")]);
        #[cfg(target_os = "macos")]
        assert!(corrupt.is_err());
        // The font database behind it skips a face it cannot read and reports success.
        #[cfg(target_os = "linux")]
        assert!(corrupt.is_ok());
    });
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        let width = window
            .text_system()
            .layout_line(
                "abc",
                px(13.),
                &[TextRun {
                    len: 3,
                    font: window.text_style().font(),
                    color: black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width;
        assert!(width > px(0.));
    });
}
