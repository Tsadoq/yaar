//! Component test: where the Frame lays the panes beside the first, and the key that moves them.
//! The Frame knows no pane by name.

mod common;

use gpui_kit::{
    AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, Pixels, Render,
    Size, Styled, TestAppContext, VisualTestContext, Window, div, px, size,
};
use yaar_frame::{Beside, BuildPane, FULL_WIDTH, MovePanes, Pane};

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

fn assert_halves(first: Bounds<Pixels>, second: Bounds<Pixels>) {
    assert_eq!(
        second.size, first.size,
        "equal halves: {first:?} {second:?}"
    );
}

#[gpui_kit::test]
fn with_nothing_set_the_second_pane_is_to_the_right_of_the_first(cx: &mut TestAppContext) {
    let (shown, mut cx) = open_frame(WINDOW, vec![plain("first"), plain("second")], cx);

    assert_eq!(beside(&shown.frame, &mut cx), Beside::Right);
    let (first, second) = (pane("first", &mut cx), pane("second", &mut cx));
    assert_lies(Beside::Right, first, second);
    assert_halves(first, second);
    assert_eq!(first.left(), FULL_WIDTH);
    assert_eq!(second.right(), WINDOW.width);
    assert_eq!(first.top(), px(0.));
    assert_eq!(first.bottom(), WINDOW.height);
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
        assert_halves(first, second);
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
            assert_halves(first, second);
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
        assert_eq!(only.left(), FULL_WIDTH);
        assert_eq!(only.right(), WINDOW.width);
        assert_eq!(only.top(), px(0.));
        assert_eq!(only.bottom(), WINDOW.height);
        cx.simulate_keystrokes(&key);
    }
    assert_eq!(beside(&shown.frame, &mut cx), Beside::Right);
}
