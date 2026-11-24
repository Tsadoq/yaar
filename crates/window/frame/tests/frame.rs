//! Component test: the Frame on GPUI's test platform, with no window on any screen.

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use gpui_kit::{
    AppContext, Bounds, Point, TestAppContext, VisualTestContext, WindowBounds, WindowOptions, px,
    size,
};
use yaar_frame::keeper::Attached;
use yaar_frame::{Frame, KeeperLink, LinkStatus};
use yaar_link::{Hello, PROTOCOL_VERSION};

/// The connection lasts as long as the returned end does.
fn keeper(protocol: u16) -> (Attached, UnixStream) {
    let (stream, other_end) = UnixStream::pair().expect("socket pair");
    let attached = Attached {
        stream,
        keeper: Hello {
            protocol,
            pid: 1,
            app_version: "9.9.9".to_owned(),
        },
        started: false,
    };
    (attached, other_end)
}

#[gpui_kit::test]
fn the_empty_window_says_how_it_stands_with_the_keeper(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let link = cx.new(|_| KeeperLink::new());
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(800.), px(600.)),
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
    let draw = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    };

    draw(&mut cx);
    assert_eq!(
        link.read_with(&cx, |l, _| l.status().clone()),
        LinkStatus::Starting
    );
    assert_eq!(
        frame.read_with(&cx, |f, cx| f.status_line(cx)),
        "Starting the Keeper"
    );

    // The connection is read by a real thread, which the test executor cannot see.
    cx.executor().allow_parking();
    let (attached, other_end) = keeper(PROTOCOL_VERSION);
    link.update(&mut cx, |l, cx| l.attached(Ok(attached), cx));
    draw(&mut cx);
    assert_eq!(
        frame.read_with(&cx, |f, cx| f.status_line(cx)),
        "Keeper 9.9.9 is running"
    );

    drop(other_end);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !matches!(
        link.read_with(&cx, |l, _| l.status().clone()),
        LinkStatus::Lost(_)
    ) {
        assert!(
            Instant::now() < deadline,
            "the loss of the Keeper was not shown"
        );
        cx.run_until_parked();
        std::thread::park_timeout(Duration::from_millis(1));
    }
    draw(&mut cx);
    assert_eq!(
        frame.read_with(&cx, |f, cx| f.status_line(cx)),
        "The Keeper went away: the other side closed the connection"
    );

    let (attached, _other_end) = keeper(PROTOCOL_VERSION + 1);
    link.update(&mut cx, |l, cx| l.attached(Ok(attached), cx));
    draw(&mut cx);
    assert!(matches!(
        link.read_with(&cx, |l, _| l.status().clone()),
        LinkStatus::OtherVersion(_)
    ));
    assert_eq!(
        frame.read_with(&cx, |f, cx| f.status_line(cx)),
        "The Keeper that is running belongs to version 9.9.9 of the app"
    );

    link.update(&mut cx, |l, cx| {
        l.attached(Err("the Keeper program was not found".to_owned()), cx)
    });
    draw(&mut cx);
    assert_eq!(
        frame.read_with(&cx, |f, cx| f.status_line(cx)),
        "No Keeper: the Keeper program was not found"
    );
}
