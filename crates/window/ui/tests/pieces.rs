//! Each piece drawn alone in a test window, in the light and the dark theme. GPUI's test platform
//! draws no glyphs, so what is read is quads, bounds and callbacks.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{
    AnyElement, AppContext, Bounds, Context, Hsla, IntoElement, Modifiers, ParentElement, Pixels,
    Render, Styled, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, div,
    point, px, size,
};
use yaar_ui::pieces::{
    Button, ButtonKind, Chip, CountBadge, Segmented, State, StateDot, Tabs, Toggle, Tone,
};
use yaar_ui::themes::{Appearance, DARK, LIGHT, Themes, palette, set_applied};
use yaar_ui::tokens::Palette;

const THEMES: [&str; 2] = [LIGHT, DARK];

type Piece = Box<dyn Fn() -> AnyElement>;

/// Which token of the palette a colour is.
type Token = fn(&Palette) -> Hsla;

struct Stage {
    piece: Piece,
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(palette(cx).window)
            .p(px(20.))
            .flex()
            .items_start()
            .child((self.piece)())
    }
}

/// One piece in a window under one of the built-in themes.
struct Shown {
    cx: VisualTestContext,
}

impl Shown {
    fn open(
        theme: &str,
        cx: &mut TestAppContext,
        piece: impl Fn() -> AnyElement + 'static,
    ) -> Self {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let definition = Themes::built_in()
                .named(theme)
                .expect("a built-in theme")
                .clone();
            let mode = match definition.appearance {
                Appearance::Light => ThemeMode::Light,
                Appearance::Dark => ThemeMode::Dark,
            };
            Theme::change(mode, None, cx);
            Theme::update(cx, |kit| definition.apply_to_kit(kit));
            set_applied(definition.clone(), cx);
            // The pieces read the applied theme, so a test under the wrong one would pass for the
            // wrong reason.
            assert_eq!(palette(cx), definition.palette, "{theme} is applied");
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(600.), px(200.)),
            })),
            ..Default::default()
        };
        let piece: Piece = Box::new(piece);
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, move |_, cx| cx.new(|_| Stage { piece }))
            })
            .expect("open a test window");
        let cx = VisualTestContext::from_window(window, cx);
        cx.run_until_parked();
        Shown { cx }
    }

    fn palette(&mut self) -> Palette {
        self.cx.update(|_, app| palette(app))
    }

    fn is_drawn(&mut self, selector: &str) -> bool {
        // The test context takes a text that lives as long as the program.
        let selector = Box::leak(selector.to_owned().into_boxed_str());
        self.cx.debug_bounds(selector).is_some()
    }

    fn bounds(&mut self, selector: &str) -> Bounds<Pixels> {
        let leaked: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.cx
            .debug_bounds(leaked)
            .unwrap_or_else(|| panic!("{selector} is not drawn"))
    }

    fn click(&mut self, selector: &str) {
        let at = self.bounds(selector).center();
        self.cx.simulate_click(at, Modifiers::none());
    }

    fn hover(&mut self, selector: &str) {
        let at = self.bounds(selector).center();
        self.cx.simulate_mouse_move(at, None, Modifiers::none());
        self.cx.run_until_parked();
    }

    /// The quads painted at the element's bounds in the last frame. The scene counts in device
    /// pixels, so the bounds are scaled by the window's factor first.
    fn quads_at(&mut self, selector: &str) -> Vec<gpui_kit::Quad> {
        let bounds = self.bounds(selector);
        let (scale, quads) = self
            .cx
            .update(|window, _| (window.scale_factor(), window.painted_quads()));
        let at = bounds.scale(scale);
        quads.into_iter().filter(|quad| quad.bounds == at).collect()
    }

    /// The solid colour the element's own quad is filled with, or `None` when it is not filled.
    fn ground(&mut self, selector: &str) -> Option<Hsla> {
        let quads = self.quads_at(selector);
        let grounds: Vec<_> = quads
            .iter()
            .filter_map(|quad| quad.background.as_solid())
            .filter(|ground| ground.a > 0.)
            .collect();
        assert!(
            grounds.len() <= 1,
            "{selector}: {} filled quads at its bounds",
            grounds.len()
        );
        grounds.first().copied()
    }

    /// The border colour the element is drawn with. GPUI paints a bordered, unfilled quad in
    /// clipped strips, which all carry it.
    fn border(&mut self, selector: &str) -> Hsla {
        let quads = self.quads_at(selector);
        let first = quads
            .first()
            .unwrap_or_else(|| panic!("{selector}: no quad at its bounds"))
            .border_color;
        assert!(
            quads.iter().all(|quad| quad.border_color == first),
            "{selector}: its quads differ in border colour"
        );
        first
    }
}

fn in_both_themes(
    cx: &mut TestAppContext,
    piece: impl Fn() -> AnyElement + Clone + 'static,
    test: impl Fn(&mut Shown),
) {
    for theme in THEMES {
        let mut shown = Shown::open(theme, cx, piece.clone());
        test(&mut shown);
    }
}

fn inside(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    outer.left() <= inner.left()
        && outer.top() <= inner.top()
        && outer.right() >= inner.right()
        && outer.bottom() >= inner.bottom()
}

fn counter() -> Rc<Cell<usize>> {
    Rc::new(Cell::new(0))
}

fn faded(colour: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..colour }
}

#[gpui_kit::test]
fn a_primary_button_is_painted_in_the_accent_and_a_secondary_one_on_no_ground(
    cx: &mut TestAppContext,
) {
    let kinds = [
        ("primary", ButtonKind::Primary),
        ("secondary", ButtonKind::Secondary),
        ("ghost", ButtonKind::Ghost),
    ];
    for theme in THEMES {
        for (name, kind) in kinds {
            let mut shown = Shown::open(theme, cx, move || {
                Button::new("go", "Go")
                    .kind(kind)
                    .selector("go")
                    .into_any_element()
            });
            let accent = shown.palette().accent;
            let wanted = (kind == ButtonKind::Primary).then_some(accent);
            assert_eq!(shown.ground("go"), wanted, "{theme} {name}");
        }
        let mut shown = Shown::open(theme, cx, || {
            Button::new("go", "Go").selector("go").into_any_element()
        });
        assert_eq!(
            shown.ground("go"),
            None,
            "{theme}: secondary is the default kind"
        );
    }
}

#[gpui_kit::test]
fn a_secondary_button_has_the_edge_for_its_border_and_a_primary_one_has_none(
    cx: &mut TestAppContext,
) {
    for theme in THEMES {
        let mut shown = Shown::open(theme, cx, || {
            Button::new("go", "Go").selector("go").into_any_element()
        });
        let edge = shown.palette().edge;
        assert_eq!(shown.border("go"), edge, "{theme}");
        let mut shown = Shown::open(theme, cx, || {
            Button::new("go", "Go")
                .kind(ButtonKind::Primary)
                .selector("go")
                .into_any_element()
        });
        assert_eq!(shown.border("go").a, 0., "{theme}");
    }
}

#[gpui_kit::test]
fn hovering_an_enabled_button_changes_its_ground_and_a_disabled_one_stays(cx: &mut TestAppContext) {
    for theme in THEMES {
        let kinds: [(ButtonKind, bool, Option<Token>); 5] = [
            (ButtonKind::Primary, false, Some(|p| p.accent_hover)),
            (ButtonKind::Secondary, false, Some(|p| p.raised)),
            (ButtonKind::Ghost, false, Some(|p| p.raised)),
            (ButtonKind::Primary, true, Some(|p| p.accent)),
            (ButtonKind::Secondary, true, None),
        ];
        for (kind, disabled, wanted) in kinds {
            let mut shown = Shown::open(theme, cx, move || {
                Button::new("go", "Go")
                    .kind(kind)
                    .disabled(disabled)
                    .selector("go")
                    .into_any_element()
            });
            shown.hover("go");
            let palette = shown.palette();
            let ground = shown.ground("go");
            let wanted = wanted.map(|colour| {
                let colour = colour(&palette);
                // A disabled button is drawn at 45 per cent.
                if disabled {
                    faded(colour, 0.45)
                } else {
                    colour
                }
            });
            assert_eq!(ground, wanted, "{theme} {kind:?} disabled: {disabled}");
        }
    }
}

#[gpui_kit::test]
fn a_disabled_button_calls_nothing_when_clicked(cx: &mut TestAppContext) {
    for theme in THEMES {
        let calls = counter();
        let handler = calls.clone();
        let mut shown = Shown::open(theme, cx, move || {
            let handler = handler.clone();
            Button::new("go", "Go")
                .disabled(true)
                .selector("go")
                .on_click(move |_, _, _| handler.set(handler.get() + 1))
                .into_any_element()
        });
        shown.click("go");
        shown.click("go");
        assert_eq!(calls.get(), 0, "{theme}");
    }
}

#[gpui_kit::test]
fn an_enabled_button_calls_its_handler_once_per_click(cx: &mut TestAppContext) {
    for theme in THEMES {
        let calls = counter();
        let handler = calls.clone();
        let mut shown = Shown::open(theme, cx, move || {
            let handler = handler.clone();
            Button::new("go", "Go")
                .small()
                .leading("+")
                .selector("go")
                .on_click(move |_, _, _| handler.set(handler.get() + 1))
                .into_any_element()
        });
        shown.click("go");
        assert_eq!(calls.get(), 1, "{theme}");
        shown.click("go");
        shown.click("go");
        assert_eq!(calls.get(), 3, "{theme}");
    }
}

#[gpui_kit::test]
fn a_small_button_is_shorter_than_a_normal_one(cx: &mut TestAppContext) {
    let mut normal = Shown::open(LIGHT, cx, || {
        Button::new("go", "Go").selector("go").into_any_element()
    });
    let mut small = Shown::open(LIGHT, cx, || {
        Button::new("go", "Go")
            .small()
            .selector("go")
            .into_any_element()
    });
    let (normal, small) = (normal.bounds("go"), small.bounds("go"));
    // Padding 4 and 10 against 1 and 8, plus a border of 1 on each side.
    assert!(
        normal.size.height - small.size.height >= px(6.),
        "{normal:?} {small:?}"
    );
    assert!(
        normal.size.width - small.size.width >= px(4.),
        "{normal:?} {small:?}"
    );
}

fn tabs() -> AnyElement {
    Tabs::new("tabs")
        .tab("changes", "Changes", Some(3), None)
        .tab("flow", "Flow", None, Some(State::Permission))
        .tab("files", "Files", None, None)
        .selected(1)
        .selector("tabs")
        .into_any_element()
}

#[gpui_kit::test]
fn a_selected_tab_is_painted_raised_and_the_others_are_not(cx: &mut TestAppContext) {
    in_both_themes(cx, tabs, |shown| {
        let raised = shown.palette().raised;
        assert_eq!(shown.ground("tabs-flow"), Some(raised));
        assert_eq!(shown.ground("tabs-changes"), None);
        assert_eq!(shown.ground("tabs-files"), None);
    });
}

#[gpui_kit::test]
fn clicking_a_tab_reports_its_index(cx: &mut TestAppContext) {
    for theme in THEMES {
        let picked = Rc::new(RefCell::new(Vec::new()));
        let heard = picked.clone();
        let mut shown = Shown::open(theme, cx, move || {
            let heard = heard.clone();
            Tabs::new("tabs")
                .tab("changes", "Changes", None, None)
                .tab("flow", "Flow", None, None)
                .tab("files", "Files", None, None)
                .selector("tabs")
                .on_select(move |index, _, _| heard.borrow_mut().push(index))
                .into_any_element()
        });
        shown.click("tabs-files");
        shown.click("tabs-changes");
        shown.click("tabs-flow");
        assert_eq!(*picked.borrow(), [2, 0, 1], "{theme}");
    }
}

#[gpui_kit::test]
fn a_tab_with_a_count_shows_the_count_and_one_without_shows_none(cx: &mut TestAppContext) {
    in_both_themes(cx, tabs, |shown| {
        assert!(shown.is_drawn("tabs-changes-count"));
        assert!(!shown.is_drawn("tabs-flow-count"));
        assert!(!shown.is_drawn("tabs-files-count"));
        let counted = shown.bounds("tabs-changes");
        assert!(inside(counted, shown.bounds("tabs-changes-count")));
    });
}

#[gpui_kit::test]
fn a_tab_shows_its_state_dot_in_the_states_colour_only_when_it_has_one(cx: &mut TestAppContext) {
    in_both_themes(cx, tabs, |shown| {
        let danger = shown.palette().danger;
        assert_eq!(shown.ground("tabs-flow-dot"), Some(danger));
        assert!(!shown.is_drawn("tabs-changes-dot"));
        assert!(!shown.is_drawn("tabs-files-dot"));
    });
}

#[gpui_kit::test]
fn a_tab_row_draws_what_is_given_to_trail_it_after_the_last_tab(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            Tabs::new("tabs")
                .tab("changes", "Changes", None, None)
                .tab("flow", "Flow", None, None)
                .trailing(Button::new("more", "More").selector("more"))
                .selector("tabs")
                .into_any_element()
        },
        |shown| {
            let (last, more) = (shown.bounds("tabs-flow"), shown.bounds("more"));
            assert!(more.left() >= last.right(), "{last:?} {more:?}");
            assert!(inside(shown.bounds("tabs"), more));
        },
    );
}

#[gpui_kit::test]
fn a_mono_tab_row_is_still_one_row_of_tabs_that_report_their_index(cx: &mut TestAppContext) {
    let picked = counter();
    let heard = picked.clone();
    let mut shown = Shown::open(LIGHT, cx, move || {
        let heard = heard.clone();
        Tabs::new("tabs")
            .mono()
            .tab("zsh", "zsh", None, None)
            .tab("claude", "claude", None, Some(State::Working))
            .selector("tabs")
            .on_select(move |index, _, _| heard.set(index + 10))
            .into_any_element()
    });
    shown.click("tabs-claude");
    assert_eq!(picked.get(), 11);
}

#[gpui_kit::test]
fn a_segmented_control_reports_the_index_clicked_and_paints_only_the_selected_one(
    cx: &mut TestAppContext,
) {
    for theme in THEMES {
        let picked = Rc::new(RefCell::new(Vec::new()));
        let heard = picked.clone();
        let mut shown = Shown::open(theme, cx, move || {
            let heard = heard.clone();
            Segmented::new("place")
                .segment("left", "Left")
                .segment("right", "Right")
                .segment("below", "Below")
                .selected(2)
                .selector("place")
                .on_select(move |index, _, _| heard.borrow_mut().push(index))
                .into_any_element()
        });
        let palette = shown.palette();
        assert_eq!(shown.ground("place-below"), Some(palette.raised), "{theme}");
        assert_eq!(shown.ground("place-left"), None, "{theme}");
        assert_eq!(shown.ground("place-right"), None, "{theme}");
        assert_eq!(shown.border("place"), palette.edge, "{theme}");
        shown.click("place-left");
        shown.click("place-right");
        shown.click("place-below");
        assert_eq!(*picked.borrow(), [0, 1, 2], "{theme}");
    }
}

#[gpui_kit::test]
fn the_selected_segment_moves_with_the_index_it_is_given(cx: &mut TestAppContext) {
    let mut shown = Shown::open(LIGHT, cx, || {
        Segmented::new("place")
            .segment("left", "Left")
            .segment("right", "Right")
            .selected(0)
            .selector("place")
            .into_any_element()
    });
    let raised = shown.palette().raised;
    assert_eq!(shown.ground("place-left"), Some(raised));
    assert_eq!(shown.ground("place-right"), None);
    // Without a handler a click is harmless.
    shown.click("place-right");
}

#[gpui_kit::test]
fn a_toggle_reports_each_click_and_its_dot_is_ok_when_on_and_dim_when_off(cx: &mut TestAppContext) {
    for theme in THEMES {
        let on = Rc::new(Cell::new(false));
        let clicks = counter();
        let (state, heard) = (on.clone(), clicks.clone());
        let mut shown = Shown::open(theme, cx, move || {
            let (state, heard) = (state.clone(), heard.clone());
            Toggle::new("follow", "Follow")
                .on(state.get())
                .selector("follow")
                .on_click(move |_, window, _| {
                    heard.set(heard.get() + 1);
                    state.set(!state.get());
                    window.refresh();
                })
                .into_any_element()
        });
        let palette = shown.palette();
        assert_eq!(
            shown.ground("follow-dot"),
            Some(palette.text_dim),
            "{theme}: off"
        );
        assert_eq!(shown.border("follow"), palette.edge, "{theme}");

        shown.click("follow");
        shown.cx.run_until_parked();
        assert_eq!(clicks.get(), 1, "{theme}");
        assert_eq!(shown.ground("follow-dot"), Some(palette.ok), "{theme}: on");

        shown.click("follow");
        shown.cx.run_until_parked();
        assert_eq!(clicks.get(), 2, "{theme}");
        assert_eq!(
            shown.ground("follow-dot"),
            Some(palette.text_dim),
            "{theme}: off again"
        );
    }
}

#[gpui_kit::test]
fn each_state_dot_is_painted_in_its_tokens_colour(cx: &mut TestAppContext) {
    let states: [(State, Token); 5] = [
        (State::Working, |p| p.link),
        (State::NeedsYou, |p| p.warn),
        (State::Ok, |p| p.ok),
        (State::Permission, |p| p.danger),
        (State::Idle, |p| p.text_dim),
    ];
    for theme in THEMES {
        for (state, token) in states {
            let mut shown = Shown::open(theme, cx, move || {
                StateDot::new(state).selector("dot").into_any_element()
            });
            let wanted = token(&shown.palette());
            assert_eq!(shown.ground("dot"), Some(wanted), "{theme} {state:?}");
        }
    }
}

#[gpui_kit::test]
fn a_state_dot_is_round_and_the_large_one_is_larger(cx: &mut TestAppContext) {
    let mut small = Shown::open(LIGHT, cx, || {
        StateDot::new(State::Ok).selector("dot").into_any_element()
    });
    let mut large = Shown::open(LIGHT, cx, || {
        StateDot::new(State::Ok)
            .large()
            .selector("dot")
            .into_any_element()
    });
    let (small_bounds, large_bounds) = (small.bounds("dot"), large.bounds("dot"));
    assert_eq!(small_bounds.size, size(px(8.), px(8.)));
    assert_eq!(large_bounds.size, size(px(10.), px(10.)));
    let quad = &small.quads_at("dot")[0];
    assert!(quad.corner_radii.top_left.0 >= quad.bounds.size.width.0 / 2.);
}

#[gpui_kit::test]
fn a_toned_chip_borders_in_its_tone_at_35_percent_and_a_plain_one_in_the_edge(
    cx: &mut TestAppContext,
) {
    let tones: [(Tone, Token); 4] = [
        (Tone::Ok, |p| p.ok),
        (Tone::Warn, |p| p.warn),
        (Tone::Info, |p| p.info),
        (Tone::Danger, |p| p.danger),
    ];
    for theme in THEMES {
        let mut shown = Shown::open(theme, cx, || {
            Chip::new("main").selector("chip").into_any_element()
        });
        let edge = shown.palette().edge;
        assert_eq!(shown.border("chip"), edge, "{theme}: plain");
        assert_eq!(shown.ground("chip"), None, "{theme}: plain");

        for (tone, token) in tones {
            let mut shown = Shown::open(theme, cx, move || {
                Chip::new("main")
                    .tone(tone)
                    .mono()
                    .selector("chip")
                    .into_any_element()
            });
            let wanted = faded(token(&shown.palette()), 0.35);
            assert_eq!(shown.border("chip"), wanted, "{theme} {tone:?}");
        }
    }
}

#[gpui_kit::test]
fn a_chip_is_a_pill(cx: &mut TestAppContext) {
    let mut shown = Shown::open(LIGHT, cx, || {
        Chip::new("main").selector("chip").into_any_element()
    });
    let quad = &shown.quads_at("chip")[0];
    let height = quad.bounds.size.height.0;
    assert!(quad.corner_radii.top_left.0 >= height / 2., "{quad:?}");
}

#[gpui_kit::test]
fn a_count_badge_is_a_pill_in_a_wash_of_the_text_colour(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || CountBadge::new(12).selector("count").into_any_element(),
        |shown| {
            let text = shown.palette().text;
            assert_eq!(shown.ground("count"), Some(faded(text, 0.14)));
            let quad = &shown.quads_at("count")[0];
            assert!(quad.corner_radii.top_left.0 >= quad.bounds.size.height.0 / 2.);
        },
    );
}

/// The piece with the selector, or without one.
macro_rules! named {
    ($piece:expr, $selector:expr) => {
        match $selector {
            Some(selector) => $piece.selector(selector).into_any_element(),
            None => $piece.into_any_element(),
        }
    };
}

#[gpui_kit::test]
fn a_selector_given_to_a_piece_finds_its_bounds(cx: &mut TestAppContext) {
    type Make = fn(Option<&'static str>) -> AnyElement;
    let makers: [(&str, Make); 7] = [
        ("button", |s| named!(Button::new("a", "A"), s)),
        ("chip", |s| named!(Chip::new("A"), s)),
        ("count", |s| named!(CountBadge::new(1), s)),
        ("tabs", |s| {
            named!(Tabs::new("a").tab("t", "T", None, None), s)
        }),
        ("segmented", |s| {
            named!(Segmented::new("a").segment("s", "A"), s)
        }),
        ("toggle", |s| named!(Toggle::new("a", "A"), s)),
        ("dot", |s| named!(StateDot::new(State::Idle), s)),
    ];
    for (name, make) in makers {
        let mut named = Shown::open(LIGHT, cx, move || make(Some("mine")));
        let found = named.bounds("mine");
        assert!(
            found.size.width > px(0.) && found.size.height > px(0.),
            "{name}: {found:?}"
        );
        let mut unnamed = Shown::open(LIGHT, cx, move || make(None));
        assert!(!unnamed.is_drawn("mine"), "{name}");
    }
}
