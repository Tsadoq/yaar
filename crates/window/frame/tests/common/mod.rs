//! The Keeper's end of the Window's connection, as a test plays it: one end of a socket
//! pair, spoken with `yaar-link` and nothing else. The Window's end is handed to the
//! [`KeeperLink`] the way a real connection is, so a test fills the models with what a
//! Keeper would send and reads what the Window really asks.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::cell::RefCell;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gpui_kit::{Entity, Task, TestAppContext};
use yaar_frame::KeeperLink;
use yaar_frame::keeper::Attached;
use yaar_link::{
    Fact, Hello, Message, PROTOCOL_VERSION, Refusal, Reply, Request, RequestId, read_message,
    write_message,
};

/// How long a test waits for something that has to happen. Reaching it fails the test.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// How long a wait rests between two looks.
const PAUSE: Duration = Duration::from_millis(1);

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
        let keeper = Hello {
            protocol,
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
        Self {
            stream: ours,
            heard,
        }
    }

    /// The next request of the Window. Fails when none comes, when the Window sends
    /// anything else, or when it closes the connection.
    pub fn request(&mut self, cx: &mut TestAppContext) -> (RequestId, Request) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            cx.run_until_parked();
            match self.heard.recv_timeout(PAUSE) {
                Ok(Message::Request { id, request }) => return (id, request),
                Ok(other) => panic!("the Window sent {other:?}, which is no request"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the Window closed the connection, and a request was expected")
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            assert!(Instant::now() < deadline, "the Window asked nothing");
        }
    }

    /// The next request of the Window, which has to be this one; gives its id.
    pub fn expect(&mut self, wanted: Request, cx: &mut TestAppContext) -> RequestId {
        let (id, request) = self.request(cx);
        assert_eq!(request, wanted);
        id
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
    mut holds: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + DEADLINE;
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
