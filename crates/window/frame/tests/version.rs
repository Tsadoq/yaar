//! Component test: a Window that meets a running Keeper of an older version says so.
//! Older means a lower protocol, or the same protocol and a lower version of the app.

mod common;

use gpui_kit::{
    AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, Pixels, Point,
    Render, Size, Styled, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions,
    div, px,
};
use yaar_frame::{BuildPane, Frame, KeeperLink, LinkStatus, Pane};
use yaar_link::{Hello, Message, PROTOCOL_VERSION};

use common::{KeeperEnd, Shown, drawn, open_frame_with_keeper};

const WINDOW: Size<Pixels> = Size {
    width: px(1280.),
    height: px(800.),
};

/// The version of the app this Window is: the one every crate of the workspace has.
const OWN_VERSION: &str = env!("CARGO_PKG_VERSION");

struct Blank {
    focus: FocusHandle,
}

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

fn blank() -> BuildPane {
    Box::new(|_, _, cx| {
        let focus = cx.focus_handle();
        let view = cx.new(|_| Blank {
            focus: focus.clone(),
        });
        Pane {
            name: "blank",
            view: view.into(),
            focus,
        }
    })
}

fn hello(protocol: u16, app_version: &str) -> Hello {
    Hello {
        protocol,
        pid: 1,
        app_version: app_version.to_owned(),
    }
}

/// A composed Frame whose Keeper says this hello.
fn meeting(keeper: Hello, cx: &mut TestAppContext) -> (Shown, VisualTestContext) {
    let (shown, cx) = open_frame_with_keeper(keeper, WINDOW, vec![blank()], cx);
    cx.run_until_parked();
    (shown, cx)
}

fn note(shown: &Shown, cx: &mut VisualTestContext) -> Option<String> {
    shown.frame.read_with(cx, |frame, cx| frame.keeper_note(cx))
}

fn status(shown: &Shown, cx: &mut VisualTestContext) -> LinkStatus {
    shown
        .models
        .link
        .read_with(cx, |link, _| link.status().clone())
}

/// The note is drawn exactly where it is given: it is in the window's frame with the
/// sentence of the model.
fn assert_no_note(shown: &Shown, cx: &mut VisualTestContext) {
    assert_eq!(note(shown, cx), None);
    assert_eq!(drawn("frame-keeper-note", cx), None, "no note is drawn");
}

#[gpui_kit::test]
fn a_keeper_of_a_lower_protocol_is_said_to_be_older_and_nothing_is_asked_of_it(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = meeting(hello(PROTOCOL_VERSION - 1, "0.0.1"), cx);

    assert!(matches!(
        status(&shown, &mut cx),
        LinkStatus::OtherVersion(_)
    ));
    let sentence = note(&shown, &mut cx).expect("a note about an older Keeper");
    assert!(
        sentence.contains("0.0.1"),
        "it names the version: {sentence}"
    );
    assert!(
        sentence.contains("older"),
        "it says it is older: {sentence}"
    );
    assert!(
        sentence.contains("go on"),
        "it says the Keeper and what runs in it go on: {sentence}"
    );
    assert!(
        sentence.contains("nothing"),
        "it says the window can ask it nothing: {sentence}"
    );
    assert!(
        drawn("frame-keeper-note", &mut cx).is_some(),
        "the note is drawn"
    );

    // Nothing is asked: the connection goes as the link lets any go, and what the Keeper's
    // end reads up to its end is no frame.
    let Shown { models, keeper, .. } = shown;
    models.link.update(&mut cx, |link, cx| {
        link.attached(Err("gone".to_owned()), cx)
    });
    assert_eq!(keeper.rest(&mut cx), Vec::<Message>::new());
}

#[gpui_kit::test]
fn a_keeper_of_the_same_protocol_and_a_lower_version_is_said_to_be_older_and_is_asked_as_usual(
    cx: &mut TestAppContext,
) {
    let (mut shown, mut cx) = meeting(hello(PROTOCOL_VERSION, "0.0.1"), cx);

    assert!(matches!(status(&shown, &mut cx), LinkStatus::Connected(_)));
    let sentence = note(&shown, &mut cx).expect("a note about an older Keeper");
    assert!(
        sentence.contains("0.0.1"),
        "it names the version: {sentence}"
    );
    assert!(
        sentence.contains("older"),
        "it says it is older: {sentence}"
    );
    assert!(
        !sentence.contains("nothing"),
        "this window works with it, and says no such thing: {sentence}"
    );
    assert!(
        drawn("frame-keeper-note", &mut cx).is_some(),
        "the note is drawn"
    );

    // The list is asked for as usual.
    shown.keeper.list(&[], &mut cx);
}

#[gpui_kit::test]
fn a_keeper_of_a_newer_version_or_of_this_one_is_no_reason_for_a_note(cx: &mut TestAppContext) {
    // The versions of a Keeper that is not older, each in a window of its own.
    let keepers = [
        ("a newer version", hello(PROTOCOL_VERSION, "9.9.9")),
        ("this version", hello(PROTOCOL_VERSION, OWN_VERSION)),
        ("a newer protocol", hello(PROTOCOL_VERSION + 1, "0.0.1")),
        (
            "a version that cannot be compared",
            hello(PROTOCOL_VERSION, "nightly"),
        ),
    ];
    for (which, keeper) in keepers {
        let (shown, mut cx) = meeting(keeper, cx);
        assert_no_note(&shown, &mut cx);
        // The window still says how it stands, whatever the note.
        assert!(
            !shown
                .frame
                .read_with(&cx, |frame, cx| frame.status_line(cx))
                .is_empty(),
            "{which}"
        );
    }
}

#[gpui_kit::test]
fn the_note_goes_with_the_connection_that_ends_or_changes(cx: &mut TestAppContext) {
    let (shown, mut cx) = meeting(hello(PROTOCOL_VERSION, "0.0.1"), cx);
    assert!(note(&shown, &mut cx).is_some());

    // Another Keeper, of this version: the note of the first is not kept.
    KeeperEnd::attach_saying(
        hello(PROTOCOL_VERSION, OWN_VERSION),
        &shown.models.link,
        &mut cx,
    );
    cx.run_until_parked();
    assert_no_note(&shown, &mut cx);

    // None reached: no Keeper, no note.
    shown.models.link.update(&mut cx, |link, cx| {
        link.attached(Err("none".to_owned()), cx)
    });
    cx.run_until_parked();
    assert_no_note(&shown, &mut cx);
}

#[gpui_kit::test]
fn a_frame_for_the_link_alone_says_it_too_under_the_status_line(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let link = cx.new(|_| KeeperLink::new());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: WINDOW,
        })),
        ..Default::default()
    };
    let (window, frame) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, {
                let link = link.clone();
                move |_, cx| cx.new(|cx| Frame::new(link, cx))
            })
        })
        .expect("open a test window");
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    assert_eq!(
        frame.read_with(&cx, |frame, cx| frame.keeper_note(cx)),
        None
    );
    assert_eq!(drawn("frame-keeper-note", &mut cx), None);

    let _keeper = KeeperEnd::attach_saying(hello(PROTOCOL_VERSION - 1, "0.0.1"), &link, &mut cx);
    cx.run_until_parked();
    let sentence = frame
        .read_with(&cx, |frame, cx| frame.keeper_note(cx))
        .expect("a note about an older Keeper");
    assert!(sentence.contains("0.0.1"));
    assert!(drawn("frame-keeper-note", &mut cx).is_some());
}
