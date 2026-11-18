//! Component test: a pane is added to the Window without a change to any pane.
//!
//! The third pane is written here, from `yaar_frame` and `gpui_kit` alone. It joins the
//! Frame beside the unchanged list of the Window program, in a window whose Keeper is the
//! built program of a world of its own. Its key is bound with the Frame's own `bind`, and
//! its action reaches it through the routes the shared models hold.
//!
//! That the commits which add a pane change no file of another pane is read from the
//! history, not by this test.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Action, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, Render,
    Styled, TestAppContext, Window, actions, div,
};
use yaar_frame::{Asks, BuildPane, Control, Models, Pane, Place};

use common::{key_of, open_composed, wait_until};

actions!(
    seam,
    [
        /// The one action of the third pane.
        Ping
    ]
);

/// A pane that watches the list of workspaces and takes one routed action.
struct Third {
    focus: FocusHandle,
    /// How many times the list told it changed.
    notified: usize,
    /// The folder of the workspace the list had on screen when it last told.
    on_screen: Option<PathBuf>,
    /// How many times the action reached it.
    pings: usize,
}

impl Third {
    fn new(models: &Models, cx: &mut Context<Self>) -> Self {
        cx.observe(&models.workspaces, |third, list, cx| {
            third.notified += 1;
            let list = list.read(cx);
            third.on_screen = list
                .selected()
                .and_then(|selected| list.workspace(selected))
                .map(|workspace| workspace.path.clone());
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            notified: 0,
            on_screen: None,
            pings: 0,
        }
    }

    fn ping(&mut self, _: &Ping, _: &mut Window, _: &mut Context<Self>) {
        self.pings += 1;
    }
}

impl Render for Third {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}

/// The builder of the third pane, which hands the view it made to `kept`.
fn third(kept: Rc<RefCell<Option<Entity<Third>>>>) -> BuildPane {
    Box::new(move |models, _, cx: &mut App| {
        let view = cx.new(|cx| Third::new(models, cx));
        models.routes.route(&view, Third::ping);
        *kept.borrow_mut() = Some(view.clone());
        let focus = view.read(cx).focus.clone();
        Pane {
            name: "third",
            view: view.into(),
            focus,
        }
    })
}

/// The key of the third pane's action: anywhere, on a key no control of the Window has.
fn ping() -> Control {
    Control {
        title: "Ping",
        action: || Box::new(Ping),
        keystroke: "secondary-alt-y",
        place: Place::Anywhere,
        sends: None,
        asks: Asks::Nothing,
    }
}

#[gpui_kit::test]
fn a_third_pane_joins_the_frame_watches_the_shared_model_and_takes_a_routed_action_with_no_pane_changed(
    cx: &mut TestAppContext,
) {
    let kept: Rc<RefCell<Option<Entity<Third>>>> = Rc::default();
    let (mut window, mut cx) = open_composed(cx, vec![third(kept.clone())]);
    let third = kept.take().expect("the Frame built the third pane");
    cx.update(|_, cx| yaar_frame::bind(&[ping()], cx));
    let ping_key = ping().key();

    // It joins the Frame: the Window's panes, then it.
    let mut listed: Vec<&str> = yaar_window::panes().iter().map(|pane| pane.name).collect();
    listed.push("third");
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.panes()),
        listed
    );
    wait_until(&mut cx, "the place of the third pane", |cx| {
        cx.debug_bounds("frame-pane-third").is_some()
    });

    // It watches the shared model: a folder opened by a user is the workspace on screen,
    // and the pane has been told.
    let told = third.read_with(&cx, |third, _| third.notified);
    let folder = window.open_a_folder(&mut cx);
    third.read_with(&cx, |third, _| {
        assert!(
            third.notified > told,
            "the list told the pane of the folder"
        );
        assert_eq!(third.on_screen.as_ref(), Some(&folder));
    });
    let selected = window.models.workspaces.read_with(&cx, |list, _| {
        list.selected()
            .and_then(|selected| list.workspace(selected))
            .map(|workspace| workspace.path.clone())
    });
    assert_eq!(
        selected.as_ref(),
        Some(&folder),
        "the shared model's own read"
    );

    // It takes a routed action from where the focus is in the review pane, and nothing
    // else runs.
    window.click_into_the_review_pane(&mut cx);
    assert!(window.focus_is_in(yaar_review_pane::KEY_CONTEXT, &mut cx));
    window.ran.borrow_mut().clear();
    cx.simulate_keystrokes(&ping_key);
    assert_eq!(window.ran.take(), [Ping.name()], "what the key ran");
    assert_eq!(third.read_with(&cx, |third, _| third.pings), 1);

    // And from the terminal pane while it holds no terminal: the key of Next Pane puts the
    // focus on the pane itself, outside any program. On Linux a key that works anywhere
    // does nothing inside a terminal, which is why no terminal is open yet.
    cx.simulate_keystrokes(&key_of("Next Pane"));
    window.ran.borrow_mut().clear();
    assert!(window.focus_is_in(yaar_terminal_pane::KEY_CONTEXT, &mut cx));
    assert!(!window.focus_is_in(yaar_frame::PROGRAM_CONTEXT, &mut cx));
    cx.simulate_keystrokes(&ping_key);
    assert_eq!(window.ran.take(), [Ping.name()], "what the key ran");
    assert_eq!(third.read_with(&cx, |third, _| third.pings), 2);
    assert!(
        window
            .terminals
            .read_with(&cx, |pane, _| pane.terminals().is_empty()),
        "the action opened no terminal"
    );

    // With no pane changed: the review pane and the terminal pane answer their own keys in
    // the same window, each read through its model.
    window.click_into_the_review_pane(&mut cx);
    let layout = window.review.read_with(&cx, |pane, _| pane.layout());
    cx.simulate_keystrokes("s");
    assert_ne!(
        window.review.read_with(&cx, |pane, _| pane.layout()),
        layout,
        "the review pane switched its diff"
    );
    cx.simulate_keystrokes(&key_of("New Terminal"));
    wait_until(&mut cx, "the terminal pane to hold a terminal", |cx| {
        window
            .terminals
            .read_with(cx, |pane, _| pane.terminals().len() == 1)
    });
    assert_eq!(third.read_with(&cx, |third, _| third.pings), 2);
}

#[gpui_kit::test]
fn the_window_lists_the_review_pane_then_the_terminal_pane_and_with_nothing_set_the_terminal_lies_to_the_right(
    cx: &mut TestAppContext,
) {
    let (window, mut cx) = open_composed(cx, Vec::new());
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.panes()),
        ["review", "terminal"]
    );
    let mut review = None;
    let mut terminal = None;
    wait_until(&mut cx, "both panes to be drawn", |cx| {
        review = cx.debug_bounds("frame-pane-review");
        terminal = cx.debug_bounds("frame-pane-terminal");
        review.is_some() && terminal.is_some()
    });
    let (review, terminal) = (review.expect("drawn"), terminal.expect("drawn"));
    assert!(terminal.left() >= review.right());
    assert_eq!(terminal.top(), review.top());
}
