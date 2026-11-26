//! Component test: where the Frame lays the panes beside the first, and the key that moves them.
//! The Frame knows no pane by name.

mod common;

use gpui_kit::{
    AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, Modifiers,
    MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Render, Size, Styled, TestAppContext,
    VisualTestContext, Window, div, point, px, size,
};
use yaar_frame::{
    Beside, BuildPane, FULL_WIDTH, FocusNextPane, MovePanes, Pane, TITLEBAR_HEIGHT, TOP_INSET,
};
use yaar_ui::metrics::GUTTER;

use common::{drawn, open_frame, pressed};

const WINDOW: Size<Pixels> = Size {
    width: px(1280.),
    height: px(800.),
};

struct Plain {
    focus: FocusHandle,
}

impl Render for Plain {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

fn plain(name: &'static str) -> BuildPane {
    Box::new(move |_, _, cx| {
        let focus = cx.focus_handle();
        let view = cx.new(|_| Plain {
            focus: focus.clone(),
        });
        Pane {
            name,
            view: view.into(),
            focus,
        }
    })
}

fn pane(name: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    drawn(&format!("frame-pane-{name}"), cx).unwrap_or_else(|| panic!("the pane {name} is drawn"))
}

fn beside(frame: &gpui_kit::Entity<yaar_frame::Frame>, cx: &mut VisualTestContext) -> Beside {
    frame.read_with(cx, |frame, _| frame.beside())
}

fn assert_lies(place: Beside, first: Bounds<Pixels>, second: Bounds<Pixels>) {
    match place {
        Beside::Right => {
            assert!(
                second.left() >= first.right(),
                "right: {first:?} {second:?}"
            );
            assert_eq!(second.top(), first.top(), "right: the tops");
        }
        Beside::Below => {
            assert!(
                second.top() >= first.bottom(),
                "below: {first:?} {second:?}"
            );
            assert_eq!(second.left(), first.left(), "below: the lefts");
        }
        Beside::Left => {
            assert!(second.right() <= first.left(), "left: {first:?} {second:?}");
            assert_eq!(second.top(), first.top(), "left: the tops");
        }
    }
}

fn assert_split(place: Beside, first: Bounds<Pixels>, second: Bounds<Pixels>) {
    match place {
        Beside::Right | Beside::Left => {
            assert_eq!(second.size.height, first.size.height, "heights");
            let total = first.size.width + second.size.width;
            let expected = total * 0.62;
            let diff = (first.size.width - expected).abs();
            assert!(
                diff <= px(2.),
                "62/38 split: first={first:?} second={second:?}"
            );
            let gap = match place {
                Beside::Right => second.left() - first.right(),
                Beside::Left => first.left() - second.right(),
                Beside::Below => unreachable!(),
            };
            assert_eq!(gap, GUTTER, "gutter between panes");
        }
        Beside::Below => {
            assert_eq!(second.size.width, first.size.width, "widths");
            let total = first.size.height + second.size.height;
            let expected = total * 0.68;
            let diff = (first.size.height - expected).abs();
            assert!(
                diff <= px(2.),
                "68/32 split: first={first:?} second={second:?}"
            );
            assert_eq!(
                second.top() - first.bottom(),
                GUTTER,
                "gutter between panes"
            );
        }
    }
}

#[gpui_kit::test]
fn with_nothing_set_the_second_pane_is_to_the_right_of_the_first(cx: &mut TestAppContext) {
    let (shown, mut cx) = open_frame(WINDOW, vec![plain("first"), plain("second")], cx);

    assert_eq!(beside(&shown.frame, &mut cx), Beside::Right);
    let (first, second) = (pane("first", &mut cx), pane("second", &mut cx));
    assert_lies(Beside::Right, first, second);
    assert_split(Beside::Right, first, second);
    assert_eq!(first.left(), FULL_WIDTH + GUTTER);
    assert_eq!(second.right(), WINDOW.width - GUTTER);
    assert_eq!(first.top(), TOP_INSET);
    assert_eq!(first.bottom(), WINDOW.height - GUTTER);
    if cfg!(target_os = "macos") {
        let bar = drawn("frame-titlebar", &mut cx).expect("titlebar strip drawn on macOS");
        assert_eq!(bar.top(), px(0.));
        assert_eq!(bar.size.height, TITLEBAR_HEIGHT);
        assert_eq!(bar.size.width, WINDOW.width);
    } else {
        assert_eq!(drawn("frame-titlebar", &mut cx), None);
    }
}

#[gpui_kit::test]
fn the_key_of_move_panes_takes_the_second_pane_below_then_to_the_left_then_to_the_right_and_to_no_other_place(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = open_frame(WINDOW, vec![plain("first"), plain("second")], cx);
    let key = pressed::<MovePanes>();

    // Six presses, two rounds of the three places. The match below has one arm per place and no
    // other, so a fourth place would not compile here.
    let order = [
        Beside::Below,
        Beside::Left,
        Beside::Right,
        Beside::Below,
        Beside::Left,
        Beside::Right,
    ];
    for expected in order {
        cx.simulate_keystrokes(&key);
        let now = beside(&shown.frame, &mut cx);
        assert_eq!(now, expected, "after the key {key}");
        let (first, second) = (pane("first", &mut cx), pane("second", &mut cx));
        assert_lies(
            match now {
                Beside::Right => Beside::Right,
                Beside::Below => Beside::Below,
                Beside::Left => Beside::Left,
            },
            first,
            second,
        );
        assert_split(now, first, second);
    }
}

#[gpui_kit::test]
fn the_place_of_the_panes_lasts_through_a_resize_of_the_window(cx: &mut TestAppContext) {
    let (shown, mut cx) = open_frame(WINDOW, vec![plain("first"), plain("second")], cx);
    let key = pressed::<MovePanes>();

    for place in [Beside::Below, Beside::Left] {
        cx.simulate_keystrokes(&key);
        for resized in [size(px(1000.), px(600.)), size(px(1500.), px(900.))] {
            cx.simulate_resize(resized);
            assert_eq!(beside(&shown.frame, &mut cx), place, "after {resized:?}");
            let (first, second) = (pane("first", &mut cx), pane("second", &mut cx));
            assert_lies(place, first, second);
            assert_split(place, first, second);
        }
    }
}

#[gpui_kit::test]
fn the_panes_after_the_first_share_their_half_stacked_beside_it_and_in_a_row_below_it(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = open_frame(
        WINDOW,
        vec![plain("first"), plain("second"), plain("third")],
        cx,
    );
    let key = pressed::<MovePanes>();
    let panes =
        |cx: &mut VisualTestContext| (pane("first", cx), pane("second", cx), pane("third", cx));

    let (first, second, third) = panes(&mut cx);
    assert_lies(Beside::Right, first, second);
    assert!(third.left() >= first.right());
    assert_eq!(third.left(), second.left());
    assert!(third.top() >= second.bottom());
    assert_eq!(second.size.height, third.size.height);
    assert_eq!(third.bottom(), first.bottom());

    cx.simulate_keystrokes(&key);
    assert_eq!(beside(&shown.frame, &mut cx), Beside::Below);
    let (first, second, third) = panes(&mut cx);
    assert_lies(Beside::Below, first, second);
    assert!(third.top() >= first.bottom());
    assert_eq!(third.top(), second.top());
    assert!(third.left() >= second.right());
    assert_eq!(second.size.width, third.size.width);
    assert_eq!(third.right(), first.right());

    cx.simulate_keystrokes(&key);
    assert_eq!(beside(&shown.frame, &mut cx), Beside::Left);
    let (first, second, third) = panes(&mut cx);
    assert_lies(Beside::Left, first, second);
    assert!(third.right() <= first.left());
    assert_eq!(third.left(), second.left());
    assert!(third.top() >= second.bottom());
}

#[gpui_kit::test]
fn a_window_with_one_pane_gives_it_the_room_and_the_key_of_move_panes_changes_nothing_in_it(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = open_frame(WINDOW, vec![plain("only")], cx);
    let key = pressed::<MovePanes>();

    for _ in 0..3 {
        let only = pane("only", &mut cx);
        assert_eq!(only.left(), FULL_WIDTH + GUTTER);
        assert_eq!(only.right(), WINDOW.width - GUTTER);
        assert_eq!(only.top(), TOP_INSET);
        assert_eq!(only.bottom(), WINDOW.height - GUTTER);
        cx.simulate_keystrokes(&key);
    }
    assert_eq!(beside(&shown.frame, &mut cx), Beside::Right);
}

#[gpui_kit::test]
fn the_focused_pane_draws_the_active_focus_rule_and_moves_it_when_focus_switches(
    cx: &mut TestAppContext,
) {
    let (_shown, mut cx) = open_frame(WINDOW, vec![plain("first"), plain("second")], cx);

    assert!(
        drawn("frame-pane-first-focus", &mut cx).is_some(),
        "first pane starts focused"
    );
    assert_eq!(drawn("frame-pane-second-focus", &mut cx), None);

    cx.simulate_keystrokes(&pressed::<FocusNextPane>());
    assert_eq!(drawn("frame-pane-first-focus", &mut cx), None);
    assert!(
        drawn("frame-pane-second-focus", &mut cx).is_some(),
        "focus indicator follows next pane"
    );

    let first = pane("first", &mut cx);
    cx.simulate_click(first.center(), Modifiers::none());
    assert!(
        drawn("frame-pane-first-focus", &mut cx).is_some(),
        "clicking back into the first pane moves the focus indicator immediately"
    );
    assert_eq!(drawn("frame-pane-second-focus", &mut cx), None);
}

#[gpui_kit::test]
fn dragging_the_split_handle_resizes_the_panes_and_double_click_resets_the_split(
    cx: &mut TestAppContext,
) {
    let (_shown, mut cx) = open_frame(WINDOW, vec![plain("first"), plain("second")], cx);
    let initial_first = pane("first", &mut cx);
    let handle = drawn("frame-split-handle", &mut cx).expect("split handle is drawn");

    // Drag left by 160px to shrink the first pane.
    let target = point(handle.center().x - px(160.), handle.center().y);
    cx.simulate_mouse_down(handle.center(), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());

    let resized_first = pane("first", &mut cx);
    let resized_second = pane("second", &mut cx);
    assert!(
        resized_first.size.width < initial_first.size.width - px(100.),
        "dragging left shrunk the first pane: before={initial_first:?} after={resized_first:?}"
    );
    assert_eq!(
        resized_second.left() - resized_first.right(),
        GUTTER,
        "gutter stays intact after resize"
    );

    // Double-click the handle to reset back to the default 62/38 split.
    let handle = drawn("frame-split-handle", &mut cx).expect("split handle is drawn");
    cx.simulate_click(handle.center(), Modifiers::none());
    cx.simulate_event(MouseDownEvent {
        position: handle.center(),
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: handle.center(),
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
    });
    let (first, second) = (pane("first", &mut cx), pane("second", &mut cx));
    assert_split(Beside::Right, first, second);
}
