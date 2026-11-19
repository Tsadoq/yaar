//! What the tests of the pane share: a window that holds the pane alone, and the Keeper's
//! end of the Window's connection, played by the test on one end of a socket pair with
//! `yaar-link` and nothing else. The models are filled by what a Keeper would send, and
//! the test reads what the Window really asks.

// Each test program takes what it needs of this.
#![allow(dead_code)]

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyView, AppContext, Bounds, Context, Entity, FocusHandle, IntoElement, ParentElement, Pixels,
    Point, Render, Size, Styled, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use yaar_frame::keeper::Attached;
use yaar_frame::{KeeperLink, Models, bind};
use yaar_link::{
    Base, BaseSource, ChangedFile, Changes, Commit, Comparison, Content, Fact, FileSides,
    FileStatus, Head, Hello, Message, PROTOCOL_VERSION, Refusal, Reply, Repository, RepositoryId,
    Request, RequestId, Review, ReviewId, Workspace, WorkspaceId, WorkspaceReviews, read_message,
    write_message,
};
use yaar_review_pane::{PaneRow, ReviewPane, Unshown};

/// How long a test waits for something that has to happen. Reaching it fails the test.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// How long a wait rests between two looks.
const PAUSE: Duration = Duration::from_millis(1);

/// The size of the window the pane is tested in: small, so that a diff of some length
/// does not fit and the pane has to scroll.
pub const WINDOW: Size<Pixels> = size(px(900.), px(420.));

/// The full id of a commit, as the Link carries it.
pub const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// What the Keeper end answers to `GetReviews` by itself, so that the reviews the pane
/// asks for at times a test does not choose never reach the test as its next request.
#[derive(Default)]
struct Reviews {
    /// What a test set, by workspace; a workspace with nothing set has one empty review
    /// of no agent, [`no_agent_review`].
    set: Mutex<HashMap<WorkspaceId, WorkspaceReviews>>,
    /// How many `GetReviews` were heard, answered or held.
    heard: AtomicUsize,
    /// Whether the answers wait: the requests are kept until [`KeeperEnd::release_reviews`].
    holding: AtomicBool,
    /// The requests kept while `holding`.
    held: Mutex<Vec<(RequestId, WorkspaceId)>>,
}

impl Reviews {
    /// What a `GetReviews` of `workspace` is answered with now.
    fn answer(&self, workspace: &WorkspaceId) -> WorkspaceReviews {
        self.set
            .lock()
            .expect("the reviews are not poisoned")
            .get(workspace)
            .cloned()
            .unwrap_or_else(|| WorkspaceReviews {
                workspace: workspace.clone(),
                reviews: vec![no_agent_review()],
                viewed: Vec::new(),
            })
    }
}

/// Writes a message on the one stream of this end. A lock keeps the frames whole, since
/// the thread that answers the reviews and the test both write.
fn write(stream: &Mutex<UnixStream>, message: &Message) {
    let mut stream = stream.lock().expect("the stream is not poisoned");
    write_message(&mut *stream, message).expect("write a message");
}

/// The Keeper, played by the test.
pub struct KeeperEnd {
    stream: Arc<Mutex<UnixStream>>,
    // What the Window sent, read by a thread of this end's own: the thread of the test
    // has to keep the app running while it waits.
    heard: Receiver<Message>,
    reviews: Arc<Reviews>,
}

impl KeeperEnd {
    /// Links `link` to a Keeper that speaks the Window's protocol.
    pub fn attach(link: &Entity<KeeperLink>, cx: &mut TestAppContext) -> Self {
        // The connection is read by a real thread, which the test executor cannot see.
        cx.executor().allow_parking();
        let (ours, theirs) = UnixStream::pair().expect("a socket pair");
        let mut reading = ours.try_clone().expect("a second handle to the socket");
        let stream = Arc::new(Mutex::new(ours));
        let reviews = Arc::new(Reviews::default());
        let (tell, heard) = channel();
        {
            let (stream, reviews) = (stream.clone(), reviews.clone());
            std::thread::spawn(move || {
                while let Ok(message) = read_message(&mut reading) {
                    match message {
                        // The reviews are asked for at times the test does not choose:
                        // they are answered here, and the test never hears of them.
                        Message::Request {
                            id,
                            request: Request::GetReviews { workspace },
                        } => {
                            reviews.heard.fetch_add(1, Ordering::SeqCst);
                            let mut held = reviews.held.lock().expect("not poisoned");
                            if reviews.holding.load(Ordering::SeqCst) {
                                held.push((id, workspace));
                            } else {
                                drop(held);
                                let answer = reviews.answer(&workspace);
                                write(
                                    &stream,
                                    &Message::Reply {
                                        to: id,
                                        reply: Ok(Reply::Reviews(answer)),
                                    },
                                );
                            }
                        }
                        message => {
                            if tell.send(message).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
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
        Self {
            stream,
            heard,
            reviews,
        }
    }

    /// The next request of the Window that is not for the reviews, which this end answers
    /// itself. Fails when none comes, when the Window sends anything else, or when it
    /// closes the connection.
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
        write(&self.stream, &Message::Reply { to, reply });
    }

    /// Tells the Window a fact.
    pub fn tell(&mut self, fact: Fact) {
        write(&self.stream, &Message::Fact(fact));
    }

    /// Sets the reviews that every `GetReviews` of this workspace is answered with from
    /// now on. It tells the Window nothing: a test that wants it to read them tells
    /// `Fact::ReviewsChanged` after.
    pub fn set_reviews(&self, reviews: WorkspaceReviews) {
        self.reviews
            .set
            .lock()
            .expect("the reviews are not poisoned")
            .insert(reviews.workspace.clone(), reviews);
    }

    /// How many times the Window asked for reviews so far.
    pub fn reviews_asked(&self) -> usize {
        self.reviews.heard.load(Ordering::SeqCst)
    }

    /// From now on keeps the requests for reviews unanswered, until
    /// [`KeeperEnd::release_reviews`]: a Keeper that is slow.
    pub fn hold_reviews(&self) {
        self.reviews.holding.store(true, Ordering::SeqCst);
    }

    /// Answers the requests for reviews that were kept, in the order they came, with what
    /// is set now, and answers the ones that follow at once.
    pub fn release_reviews(&self) {
        let mut held = self.reviews.held.lock().expect("not poisoned");
        self.reviews.holding.store(false, Ordering::SeqCst);
        for (id, workspace) in held.drain(..) {
            let answer = self.reviews.answer(&workspace);
            write(
                &self.stream,
                &Message::Reply {
                    to: id,
                    reply: Ok(Reply::Reviews(answer)),
                },
            );
        }
    }

    /// Answers the next request, which has to be the one for the changes of the
    /// workspace on screen, with `changes`; then the request for each of their files,
    /// which the pane makes in the order of the list, with the versions `sides` gives
    /// for its path.
    pub fn show(
        &mut self,
        changes: &Changes,
        sides: impl Fn(&ChangedFile) -> Result<FileSides, Refusal>,
        cx: &mut TestAppContext,
    ) {
        let asked = self.expect(
            Request::GetChanges {
                workspace: changes.workspace.clone(),
            },
            cx,
        );
        self.reply(asked, Ok(Reply::Changes(changes.clone())));
        for file in &changes.files {
            let asked = self.expect(read_file(changes, file), cx);
            self.reply(asked, sides(file).map(Reply::File));
        }
    }
}

/// The id of the review of no agent that every workspace has until a test says otherwise.
pub fn review_id() -> ReviewId {
    ReviewId::new("e".repeat(32))
}

/// A review of no agent with nothing in it.
pub fn no_agent_review() -> Review {
    Review {
        id: review_id(),
        agent: None,
        reviewed: None,
        attached: None,
        threads: Vec::new(),
        rounds: Vec::new(),
    }
}

/// The request by which a Window reads the two versions of one file of `changes`.
pub fn read_file(changes: &Changes, file: &ChangedFile) -> Request {
    let (since, old_path) = match &file.status {
        FileStatus::Added => (None, file.path.clone()),
        FileStatus::Renamed { from } => (changes.counted_from.clone(), from.clone()),
        FileStatus::Modified | FileStatus::Deleted => {
            (changes.counted_from.clone(), file.path.clone())
        }
    };
    Request::ReadFile {
        workspace: changes.workspace.clone(),
        since,
        old_path,
        path: file.path.clone(),
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

pub fn id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

pub fn workspace(digit: char, branch: &str) -> Workspace {
    Workspace {
        id: id(digit),
        path: PathBuf::from(format!("/gits/{branch}")),
        head: Some(Head::Branch(branch.to_owned())),
    }
}

pub fn repository(workspaces: Vec<Workspace>) -> Repository {
    Repository {
        id: RepositoryId::new("a".repeat(32)),
        name: "repository".to_owned(),
        workspaces,
    }
}

pub fn file(path: &str, status: FileStatus) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        status,
    }
}

pub fn commit(digit: char, summary: &str) -> Commit {
    Commit {
        id: digit.to_string().repeat(40),
        summary: summary.to_owned(),
    }
}

/// The base a repository has when nobody set one.
pub fn default_base(name: &str) -> Base {
    Base {
        name: name.to_owned(),
        source: BaseSource::Default,
    }
}

/// The changes of a branch of workspace `1` since it left `main`, counted from
/// [`COMMIT`].
pub fn since_main(commits: Vec<Commit>, files: Vec<ChangedFile>) -> Changes {
    Changes {
        workspace: id('1'),
        comparison: Comparison::SinceBase {
            base: default_base("main"),
        },
        counted_from: Some(COMMIT.to_owned()),
        commits,
        files,
    }
}

/// Two versions of a file that is text on both sides.
pub fn texts(old: &str, new: &str) -> Result<FileSides, Refusal> {
    Ok(FileSides {
        old: Content::Text(old.as_bytes().to_vec()),
        new: Content::Text(new.as_bytes().to_vec()),
    })
}

/// The view a test window holds: the pane as the Frame would lay it out, alone.
struct Host {
    pane: AnyView,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.pane.clone())
    }
}

/// A test window with the pane in it, and the Keeper the test plays.
pub struct Stage {
    /// The pane, as the model a test reads back through.
    pub pane: Entity<ReviewPane>,
    /// Where the keys go when the pane has the focus, as the pane told the Frame.
    pub focus: FocusHandle,
    /// The shared models the pane was built on.
    pub models: Models,
    /// The other end of the Window's connection.
    pub keeper: KeeperEnd,
}

/// The shared models of a window, fed by a Keeper the test plays. The Keeper has listed
/// `workspaces` of one repository, and the first of them is on screen; the request for
/// its changes is the next the Keeper hears. The keys of the pane are bound from the
/// pane's own table, the way the Window program binds them.
pub fn link_up(workspaces: Vec<Workspace>, cx: &mut TestAppContext) -> (Models, KeeperEnd) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind(&(yaar_review_pane::entry().controls)(), cx);
    });
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach(&link, cx);
    let asked = keeper.expect(Request::ListWorkspaces, cx);
    keeper.reply(asked, Ok(Reply::Workspaces(vec![repository(workspaces)])));
    wait_until(cx, "a workspace on screen", |cx| {
        models
            .workspaces
            .read_with(cx, |list, _| list.selected().is_some())
    });
    (models, keeper)
}

/// Opens a window that holds the pane alone, built by the pane's own builder the way
/// the Window program builds it.
pub fn open_window(
    models: Models,
    keeper: KeeperEnd,
    cx: &mut TestAppContext,
) -> (Stage, VisualTestContext) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: WINDOW,
        })),
        ..Default::default()
    };
    let mut built = None;
    let (window, _host) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                let pane = (yaar_review_pane::entry().build)()(&models, window, cx);
                assert_eq!(pane.name, yaar_review_pane::NAME);
                let view = pane.view.clone();
                built = Some(pane);
                cx.new(|_| Host { pane: view })
            })
        })
        .expect("open a test window");
    let built = built.expect("the pane was built");
    let pane = built
        .view
        .downcast::<ReviewPane>()
        .expect("the view of the pane is the pane");
    let cx = VisualTestContext::from_window(window, cx);
    (
        Stage {
            pane,
            focus: built.focus,
            models,
            keeper,
        },
        cx,
    )
}

/// A window with the pane in it, linked as [`link_up`] links it: the pane is there
/// before the Keeper has said anything about the changes.
pub fn open(workspaces: Vec<Workspace>, cx: &mut TestAppContext) -> (Stage, VisualTestContext) {
    let (models, keeper) = link_up(workspaces, cx);
    open_window(models, keeper, cx)
}

/// The rows of the pane, as they are now.
pub fn rows(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Vec<PaneRow> {
    pane.read_with(cx, |pane, _| pane.rows().to_vec())
}

/// Waits until the pane shows rows and none of its files is still on its way.
pub fn wait_for_files(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) {
    wait_until(cx, "every file of the changes", |cx| {
        pane.read_with(cx, |pane, _| {
            !pane.rows().is_empty()
                && !pane.rows().iter().any(|row| {
                    matches!(
                        row,
                        PaneRow::Unshown {
                            why: Unshown::Loading,
                            ..
                        }
                    )
                })
        })
    });
}

/// Where a row of the pane was drawn in the last frame; fails when it was not laid out.
pub fn bounds_of_row(index: usize, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    drawn(&format!("review-row-{index}"), cx)
        .unwrap_or_else(|| panic!("row {index} of the pane is not laid out"))
}

/// Where the element with this selector was drawn in the last frame, if it was.
pub fn drawn(selector: &str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    // The selector has to outlive the call, and a test ends soon.
    cx.debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
}
