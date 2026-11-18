//! Component test: the keys of a table of controls, bound the way the Window program
//! binds them and pressed on GPUI's test platform.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    Action, App, Context, FocusHandle, InteractiveElement, IntoElement, ParentElement, Render,
    Styled, TestAppContext, VisualTestContext, Window, actions, div,
};
use yaar_frame::{
    Asks, Control, OpenFolder, SelectNextWorkspace, SelectPreviousWorkspace, SetBase,
    ToggleSidebar, UnpinSidebar, bind, controls,
};

actions!(
    keys_test,
    [
        /// An action of the test's own, bound in a context.
        Mark
    ]
);

/// A window with two places the focus can be, one of them inside the context `Probe`.
struct Probe {
    inside: FocusHandle,
    outside: FocusHandle,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(
                div()
                    .key_context("Probe")
                    .track_focus(&self.inside)
                    .size_10(),
            )
            .child(div().track_focus(&self.outside).size_10())
    }
}

fn open_probe(cx: &mut TestAppContext) -> (gpui_kit::Entity<Probe>, &mut VisualTestContext) {
    cx.add_window_view(|_, cx| Probe {
        inside: cx.focus_handle(),
        outside: cx.focus_handle(),
    })
}

/// Notes the name of every action of type `A` that nothing in the window handled.
fn note<A: Action>(ran: &Rc<RefCell<Vec<&'static str>>>, cx: &mut App) {
    let ran = ran.clone();
    cx.on_action(move |action: &A, _| ran.borrow_mut().push(action.name()));
}

#[gpui_kit::test]
fn every_key_of_the_table_runs_its_action_and_no_other(cx: &mut TestAppContext) {
    let ran = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        bind(&controls(), cx);
        note::<OpenFolder>(&ran, cx);
        note::<SetBase>(&ran, cx);
        note::<ToggleSidebar>(&ran, cx);
        note::<UnpinSidebar>(&ran, cx);
        note::<SelectNextWorkspace>(&ran, cx);
        note::<SelectPreviousWorkspace>(&ran, cx);
    });
    let (_probe, cx) = open_probe(cx);

    // An action of the table that the test does not note fails here, with nothing run.
    for control in controls() {
        cx.simulate_keystrokes(control.keystroke);
        assert_eq!(
            ran.take(),
            [(control.action)().name()],
            "the key {} of {}",
            control.keystroke,
            control.title
        );
    }
}

#[gpui_kit::test]
fn a_key_bound_in_a_context_works_only_where_the_context_is(cx: &mut TestAppContext) {
    let ran = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        bind(
            &[Control {
                title: "Mark",
                action: || Box::new(Mark),
                keystroke: "j",
                context: Some("Probe"),
                sends: None,
                asks: Asks::Nothing,
            }],
            cx,
        );
        note::<Mark>(&ran, cx);
    });
    let (probe, cx) = open_probe(cx);

    probe.update_in(cx, |probe, window, cx| probe.outside.focus(window, cx));
    cx.simulate_keystrokes("j");
    assert_eq!(ran.take(), Vec::<&str>::new(), "outside the context");

    probe.update_in(cx, |probe, window, cx| probe.inside.focus(window, cx));
    cx.simulate_keystrokes("j");
    assert_eq!(ran.take(), [Mark.name()], "inside the context");
}
