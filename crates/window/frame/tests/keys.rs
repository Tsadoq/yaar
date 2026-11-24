use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{
    Action, App, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, TestAppContext, VisualTestContext, Window, actions, div,
};
use yaar_frame::{
    Asks, Control, FocusNextPane, FocusPreviousPane, KEY_CONTEXT, MovePanes, NewWorktree,
    OpenFolder, OpenSettings, Place, Quit, QuitAndStopAgents, SelectNextWorkspace,
    SelectPreviousWorkspace, SetBase, ShowFinishedTurn, ToggleSidebar, UnpinSidebar, bind,
    controls,
};

actions!(keys_test, [Mark]);

struct Probe {
    context: &'static str,
    inside: FocusHandle,
    outside: FocusHandle,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(
                div()
                    .key_context(self.context)
                    .track_focus(&self.inside)
                    .size_10(),
            )
            .child(div().track_focus(&self.outside).size_10())
    }
}

fn open_probe<'a>(
    context: &'static str,
    cx: &'a mut TestAppContext,
) -> (Entity<Probe>, &'a mut VisualTestContext) {
    let (probe, cx) = cx.add_window_view(|_, cx| Probe {
        context,
        inside: cx.focus_handle(),
        outside: cx.focus_handle(),
    });
    probe.update_in(cx, |probe, window, cx| probe.inside.focus(window, cx));
    (probe, cx)
}

type Ran = Rc<RefCell<Vec<&'static str>>>;

fn note<A: Action>(ran: &Ran, cx: &mut App) {
    let ran = ran.clone();
    cx.on_action(move |action: &A, _| ran.borrow_mut().push(action.name()));
}

fn bind_the_table(cx: &mut TestAppContext) -> Ran {
    let ran = Ran::default();
    cx.update(|cx| {
        bind(&controls(), cx);
        note::<OpenFolder>(&ran, cx);
        note::<SetBase>(&ran, cx);
        note::<OpenSettings>(&ran, cx);
        note::<NewWorktree>(&ran, cx);
        note::<ToggleSidebar>(&ran, cx);
        note::<UnpinSidebar>(&ran, cx);
        note::<SelectNextWorkspace>(&ran, cx);
        note::<SelectPreviousWorkspace>(&ran, cx);
        note::<FocusNextPane>(&ran, cx);
        note::<FocusPreviousPane>(&ran, cx);
        note::<MovePanes>(&ran, cx);
        note::<ShowFinishedTurn>(&ran, cx);
        note::<Quit>(&ran, cx);
        note::<QuitAndStopAgents>(&ran, cx);
    });
    ran
}

/// The keys a user presses, written out here only: other tests press what the table has, so a key
/// changed by mistake would pass them all.
fn chosen() -> [(&'static str, &'static str, &'static str); 14] {
    [
        ("cmd-o", "ctrl-o", OpenFolder.name()),
        ("cmd-shift-b", "ctrl-shift-b", SetBase.name()),
        ("cmd-,", "ctrl-,", OpenSettings.name()),
        ("cmd-shift-n", "ctrl-shift-n", NewWorktree.name()),
        ("cmd-b", "ctrl-b", ToggleSidebar.name()),
        ("cmd-alt-b", "ctrl-alt-b", UnpinSidebar.name()),
        ("cmd-alt-down", "ctrl-alt-down", SelectNextWorkspace.name()),
        ("cmd-alt-up", "ctrl-alt-up", SelectPreviousWorkspace.name()),
        ("ctrl-tab", "ctrl-tab", FocusNextPane.name()),
        ("ctrl-shift-tab", "ctrl-shift-tab", FocusPreviousPane.name()),
        ("cmd-alt-p", "ctrl-alt-p", MovePanes.name()),
        ("cmd-alt-n", "ctrl-alt-n", ShowFinishedTurn.name()),
        ("cmd-q", "ctrl-q", Quit.name()),
        ("cmd-alt-q", "ctrl-alt-q", QuitAndStopAgents.name()),
    ]
}

#[gpui_kit::test]
fn every_key_of_the_table_runs_its_action_and_no_other(cx: &mut TestAppContext) {
    let ran = bind_the_table(cx);
    let (_probe, cx) = open_probe(KEY_CONTEXT, cx);

    for control in controls() {
        cx.simulate_keystrokes(&control.key());
        assert_eq!(
            ran.take(),
            [(control.action)().name()],
            "the key {} of {}",
            control.key(),
            control.title
        );
    }
}

#[gpui_kit::test]
fn each_action_of_the_frame_runs_on_the_key_chosen_for_it_on_the_system_the_app_is_built_for(
    cx: &mut TestAppContext,
) {
    let ran = bind_the_table(cx);
    let (_probe, cx) = open_probe(KEY_CONTEXT, cx);

    for (on_a_mac, elsewhere, action) in chosen() {
        let (of_this_system, of_the_other) = if cfg!(target_os = "macos") {
            (on_a_mac, elsewhere)
        } else {
            (elsewhere, on_a_mac)
        };
        cx.simulate_keystrokes(of_this_system);
        assert_eq!(ran.take(), [action], "the key {of_this_system}");
        if of_the_other != of_this_system {
            cx.simulate_keystrokes(of_the_other);
            assert_eq!(ran.take(), Vec::<&str>::new(), "the key {of_the_other}");
        }
    }
    assert_eq!(controls().len(), chosen().len());
}

#[test]
fn every_key_of_the_frame_works_anywhere_but_the_two_that_move_the_focus_between_panes() {
    // Off macOS a key placed `Anywhere` and a kept key are both Ctrl keys bound in the Frame's
    // context, so no pressed key tells them apart: the place written in each row is read instead.
    let kept = [FocusNextPane.name(), FocusPreviousPane.name()];
    for control in controls() {
        let expected = if kept.contains(&(control.action)().name()) {
            Place::Kept
        } else {
            Place::Anywhere
        };
        assert_eq!(control.place, expected, "{}", control.title);
    }
}

#[gpui_kit::test]
fn outside_the_context_of_the_frame_the_keys_of_the_table_work_on_macos_and_on_no_other_system(
    cx: &mut TestAppContext,
) {
    let ran = bind_the_table(cx);
    let (probe, cx) = open_probe(KEY_CONTEXT, cx);
    probe.update_in(cx, |probe, window, cx| probe.outside.focus(window, cx));

    // On macOS the `Anywhere` keys are Cmd keys and work wherever the focus is; elsewhere they are
    // Ctrl keys, which a terminal program waits for, so they work only in the Frame's context.
    let work_here = cfg!(target_os = "macos");
    for control in controls() {
        cx.simulate_keystrokes(&control.key());
        let expected = if work_here && control.place == Place::Anywhere {
            vec![(control.action)().name()]
        } else {
            Vec::new()
        };
        assert_eq!(
            ran.take(),
            expected,
            "the key {} of {}",
            control.key(),
            control.title
        );
    }
}

#[gpui_kit::test]
fn a_key_bound_in_a_context_works_only_where_the_context_is(cx: &mut TestAppContext) {
    let ran = Ran::default();
    cx.update(|cx| {
        bind(
            &[Control {
                title: "Mark",
                action: || Box::new(Mark),
                keystroke: "j",
                place: Place::In("Probe"),
                sends: None,
                asks: Asks::Nothing,
            }],
            cx,
        );
        note::<Mark>(&ran, cx);
    });
    let (probe, cx) = open_probe("Probe", cx);

    probe.update_in(cx, |probe, window, cx| probe.outside.focus(window, cx));
    cx.simulate_keystrokes("j");
    assert_eq!(ran.take(), Vec::<&str>::new(), "outside the context");

    probe.update_in(cx, |probe, window, cx| probe.inside.focus(window, cx));
    cx.simulate_keystrokes("j");
    assert_eq!(ran.take(), [Mark.name()], "inside the context");
}

#[gpui_kit::test]
#[should_panic(expected = "the key \"\" of \"Mark\" is not a key")]
fn a_control_with_an_empty_keystroke_is_refused_naming_the_control(cx: &mut TestAppContext) {
    cx.update(|cx| {
        bind(
            &[Control {
                title: "Mark",
                action: || Box::new(Mark),
                keystroke: "",
                place: Place::Anywhere,
                sends: None,
                asks: Asks::Nothing,
            }],
            cx,
        );
    });
}
