//! Each piece drawn alone in a test window, in the light and the dark theme. GPUI's test platform
//! draws no glyphs, so what is read is quads, bounds and callbacks.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{
    AnyElement, AppContext, BorderStyle, Bounds, Context, Entity, Hsla, InteractiveElement,
    IntoElement, Modifiers, ParentElement, Pixels, Render, Styled, TestAppContext,
    VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use yaar_ui::colour::composited;
use yaar_ui::diff::GutterPlus;
use yaar_ui::pieces::{
    Avatar, Banner, Button, ButtonKind, Card, Chip, CountBadge, Divider, EmptyState, Field,
    ListRow, Pane, Pill, Popover, SectionLabel, Segmented, State, StateDot, Tabs, Toast, Toggle,
    Tone,
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

    /// The quads painted anywhere inside the element's bounds that draw a border.
    fn bordered_quads_within(&mut self, selector: &str) -> Vec<gpui_kit::Quad> {
        let bounds = self.bounds(selector);
        let (scale, quads) = self
            .cx
            .update(|window, _| (window.scale_factor(), window.painted_quads()));
        let at = bounds.scale(scale);
        quads
            .into_iter()
            .filter(|quad| at.union(&quad.bounds) == at)
            .filter(|quad| quad.border_widths != Default::default())
            .collect()
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
    /// clipped strips, which all carry it; a filled one is a quad without a border and the strips.
    fn border(&mut self, selector: &str) -> Hsla {
        let all = self.quads_at(selector);
        let bordered: Vec<_> = all
            .iter()
            .filter(|quad| quad.border_widths != Default::default())
            .cloned()
            .collect();
        // A border that is fully transparent is not painted: the quad then has no widths.
        let quads = if bordered.is_empty() { all } else { bordered };
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
fn a_tab_row_draws_every_element_given_to_trail_it_in_the_order_given(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            Tabs::new("tabs")
                .tab("changes", "Changes", None, None)
                .trailing(Button::new("more", "More").selector("more"))
                .trailing(Button::new("less", "Less").selector("less"))
                .selector("tabs")
                .into_any_element()
        },
        |shown| {
            let (tab, more, less) = (
                shown.bounds("tabs-changes"),
                shown.bounds("more"),
                shown.bounds("less"),
            );
            assert!(tab.right() <= more.left(), "{tab:?} {more:?}");
            assert!(more.right() <= less.left(), "{more:?} {less:?}");
            assert!(inside(shown.bounds("tabs"), less));
        },
    );
}

#[gpui_kit::test]
fn a_mono_tab_row_is_still_one_row_of_tabs_that_report_their_index(cx: &mut TestAppContext) {
    let picked = counter();
    let heard = picked.clone();
    let mut shown = Shown::open(LIGHT, cx, move || {
        let heard = heard.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(
                Tabs::new("plain")
                    .tab("zsh", "zsh", None, None)
                    .tab("claude", "claude", None, Some(State::Working))
                    .selector("plain"),
            )
            .child(
                Tabs::new("tabs")
                    .mono()
                    .tab("zsh", "zsh", None, None)
                    .tab("claude", "claude", None, Some(State::Working))
                    .selector("tabs")
                    .on_select(move |index, _, _| heard.set(index + 10)),
            )
            .into_any_element()
    });
    let (first, second) = (shown.bounds("tabs-zsh"), shown.bounds("tabs-claude"));
    assert_eq!(first.top(), second.top(), "one row");
    assert!(first.right() <= second.left(), "side by side");
    let plain = shown.bounds("plain-zsh");
    assert!(
        first.size.height < plain.size.height,
        "the mono tab is shorter than a plain one: {first:?} {plain:?}"
    );
    shown.click("tabs-claude");
    assert_eq!(picked.get(), 11);
}

#[gpui_kit::test]
fn clicking_a_tab_close_button_reports_close_without_selecting_the_tab(cx: &mut TestAppContext) {
    for theme in THEMES {
        let selected = Rc::new(RefCell::new(Vec::new()));
        let closed = Rc::new(RefCell::new(Vec::new()));
        let (heard_select, heard_close) = (selected.clone(), closed.clone());
        let mut shown = Shown::open(theme, cx, move || {
            let (heard_select, heard_close) = (heard_select.clone(), heard_close.clone());
            Tabs::new("tabs")
                .mono()
                .tab("one", "Shell", None, None)
                .tab("two", "Claude Code", None, Some(State::Working))
                .selected(0)
                .selector("tabs")
                .on_select(move |index, _, _| heard_select.borrow_mut().push(index))
                .on_close(move |index, _, _| heard_close.borrow_mut().push(index))
                .into_any_element()
        });
        assert!(shown.is_drawn("tabs-one-close"), "{theme}");
        assert!(shown.is_drawn("tabs-two-close"), "{theme}");
        assert!(
            inside(shown.bounds("tabs-two"), shown.bounds("tabs-two-close")),
            "{theme}"
        );

        shown.click("tabs-two-close");
        assert_eq!(*closed.borrow(), [1], "{theme}");
        assert!(
            selected.borrow().is_empty(),
            "{theme}: closing a tab must not select it first"
        );

        let tab_bounds = shown.bounds("tabs-two");
        let label_point = point(tab_bounds.left() + px(12.), tab_bounds.center().y);
        shown.cx.simulate_click(label_point, Modifiers::none());
        assert_eq!(*selected.borrow(), [1], "{theme}");
        assert_eq!(*closed.borrow(), [1], "{theme}");
    }

    let mut plain = Shown::open(LIGHT, cx, tabs);
    assert!(!plain.is_drawn("tabs-changes-close"));
    assert!(!plain.is_drawn("tabs-flow-close"));
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
    // Without a handler a click is harmless: the selection stays where it was given.
    shown.click("place-right");
    assert_eq!(shown.ground("place-left"), Some(raised));
    assert_eq!(shown.ground("place-right"), None);
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

impl Shown {
    /// The corner radius of the element's own quad, in logical pixels.
    fn radius(&mut self, selector: &str) -> f32 {
        let scale = self.cx.update(|window, _| window.scale_factor());
        let quads = self.quads_at(selector);
        let quad = quads
            .first()
            .unwrap_or_else(|| panic!("{selector}: no quad at its bounds"));
        quad.corner_radii.top_left.0 / scale
    }

    fn window_bounds(&mut self) -> Bounds<Pixels> {
        self.cx.update(|window, _| window.bounds())
    }

    /// Draws the frame again after something the pieces read has changed.
    fn redraw(&mut self) {
        self.cx.update(|window, _| window.refresh());
        self.cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn a_pane_is_painted_on_the_pane_ground_inside_an_edge_border(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            Pane::new()
                .w(px(300.))
                .h(px(100.))
                .selector("pane")
                .child(div().child("inside"))
                .into_any_element()
        },
        |shown| {
            let palette = shown.palette();
            assert_eq!(shown.ground("pane"), Some(palette.pane));
            assert_eq!(shown.border("pane"), palette.edge);
            assert_eq!(shown.radius("pane"), 10.);
            // The layout methods the pane takes reach it.
            assert_eq!(shown.bounds("pane").size, size(px(300.), px(100.)));
        },
    );
}

#[gpui_kit::test]
fn a_card_is_on_the_window_ground_in_an_edge_border_and_a_dashed_one_is_an_outline(
    cx: &mut TestAppContext,
) {
    in_both_themes(
        cx,
        || {
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(Card::new().selector("card").child(div().child("a thread")))
                .child(
                    Card::new()
                        .dashed()
                        .selector("end")
                        .child(div().child("the end")),
                )
                .into_any_element()
        },
        |shown| {
            let palette = shown.palette();
            assert_eq!(shown.ground("card"), Some(palette.window));
            assert_eq!(shown.border("card"), palette.edge);
            assert_eq!(shown.radius("card"), 10.);
            assert_eq!(shown.quads_at("card")[0].border_style, BorderStyle::Solid);
            assert_eq!(shown.ground("end"), None);
            assert_eq!(shown.border("end"), palette.edge);
            assert!(
                shown
                    .quads_at("end")
                    .iter()
                    .all(|quad| quad.border_style == BorderStyle::Dashed)
            );
        },
    );
}

#[gpui_kit::test]
fn a_cards_header_sits_above_its_children_under_a_separator(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            Card::new()
                .selector("card")
                .header(
                    div()
                        .id("title")
                        .debug_selector(|| "title".into())
                        .child("Thread"),
                )
                .child(div().debug_selector(|| "body".into()).child("text"))
                .into_any_element()
        },
        |shown| {
            let (card, header) = (shown.bounds("card"), shown.bounds("card-header"));
            let (title, body) = (shown.bounds("title"), shown.bounds("body"));
            assert!(inside(card, header) && inside(header, title));
            assert!(header.bottom() <= body.top(), "{header:?} {body:?}");
            assert!(header.size.height >= px(32.));
            // The rule under the header is drawn in the separator.
            let separator = shown.palette().separator;
            let scale = shown.cx.update(|window, _| window.scale_factor());
            let at = header.scale(scale);
            let quads = shown.cx.update(|window, _| window.painted_quads());
            assert!(quads.iter().any(|quad| {
                quad.bounds == at
                    && quad.border_color == separator
                    && quad.border_widths.bottom.0 > 0.
            }));
        },
    );
}

#[gpui_kit::test]
fn a_selected_row_is_painted_raised_and_an_unselected_one_is_not(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(300.))
                .flex()
                .flex_col()
                .child(
                    ListRow::new("one")
                        .selected(true)
                        .selector("one")
                        .child("One"),
                )
                .child(ListRow::new("two").selector("two").child("Two"))
                .into_any_element()
        },
        |shown| {
            let raised = shown.palette().raised;
            assert_eq!(shown.ground("one"), Some(raised));
            assert_eq!(shown.ground("two"), None);
            assert_eq!(shown.radius("one"), 6.);
            // Both fill the width they are given.
            assert_eq!(shown.bounds("one").size.width, px(300.));
        },
    );
}

#[gpui_kit::test]
fn a_row_under_the_pointer_is_raised_and_a_click_on_it_is_reported(cx: &mut TestAppContext) {
    for theme in THEMES {
        let clicks = counter();
        let heard = clicks.clone();
        let mut shown = Shown::open(theme, cx, move || {
            let heard = heard.clone();
            ListRow::new("row")
                .selector("row")
                .on_click(move |_, _, _| heard.set(heard.get() + 1))
                .child("Row")
                .into_any_element()
        });
        let raised = shown.palette().raised;
        assert_eq!(shown.ground("row"), None, "{theme}");
        shown.hover("row");
        assert_eq!(shown.ground("row"), Some(raised), "{theme}");
        shown.click("row");
        assert_eq!(clicks.get(), 1, "{theme}");
    }
}

#[gpui_kit::test]
fn a_nested_row_starts_twenty_two_pixels_right_of_its_parent(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(300.))
                .flex()
                .flex_col()
                .child(
                    ListRow::new("parent")
                        .selector("parent")
                        .child(div().debug_selector(|| "parent-text".into()).child("repo")),
                )
                .child(
                    ListRow::new("child").nested().selector("child").child(
                        div()
                            .debug_selector(|| "child-text".into())
                            .child("worktree"),
                    ),
                )
                .into_any_element()
        },
        |shown| {
            let offset = |shown: &mut Shown, row: &str, text: &str| {
                shown.bounds(text).left() - shown.bounds(row).left()
            };
            assert_eq!(offset(shown, "parent", "parent-text"), px(10.));
            assert_eq!(offset(shown, "child", "child-text"), px(22.));
            assert_eq!(shown.bounds("child").left(), shown.bounds("parent").left());
        },
    );
}

#[gpui_kit::test]
fn a_section_label_is_drawn_with_its_text(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            SectionLabel::new("Workspaces")
                .selector("label")
                .into_any_element()
        },
        |shown| {
            let label = shown.bounds("label");
            assert!(label.size.width > px(0.) && label.size.height > px(0.));
        },
    );
}

#[gpui_kit::test]
fn a_banner_is_painted_on_its_tones_wash_over_the_pane_and_the_live_one_on_the_plain_wash(
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
            Banner::new("Plain").selector("banner").into_any_element()
        });
        let palette = shown.palette();
        // The stage is the window ground, and the banner's own ground does not show it through.
        let plain = composited(faded(palette.text, 0.03), palette.pane);
        assert_eq!(shown.ground("banner"), Some(plain), "{theme}");

        for (tone, token) in tones {
            let mut shown = Shown::open(theme, cx, move || {
                Banner::new("Notice")
                    .tone(tone)
                    .selector("banner")
                    .into_any_element()
            });
            let palette = shown.palette();
            let wanted = composited(faded(token(&palette), 0.08), palette.pane);
            assert_eq!(shown.ground("banner"), Some(wanted), "{theme} {tone:?}");
        }

        let mut shown = Shown::open(theme, cx, || {
            Banner::new("Working")
                .live()
                .selector("banner")
                .into_any_element()
        });
        let palette = shown.palette();
        let plain = composited(faded(palette.text, 0.03), palette.pane);
        assert_eq!(shown.ground("banner"), Some(plain), "{theme} live");
    }
}

#[gpui_kit::test]
fn a_banner_holds_its_action_at_its_end(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(400.))
                .child(
                    Banner::new("Notice")
                        .action(Button::new("act", "Retry").small().selector("act"))
                        .selector("banner"),
                )
                .into_any_element()
        },
        |shown| {
            let (banner, action) = (shown.bounds("banner"), shown.bounds("act"));
            assert!(inside(banner, action));
            assert!(
                banner.right() - action.right() <= px(14.),
                "{banner:?} {action:?}"
            );
        },
    );
}

#[gpui_kit::test]
fn a_toast_stays_until_dismissed_and_reports_the_dismissal(cx: &mut TestAppContext) {
    for theme in THEMES {
        let gone = Rc::new(Cell::new(false));
        let calls = counter();
        let (flag, heard) = (gone.clone(), calls.clone());
        let mut shown = Shown::open(theme, cx, move || {
            if flag.get() {
                return div().into_any_element();
            }
            let (flag, heard) = (flag.clone(), heard.clone());
            Toast::new("toast", "Saved")
                .selector("toast")
                .action(Button::new("undo", "Undo").small().selector("undo"))
                .on_dismiss(move |window, _| {
                    heard.set(heard.get() + 1);
                    flag.set(true);
                    window.refresh();
                })
                .into_any_element()
        });
        let palette = shown.palette();
        assert_eq!(shown.ground("toast"), Some(palette.raised), "{theme}");
        assert_eq!(shown.border("toast"), palette.edge, "{theme}");
        assert_eq!(shown.radius("toast"), 6., "{theme}");
        assert!(inside(shown.bounds("toast"), shown.bounds("toast-dismiss")));
        assert!(inside(shown.bounds("toast"), shown.bounds("undo")));

        // Nothing but the dismissal takes it away, however long it is left.
        shown
            .cx
            .executor()
            .advance_clock(Duration::from_secs(3_600));
        shown.redraw();
        assert!(
            shown.is_drawn("toast"),
            "{theme}: still there after an hour"
        );
        assert_eq!(calls.get(), 0, "{theme}");

        shown.click("toast-dismiss");
        shown.cx.run_until_parked();
        assert_eq!(calls.get(), 1, "{theme}");
        assert!(!shown.is_drawn("toast"), "{theme}: gone once dismissed");
    }
}

#[gpui_kit::test]
fn a_pill_is_a_raised_pill_that_holds_its_action(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            Pill::new("pill", "Agent is working")
                .action(Button::new("stop", "Stop").small().selector("stop"))
                .selector("pill")
                .into_any_element()
        },
        |shown| {
            let palette = shown.palette();
            assert_eq!(shown.ground("pill"), Some(palette.raised));
            assert_eq!(shown.border("pill"), palette.edge);
            let pill = shown.bounds("pill");
            assert!(shown.radius("pill") >= f32::from(pill.size.height) / 2.);
            assert!(inside(pill, shown.bounds("stop")));
        },
    );
}

#[gpui_kit::test]
fn the_two_avatars_are_round_22_pixel_discs_in_the_accent_and_the_raised_ground(
    cx: &mut TestAppContext,
) {
    for theme in THEMES {
        for (agent, token) in [
            (false, (|p: &Palette| p.accent) as Token),
            (true, |p| p.raised),
        ] {
            let mut shown = Shown::open(theme, cx, move || {
                let avatar = if agent {
                    Avatar::agent()
                } else {
                    Avatar::you()
                };
                avatar.selector("avatar").into_any_element()
            });
            let wanted = token(&shown.palette());
            assert_eq!(
                shown.ground("avatar"),
                Some(wanted),
                "{theme} agent: {agent}"
            );
            assert_eq!(shown.bounds("avatar").size, size(px(22.), px(22.)));
            assert_eq!(shown.radius("avatar"), 11.);
        }
    }
}

#[gpui_kit::test]
fn an_empty_state_centres_its_title_lines_and_action(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            EmptyState::new("Nothing to review")
                .line("The agent has not changed any file in this workspace yet.")
                .action(Button::new("go", "Refresh").selector("go"))
                .selector("empty")
                .into_any_element()
        },
        |shown| {
            let empty = shown.bounds("empty");
            // The line is the widest part, so the button is what shows the centring.
            let go = shown.bounds("go");
            assert!(inside(empty, go));
            assert!(go.left() > empty.left() + px(24.), "{empty:?} {go:?}");
            let (left, right) = (go.left() - empty.left(), empty.right() - go.right());
            assert!((left - right).abs() <= px(1.), "{left:?} {right:?}");
            assert!(empty.size.height >= px(80.));
        },
    );
}

#[gpui_kit::test]
fn an_empty_state_wraps_long_lines_inside_a_narrow_pane(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(300.))
                .flex()
                .flex_col()
                .child(
                    EmptyState::new("No terminal open")
                        .line(
                            "Start an agent CLI (Cmd-Shift-T) or a shell (Cmd-T) in this workspace.",
                        )
                        .action(Button::new("go", "Open Claude Code").selector("go"))
                        .selector("empty"),
                )
                .into_any_element()
        },
        |shown| {
            let empty = shown.bounds("empty");
            assert!(empty.size.width <= px(300.), "{empty:?}");
            assert!(inside(empty, shown.bounds("go")));
        },
    );
}

#[gpui_kit::test]
fn a_strong_divider_is_painted_in_edge_and_a_plain_one_in_separator(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(300.))
                .h(px(100.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(Divider::horizontal().selector("plain"))
                .child(Divider::horizontal().strong().selector("strong"))
                .child(
                    div()
                        .h(px(40.))
                        .flex()
                        .gap(px(10.))
                        .child(Divider::vertical().selector("upright"))
                        .child(Divider::vertical().strong().selector("upright-strong")),
                )
                .into_any_element()
        },
        |shown| {
            let palette = shown.palette();
            assert_eq!(shown.ground("plain"), Some(palette.separator));
            assert_eq!(shown.ground("strong"), Some(palette.edge));
            assert_eq!(shown.ground("upright"), Some(palette.separator));
            assert_eq!(shown.ground("upright-strong"), Some(palette.edge));
            assert_eq!(shown.bounds("plain").size, size(px(300.), px(1.)));
            assert_eq!(shown.bounds("upright").size, size(px(1.), px(40.)));
        },
    );
}

/// Opens one piece that draws a toolkit input, whose state needs the window to exist.
fn open_with_state<S: 'static>(
    theme: &str,
    cx: &mut TestAppContext,
    make: impl FnOnce(&mut Window, &mut gpui_kit::Context<S>) -> S,
    piece: impl Fn(&Entity<S>) -> AnyElement + 'static,
) -> (Shown, Entity<S>) {
    let cell: Rc<RefCell<Option<Entity<S>>>> = Rc::new(RefCell::new(None));
    let seen = cell.clone();
    let mut shown = Shown::open(theme, cx, move || match seen.borrow().as_ref() {
        Some(state) => piece(state),
        None => div().into_any_element(),
    });
    let state = shown
        .cx
        .update(|window, app| app.new(|cx| make(window, cx)));
    *cell.borrow_mut() = Some(state.clone());
    shown.redraw();
    (shown, state)
}

#[gpui_kit::test]
fn a_field_holds_the_kits_input_and_typing_into_it_changes_its_text(cx: &mut TestAppContext) {
    for theme in THEMES {
        let (mut shown, state) = open_with_state(theme, cx, InputState::new, |state| {
            Field::input(state)
                .footer(Button::new("save", "Save").small().selector("save"))
                .selector("field")
                .into_any_element()
        });
        let palette = shown.palette();
        assert_eq!(shown.ground("field"), Some(palette.window), "{theme}");
        assert_eq!(shown.border("field"), palette.edge, "{theme}");
        assert_eq!(shown.radius("field"), 10., "{theme}");
        let (field, input) = (shown.bounds("field"), shown.bounds("field-input"));
        assert!(inside(field, input), "{field:?} {input:?}");
        assert!(input.size.width > px(100.) && input.size.height > px(10.));
        assert!(
            shown.bordered_quads_within("field-input").is_empty(),
            "{theme}: the input draws no box inside the field's"
        );
        let (footer, save) = (shown.bounds("field-footer"), shown.bounds("save"));
        assert!(input.bottom() <= footer.top() && inside(footer, save));

        shown.cx.update(|window, app| {
            state.update(app, |state, cx| state.focus(window, cx));
        });
        assert_eq!(state.read_with(&shown.cx, |s, _| s.value()), "");
        shown.cx.simulate_keystrokes("h e l l o");
        assert_eq!(
            state.read_with(&shown.cx, |s, _| s.value()),
            "hello",
            "{theme}"
        );
        shown.cx.simulate_keystrokes("backspace x");
        assert_eq!(
            state.read_with(&shown.cx, |s, _| s.value()),
            "hellx",
            "{theme}"
        );
    }
}

#[gpui_kit::test]
fn a_textarea_field_takes_several_lines(cx: &mut TestAppContext) {
    let (mut shown, state) = open_with_state(LIGHT, cx, TextareaState::new, |state| {
        Field::textarea(state).selector("field").into_any_element()
    });
    let (field, input) = (shown.bounds("field"), shown.bounds("field-input"));
    assert!(inside(field, input), "{field:?} {input:?}");
    assert!(input.size.height >= px(40.), "{input:?}");
    assert!(
        shown.bordered_quads_within("field-input").is_empty(),
        "the textarea draws no box inside the field's"
    );
    shown.cx.update(|window, app| {
        state.update(app, |state, cx| state.focus(window, cx));
    });
    shown.cx.simulate_keystrokes("a enter b");
    assert_eq!(state.read_with(&shown.cx, |s, _| s.value()), "a\nb");
}

/// A popover under test, placed by the anchor and the height of what it holds.
fn popover_shown(
    theme: &str,
    cx: &mut TestAppContext,
    anchor: Bounds<Pixels>,
    height: Pixels,
) -> Shown {
    Shown::open(theme, cx, move || {
        div()
            .child(
                Popover::new(anchor)
                    .selector("pop")
                    .child(div().h(height).child("hover text")),
            )
            .into_any_element()
    })
}

#[gpui_kit::test]
fn a_popover_opens_under_its_anchor_on_the_pane_ground(cx: &mut TestAppContext) {
    for theme in THEMES {
        let anchor = Bounds {
            origin: point(px(30.), px(20.)),
            size: size(px(60.), px(18.)),
        };
        let mut shown = popover_shown(theme, cx, anchor, px(60.));
        let palette = shown.palette();
        let pop = shown.bounds("pop");
        assert_eq!(pop.left(), anchor.left(), "{theme}");
        assert_eq!(pop.top(), anchor.bottom() + px(6.), "{theme}");
        assert_eq!(pop.size.width, px(460.), "{theme}");
        assert_eq!(shown.ground("pop"), Some(palette.pane), "{theme}");
        assert_eq!(shown.border("pop"), palette.edge, "{theme}");
        assert_eq!(shown.radius("pop"), 10., "{theme}");
    }
}

#[gpui_kit::test]
fn a_popover_near_the_right_edge_is_moved_to_stay_inside_the_window(cx: &mut TestAppContext) {
    for theme in THEMES {
        let near_right = Bounds {
            origin: point(px(520.), px(20.)),
            size: size(px(60.), px(18.)),
        };
        let mut shown = popover_shown(theme, cx, near_right, px(60.));
        let (window, pop) = (shown.window_bounds(), shown.bounds("pop"));
        assert!(inside(window, pop), "{theme}: {pop:?} in {window:?}");
        assert_eq!(pop.size.width, px(460.), "{theme}");
        assert!(pop.left() < near_right.left(), "{theme}: moved left");
        assert_eq!(pop.top(), near_right.bottom() + px(6.), "{theme}");

        // Under an anchor near the bottom there is no room below: it moves up.
        let near_bottom = Bounds {
            origin: point(px(100.), px(150.)),
            size: size(px(60.), px(18.)),
        };
        let mut shown = popover_shown(theme, cx, near_bottom, px(100.));
        let (window, pop) = (shown.window_bounds(), shown.bounds("pop"));
        assert!(inside(window, pop), "{theme}: {pop:?} in {window:?}");
        assert!(
            pop.bottom() <= near_bottom.top(),
            "{theme}: above the anchor, not over it: {pop:?}"
        );
    }
}

#[gpui_kit::test]
fn a_popover_in_a_window_narrower_than_itself_narrows_to_fit(cx: &mut TestAppContext) {
    let mut shown = popover_shown(
        LIGHT,
        cx,
        Bounds {
            origin: point(px(10.), px(10.)),
            size: size(px(40.), px(18.)),
        },
        px(40.),
    );
    shown.cx.simulate_resize(size(px(300.), px(200.)));
    shown.redraw();
    let (window, pop) = (shown.window_bounds(), shown.bounds("pop"));
    assert!(inside(window, pop), "{pop:?} in {window:?}");
    assert!(pop.size.width <= px(284.), "{pop:?}");
}

#[gpui_kit::test]
fn a_selector_given_to_a_surface_piece_finds_its_bounds(cx: &mut TestAppContext) {
    type Make = fn(Option<&'static str>) -> AnyElement;
    let makers: [(&str, Make); 9] = [
        ("pane", |s| named!(Pane::new(), s)),
        ("card", |s| named!(Card::new(), s)),
        ("row", |s| named!(ListRow::new("a"), s)),
        ("label", |s| named!(SectionLabel::new("a"), s)),
        ("banner", |s| named!(Banner::new("a"), s)),
        ("toast", |s| named!(Toast::new("a", "a"), s)),
        ("pill", |s| named!(Pill::new("a", "a"), s)),
        ("avatar", |s| named!(Avatar::you(), s)),
        ("empty", |s| named!(EmptyState::new("a"), s)),
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

#[gpui_kit::test]
fn a_second_action_is_shown_beside_the_first_on_each_piece_that_takes_one(cx: &mut TestAppContext) {
    type Make = fn() -> AnyElement;
    let makers: [(&str, &str, Make); 4] = [
        ("banner", "banner", || {
            Banner::new("A language server is missing")
                .action(Button::new("one", "Install").small().selector("one"))
                .action(Button::new("two", "Not now").small().selector("two"))
                .selector("banner")
                .into_any_element()
        }),
        ("toast", "toast", || {
            Toast::new("t", "Saved")
                .action(Button::new("one", "Undo").small().selector("one"))
                .action(Button::new("two", "View").small().selector("two"))
                .selector("toast")
                .into_any_element()
        }),
        ("pill", "pill", || {
            Pill::new("p", "Working")
                .action(Button::new("one", "Stop").small().selector("one"))
                .action(Button::new("two", "Open").small().selector("two"))
                .selector("pill")
                .into_any_element()
        }),
        ("empty", "empty", || {
            EmptyState::new("Nothing here")
                .action(Button::new("one", "Refresh").selector("one"))
                .action(Button::new("two", "Settings").selector("two"))
                .selector("empty")
                .into_any_element()
        }),
    ];
    for theme in THEMES {
        for (name, root, make) in makers {
            let mut shown = Shown::open(theme, cx, make);
            let (root, one, two) = (shown.bounds(root), shown.bounds("one"), shown.bounds("two"));
            assert!(inside(root, one) && inside(root, two), "{theme} {name}");
            // Beside each other in a row, or one under the other in the centred column.
            assert!(
                one.right() <= two.left() || one.bottom() <= two.top(),
                "{theme} {name}: {one:?} {two:?}"
            );
        }
    }
}

#[gpui_kit::test]
fn a_list_row_aligns_multi_line_children_at_the_top_and_a_rail_row_centres_them(
    cx: &mut TestAppContext,
) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(220.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    ListRow::new("full")
                        .selector("full")
                        .child(div().debug_selector(|| "dot".into()).size(px(8.)))
                        .child(
                            div()
                                .debug_selector(|| "lines".into())
                                .flex()
                                .flex_col()
                                .child("line one")
                                .child("line two"),
                        ),
                )
                .child(
                    div().w(px(54.)).child(
                        ListRow::rail("rail")
                            .selected(true)
                            .selector("rail")
                            .child(div().debug_selector(|| "mark".into()).size(px(12.))),
                    ),
                )
                .into_any_element()
        },
        |shown| {
            let (dot, lines) = (shown.bounds("dot"), shown.bounds("lines"));
            assert_eq!(dot.top(), lines.top(), "dot aligns with the first line");
            let (rail, mark) = (shown.bounds("rail"), shown.bounds("mark"));
            assert_eq!(shown.ground("rail"), Some(shown.palette().raised));
            let (left, right) = (mark.left() - rail.left(), rail.right() - mark.right());
            assert!((left - right).abs() <= px(1.), "{left:?} {right:?}");
        },
    );
}

#[gpui_kit::test]
fn a_pulsing_working_dot_draws_in_its_colour_and_holds_steady_under_reduced_motion(
    cx: &mut TestAppContext,
) {
    for reduce in [false, true] {
        cx.update(|cx| cx.set_reduce_motion(reduce));
        let mut shown = Shown::open(DARK, cx, || {
            StateDot::new(State::Working)
                .pulsing("pulse-test")
                .selector("dot")
                .into_any_element()
        });
        let link = shown.palette().link;
        assert_eq!(shown.ground("dot"), Some(link), "reduce={reduce}");
    }
    cx.update(|cx| cx.set_reduce_motion(false));
}

#[gpui_kit::test]
fn a_card_banner_and_selected_card_paint_their_borders_and_grounds(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(400.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    Banner::new("Notice")
                        .tone(Tone::Danger)
                        .card()
                        .selector("banner"),
                )
                .child(
                    Card::new()
                        .selected(true)
                        .selector("card")
                        .child("Selected"),
                )
                .child(
                    EmptyState::new("Open a repository")
                        .display()
                        .line("Pick a folder")
                        .selector("welcome"),
                )
                .into_any_element()
        },
        |shown| {
            let palette = shown.palette();
            assert_eq!(shown.border("banner"), palette.edge);
            assert_eq!(shown.radius("banner"), 10.);
            assert_eq!(shown.border("card"), palette.focus);
            assert_eq!(shown.ground("card"), Some(palette.raised));
            assert!(shown.is_drawn("welcome"));
        },
    );
}

#[gpui_kit::test]
fn a_toast_clamps_its_width_to_a_narrow_viewport(cx: &mut TestAppContext) {
    let mut shown = Shown::open(DARK, cx, || {
        Toast::new(
            "narrow",
            "A very long toast message that would otherwise stretch past a narrow window edge",
        )
        .selector("narrow")
        .into_any_element()
    });
    shown.cx.simulate_resize(size(px(260.), px(400.)));
    shown.redraw();
    let toast = shown.bounds("narrow");
    assert!(
        toast.size.width <= px(220.5),
        "toast width {:?} exceeds viewport - 40",
        toast.size.width
    );
}

#[gpui_kit::test]
fn a_focused_pane_highlights_its_border_and_draws_a_top_focus_bar(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .flex()
                .gap(px(8.))
                .child(
                    Pane::new()
                        .focused(true)
                        .w(px(200.))
                        .h(px(100.))
                        .selector("focused")
                        .child(div().child("active")),
                )
                .child(
                    Pane::new()
                        .w(px(200.))
                        .h(px(100.))
                        .selector("idle")
                        .child(div().child("inactive")),
                )
                .into_any_element()
        },
        |shown| {
            let palette = shown.palette();
            assert_eq!(shown.border("focused"), palette.focus.opacity(0.55));
            assert_eq!(shown.border("idle"), palette.edge);
            assert!(shown.is_drawn("focused-focus"));
            assert!(!shown.is_drawn("idle-focus"));
            assert_eq!(
                shown.ground("focused-focus"),
                Some(palette.focus.opacity(0.85))
            );
            let bar = shown.bounds("focused-focus");
            assert_eq!(bar.size.height, px(2.));
            assert_eq!(shown.bounds("focused").size, shown.bounds("idle").size);
        },
    );
}

#[gpui_kit::test]
fn an_elevated_and_accented_card_lifts_above_the_pane_with_a_left_bar(cx: &mut TestAppContext) {
    for theme in THEMES {
        let mut shown = Shown::open(theme, cx, || {
            Card::new()
                .elevated()
                .w(px(240.))
                .selector("card")
                .child(div().child("thread"))
                .into_any_element()
        });
        let palette = shown.palette();
        let lifted = composited(palette.raised.opacity(0.45), palette.pane);
        assert_eq!(shown.ground("card"), Some(lifted), "{theme}");
        assert!(!shown.is_drawn("card-accent"), "{theme}");

        let accent = palette.ok;
        let mut accented = Shown::open(theme, cx, move || {
            Card::new()
                .elevated()
                .accent(accent)
                .w(px(240.))
                .selector("card")
                .child(div().child("thread"))
                .into_any_element()
        });
        assert_eq!(accented.ground("card"), Some(lifted), "{theme}");
        assert_eq!(accented.ground("card-accent"), Some(accent), "{theme}");
        assert_eq!(accented.bounds("card-accent").size.width, px(2.));
    }
}

#[gpui_kit::test]
fn a_list_row_accent_bar_paints_accent_text_and_focus_takes_precedence(cx: &mut TestAppContext) {
    in_both_themes(
        cx,
        || {
            div()
                .w(px(240.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    ListRow::new("active")
                        .accent_bar(true)
                        .selector("active")
                        .child("Workspace"),
                )
                .child(
                    ListRow::new("both")
                        .accent_bar(true)
                        .focused(true)
                        .selector("both")
                        .child("Focused workspace"),
                )
                .child(ListRow::new("plain").selector("plain").child("Plain"))
                .into_any_element()
        },
        |shown| {
            let palette = shown.palette();
            assert_eq!(shown.ground("active-bar"), Some(palette.accent_text));
            assert_eq!(shown.bounds("active-bar").size.width, px(2.));
            assert_eq!(shown.ground("both-bar"), Some(palette.focus));
            assert!(!shown.is_drawn("plain-bar"));
        },
    );
}

#[gpui_kit::test]
fn a_gutter_plus_reveals_on_group_hover_and_escalates_on_direct_hover(cx: &mut TestAppContext) {
    for theme in THEMES {
        let mut shown = Shown::open(theme, cx, || {
            div()
                .group("diff-row")
                .w(px(200.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    GutterPlus::new("plus")
                        .reveal_on_group_hover("diff-row")
                        .selector("plus"),
                )
                .child(
                    div()
                        .flex_1()
                        .debug_selector(|| "code".into())
                        .child("let x = 1;"),
                )
                .into_any_element()
        });
        let palette = shown.palette();
        assert_eq!(shown.ground("plus"), None, "{theme}: hidden at rest");
        shown.hover("code");
        assert_eq!(
            shown.ground("plus"),
            Some(palette.raised),
            "{theme}: revealed on row hover"
        );
        shown.hover("plus");
        assert_eq!(
            shown.ground("plus"),
            Some(palette.accent),
            "{theme}: accented on direct hover"
        );
    }
}

#[gpui_kit::test]
fn hovering_an_unselected_interactive_segment_tints_its_ground(cx: &mut TestAppContext) {
    for theme in THEMES {
        let mut shown = Shown::open(theme, cx, || {
            Segmented::new("mode")
                .segment("unified", "Unified")
                .segment("split", "Split")
                .selected(0)
                .selector("mode")
                .on_select(|_, _, _| {})
                .into_any_element()
        });
        let palette = shown.palette();
        assert_eq!(shown.ground("mode-split"), None, "{theme}");
        shown.hover("mode-split");
        assert_eq!(
            shown.ground("mode-split"),
            Some(palette.raised.opacity(0.50)),
            "{theme}"
        );
    }
}
