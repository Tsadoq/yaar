//! The Keeper's end of the Window's connection, as a test plays it: one end of a socket
//! pair, spoken with `yaar-link` and nothing else. The Window's end is handed to the
//! [`KeeperLink`] the way a real connection is, so a test fills the models with what a
//! Keeper would send and reads what the Window really asks.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::cell::RefCell;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gpui_kit::{
    Action, AppContext, Bounds, Entity, Pixels, Point, Size, Task, TestAppContext,
    VisualTestContext, WindowBounds, WindowOptions,
};
use yaar_frame::keeper::Attached;
use yaar_frame::{BuildPane, Frame, KeeperLink, Models, Sidebar, bind, controls};
use yaar_link::{
    Fact, Head, Hello, Message, PROTOCOL_VERSION, Refusal, Reply, Repository, RepositoryId,
    Request, RequestId, TerminalId, Workspace, WorkspaceId, read_message, write_message,
};

/// How long a test waits for something that has to happen. Reaching it fails the test.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// How long a wait rests between two looks.
const PAUSE: Duration = Duration::from_millis(1);

/// The id of a workspace, made of one digit.
pub fn id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

/// A worktree with this branch checked out.
pub fn workspace(digit: char, branch: &str) -> Workspace {
    Workspace {
        id: id(digit),
        path: PathBuf::from(format!("/gits/{branch}")),
        head: Head::Branch(branch.to_owned()),
    }
}

/// A repository with these worktrees, named `repository-<letter>`.
pub fn repository(letter: char, workspaces: Vec<Workspace>) -> Repository {
    Repository {
        id: RepositoryId::new(letter.to_string().repeat(32)),
        name: format!("repository-{letter}"),
        workspaces,
    }
}

/// The key the Frame's table of controls gives an action. A test presses the key the
/// table has, so that it follows the table when a key changes.
pub fn key_of<A: Action + Default>() -> &'static str {
    let name = A::default().name();
    controls()
        .into_iter()
        .find(|control| (control.action)().name() == name)
        .expect("the action has a row in the table")
        .keystroke
}

/// The key the Frame's table gives an action, as a user presses it on the system the app
/// is built for: `secondary` written out.
pub fn pressed<A: Action + Default>() -> String {
    let name = A::default().name();
    controls()
        .into_iter()
        .find(|control| (control.action)().name() == name)
        .expect("the action has a row in the table")
        .key()
}

/// The Keeper, played by the test.
pub struct KeeperEnd {
    stream: UnixStream,
    // What the Window sent, read by a thread of this end's own: the thread of the test
    // has to keep the app running while it waits.
    heard: Receiver<Message>,
}

impl KeeperEnd {
    /// Links `link` to a Keeper that speaks the Window's protocol.
    pub fn attach(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> Self {
        Self::attach_speaking(PROTOCOL_VERSION, link, cx)
    }

    /// Links `link` to a Keeper that speaks this version of the protocol.
    pub fn attach_speaking(
        protocol: u16,
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
    ) -> Self {
        Self::attach_saying(
            Hello {
                protocol,
                pid: 1,
                app_version: "9.9.9".to_owned(),
            },
            link,
            cx,
        )
    }

    /// Links `link` to a Keeper that says this hello: the protocol it speaks and the
    /// version of the app it belongs to.
    pub fn attach_saying(
        keeper: Hello,
        link: &Entity<KeeperLink>,
        cx: &mut TestAppContext,
    ) -> Self {
        // The connection is read by a real thread, which the test executor cannot see.
        cx.executor().allow_parking();
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        let mut reading = ours.try_clone().expect("a second handle to the socket");
        let (tell, heard) = channel();
        std::thread::spawn(move || {
            while let Ok(message) = read_message(&mut reading) {
                if tell.send(message).is_err() {
                    break;
                }
            }
        });
        link.update(cx, |link, cx| {
            link.attached(
                Ok(Attached {
                    stream: theirs,
                    keeper,
                    started: false,
                }),
                cx,
            )
        });
        Self {
            stream: ours,
            heard,
        }
    }

    /// Links `link` to a Keeper that never reads: the connection is open, and what the
    /// Window writes to it stays where it is. Gives the Keeper's end, which keeps the
    /// connection open for as long as it lives.
    pub fn attach_deaf(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> UnixStream {
        cx.executor().allow_parking();
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        let keeper = Hello {
            protocol: PROTOCOL_VERSION,
            pid: 1,
            app_version: "9.9.9".to_owned(),
        };
        link.update(cx, |link, cx| {
            link.attached(
                Ok(Attached {
                    stream: theirs,
                    keeper,
                    started: false,
                }),
                cx,
            )
        });
        ours
    }

    /// The next message of the Window, waiting for it with the app running. Fails when
    /// none comes, or when the Window closes the connection.
    fn message(&mut self, wanted: &str, cx: &mut TestAppContext) -> Message {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.heard.recv_timeout(PAUSE) {
                Ok(message) => return message,
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the Window closed the connection, and {wanted} was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(Instant::now() < deadline, "the Window sent no {wanted}");
        }
    }

    /// The next request of the Window. Fails when none comes, when the Window sends
    /// anything else, or when it closes the connection.
    pub fn request(&mut self, cx: &mut TestAppContext) -> (RequestId, Request) {
        match self.message("a request", cx) {
            Message::Request { id, request } => (id, request),
            other => panic!("the Window sent {other:?}, which is no request"),
        }
    }

    /// The next input the Window sends to a terminal: the terminal and the bytes of one
    /// frame. Fails when none comes, when the Window sends anything else, or when it
    /// closes the connection.
    pub fn input(&mut self, cx: &mut TestAppContext) -> (TerminalId, Vec<u8>) {
        match self.message("input", cx) {
            Message::Input { terminal, bytes } => (terminal, bytes),
            other => panic!("the Window sent {other:?}, which is no input"),
        }
    }

    /// Sends the Window the output of a terminal's program.
    pub fn output(&mut self, terminal: &TerminalId, bytes: &[u8]) {
        let message = Message::Output {
            terminal: terminal.clone(),
            bytes: bytes.to_vec(),
        };
        write_message(&mut self.stream, &message).expect("write output");
    }

    /// Sends the Window a message of any kind.
    pub fn send(&mut self, message: &Message) {
        write_message(&mut self.stream, message).expect("write a message");
    }

    /// The next request of the Window, which has to be this one; gives its id.
    pub fn expect(&mut self, wanted: Request, cx: &mut TestAppContext) -> RequestId {
        let (id, request) = self.request(cx);
        assert_eq!(request, wanted);
        id
    }

    /// Answers the question for the list of workspaces, which has to be the next request,
    /// with these repositories.
    pub fn list(&mut self, listed: &[Repository], cx: &mut TestAppContext) {
        let asked = self.expect(Request::ListWorkspaces, cx);
        self.reply(asked, Ok(Reply::Workspaces(listed.to_vec())));
    }

    /// Answers a request.
    pub fn reply(&mut self, to: RequestId, reply: Result<Reply, Refusal>) {
        write_message(&mut self.stream, &Message::Reply { to, reply }).expect("write a reply");
    }

    /// Tells the Window a fact.
    pub fn tell(&mut self, fact: Fact) {
        write_message(&mut self.stream, &Message::Fact(fact)).expect("write a fact");
    }

    /// Sends bytes as they are.
    pub fn send_raw(&mut self, bytes: &[u8]) {
        use std::io::Write;
        self.stream.write_all(bytes).expect("write bytes");
    }

    /// Closes the connection, as a Keeper that exits does.
    pub fn close(self) {
        self.stream
            .shutdown(Shutdown::Both)
            .expect("close the socket");
    }

    /// Everything the Window still sends, up to the moment it closes the connection.
    /// Fails when the Window keeps it open.
    pub fn rest(self, cx: &mut TestAppContext) -> Vec<Message> {
        let deadline = Instant::now() + DEADLINE;
        let mut rest = Vec::new();
        loop {
            cx.run_until_parked();
            match self.heard.recv_timeout(PAUSE) {
                Ok(message) => rest.push(message),
                Err(RecvTimeoutError::Disconnected) => return rest,
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(
                Instant::now() < deadline,
                "the Window did not close the connection"
            );
        }
    }
}

/// Keeps the app running until `holds` says yes. Fails at the deadline, naming `what`.
pub fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    holds: impl FnMut(&mut TestAppContext) -> bool,
) {
    wait_until_within(DEADLINE, cx, what, holds);
}

/// Keeps the app running until `holds` says yes, for at most `within`. Fails at the
/// deadline, naming `what`. For what the Window itself waits for before it acts.
pub fn wait_until_within(
    within: Duration,
    cx: &mut TestAppContext,
    what: &str,
    mut holds: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + within;
    loop {
        cx.run_until_parked();
        if holds(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "waited in vain for {what}");
        std::thread::park_timeout(PAUSE);
    }
}

/// Waits for what a task resolves to.
pub fn answer<T: 'static>(task: Task<T>, cx: &mut TestAppContext) -> T {
    let slot = Rc::new(RefCell::new(None));
    cx.foreground_executor()
        .spawn({
            let slot = slot.clone();
            async move {
                let answer = task.await;
                *slot.borrow_mut() = Some(answer);
            }
        })
        .detach();
    wait_until(cx, "an answer", |_| slot.borrow().is_some());
    slot.take().expect("the answer")
}

/// A window of the app on the test platform, with the Keeper the test plays.
pub struct Shown {
    /// The root view of the window.
    pub frame: Entity<Frame>,
    /// Its sidebar.
    pub sidebar: Entity<Sidebar>,
    /// The shared models of the window.
    pub models: Models,
    /// The other end of the window's connection.
    pub keeper: KeeperEnd,
}

/// Opens a window of this size the way the Window program does, short of reaching a real
/// Keeper: the toolkit prepared, the keys of the Frame bound from its table, the Frame
/// with its sidebar and these panes as the root view. Then links it to a Keeper the test
/// plays, which is asked for the list of workspaces first.
pub fn open_frame(
    window_size: Size<Pixels>,
    panes: Vec<BuildPane>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    let keeper = Hello {
        protocol: PROTOCOL_VERSION,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    };
    open_frame_with_keeper(keeper, window_size, panes, cx)
}

/// [`open_frame`], with a Keeper that says this hello.
pub fn open_frame_with_keeper(
    keeper: Hello,
    window_size: Size<Pixels>,
    panes: Vec<BuildPane>,
    cx: &mut TestAppContext,
) -> (Shown, VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind(&controls(), cx);
    });
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: window_size,
        })),
        ..Default::default()
    };
    let (window, frame) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, {
                let models = models.clone();
                move |window, cx| cx.new(|cx| Frame::with_panes(models, panes, window, cx))
            })
        })
        .expect("open a test window");
    let mut cx = VisualTestContext::from_window(window, cx);
    // Whatever opening the window sets off is done before a Keeper is there to hear it,
    // so the first thing the Keeper hears is the question for the list.
    cx.run_until_parked();
    let sidebar = frame.read_with(&cx, |frame, _| {
        frame
            .sidebar()
            .expect("a Frame with panes has a sidebar")
            .clone()
    });
    let keeper = KeeperEnd::attach_saying(keeper, &link, &mut cx);
    (
        Shown {
            frame,
            sidebar,
            models,
            keeper,
        },
        cx,
    )
}

/// Where the element with this selector was drawn in the last frame, if it was.
pub fn drawn(selector: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    // The test context takes a text that lives as long as the program.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}
