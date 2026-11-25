//! Component test: a pane is added to the Window without a change to any pane. The third pane is
//! written here from `yaar_frame` and `gpui_kit` alone,
//! its key bound with the Frame's `bind`, its action routed through the shared models.

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

actions!(seam, [Ping]);

struct Third {
    focus: FocusHandle,
    notified: usize,
    on_screen: Option<PathBuf>,
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

/// The third pane's action, bound anywhere on a key no control of the Window has.
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

    let mut listed: Vec<&str> = yaar_window::panes().iter().map(|pane| pane.name).collect();
    listed.push("third");
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.panes()),
        listed
    );
    wait_until(
        &mut cx,
        "the welcome screen before a folder is opened",
        |cx| cx.debug_bounds("frame-welcome").is_some(),
    );

    let told = third.read_with(&cx, |third, _| third.notified);
    let folder = window.open_a_folder(&mut cx);
    wait_until(&mut cx, "the place of the third pane", |cx| {
        cx.debug_bounds("frame-pane-third").is_some()
    });
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

    window.click_into_the_review_pane(&mut cx);
    assert!(window.focus_is_in(yaar_review_pane::KEY_CONTEXT, &mut cx));
    window.ran.borrow_mut().clear();
    cx.simulate_keystrokes(&ping_key);
    assert_eq!(window.ran.take(), [Ping.name()], "what the key ran");
    assert_eq!(third.read_with(&cx, |third, _| third.pings), 1);

    // On Linux a key that works anywhere does nothing inside a terminal, so no terminal is open
    // yet: Next Pane puts the focus on the pane itself.
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
    let (mut window, mut cx) = open_composed(cx, Vec::new());
    assert_eq!(
        window.frame.read_with(&cx, |frame, _| frame.panes()),
        ["review", "terminal"]
    );
    window.open_a_folder(&mut cx);
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
    assert!(review.size.width > terminal.size.width);
}
