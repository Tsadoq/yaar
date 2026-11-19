//! Component test: the comment box and the threads of the Review pane on GPUI's test
//! platform, with no window on any screen. Keys, clicks and the wheel go in; the pane is
//! read back through the model it shows. The test plays the Keeper on the other end of a
//! socket pair: it answers what the Window asks for the changes and for each file, and
//! the helper of `common` answers the reviews.

mod common;

use std::path::PathBuf;

use gpui_kit::{
    ClipboardItem, Entity, Modifiers, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext,
    point, px,
};
use yaar_diff::{Row, Side};
use yaar_link::{
    Anchor, Author, Changes, Comment, CommentId, DraftOn, Fact, FileStatus, Lines, Refusal, Reply,
    Request, Review, Round, SentBy, Side as LinkSide, Thread, ThreadId, WorkspaceReviews,
};
use yaar_review_pane::{CommentBox, PaneRow, Position, ReviewPane};

use common::{
    COMMIT, Stage, bounds_of_row, drawn, file, id, no_agent_review, open, review_id, rows,
    since_main, texts, wait_for_files, wait_until, workspace,
};

/// Two versions of `src/lib.rs`: a line removed, a line changed and a line added, with
/// unchanged lines between.
const LIB_OLD: &str = "fn start() {}\nfn keep_a() {}\nfn keep_b() {}\nfn keep_c() {}\n\
removed line\nfn keep_d() {}\nfn keep_e() {}\nfn keep_f() {}\nfn keep_g() {}\n\
let old = 1;\nfn keep_h() {}\n";
const LIB_NEW: &str = "fn start() {}\nfn keep_a() {}\nfn keep_b() {}\nfn keep_c() {}\n\
fn keep_d() {}\nfn keep_e() {}\nfn keep_f() {}\nfn keep_g() {}\n\
let new = 2;\nfn keep_h() {}\nadded line\n";

/// A file of 200 lines whose change is near its end.
fn long(changed: bool) -> String {
    (0..200)
        .map(|line| match (changed, line) {
            (true, 197) => "line 197 changed\n".to_owned(),
            _ => format!("line {line}\n"),
        })
        .collect()
}

fn lib() -> PathBuf {
    PathBuf::from("src/lib.rs")
}

fn long_path() -> PathBuf {
    PathBuf::from("src/long.rs")
}

/// Opens the pane on the two files and waits until every file is shown. The pane has the
/// focus, as after a click on the first line of the diff.
fn open_on_two_files(cx: &mut TestAppContext) -> (Stage, Changes, VisualTestContext) {
    open_on_two_files_of(vec![workspace('1', "feature")], cx)
}

/// As [`open_on_two_files`], for the first of these workspaces, which is on screen.
fn open_on_two_files_of(
    workspaces: Vec<yaar_link::Workspace>,
    cx: &mut TestAppContext,
) -> (Stage, Changes, VisualTestContext) {
    let (mut stage, mut cx) = open(workspaces, cx);
    let changes = since_main(
        Vec::new(),
        vec![
            file("src/lib.rs", FileStatus::Modified),
            file("src/long.rs", FileStatus::Modified),
        ],
    );
    stage.keeper.show(
        &changes,
        |file| {
            if file.path == lib() {
                texts(LIB_OLD, LIB_NEW)
            } else {
                texts(&long(false), &long(true))
            }
        },
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);
    let first_line = bounds_of_row(2, &mut cx).center();
    cx.simulate_click(first_line, Modifiers::none());
    cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    (stage, changes, cx)
}

fn position(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Position {
    pane.read_with(cx, |pane, _| pane.position().cloned())
        .expect("the reviewer is on a line")
}

fn comment_box(pane: &Entity<ReviewPane>, cx: &mut TestAppContext) -> Option<CommentBox> {
    pane.read_with(cx, |pane, _| pane.comment_box())
}

/// Presses `key` until the reviewer is on `wanted`.
fn walk_to(wanted: &Position, key: &str, pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) {
    for _ in 0..500 {
        if position(pane, cx) == *wanted {
            return;
        }
        cx.simulate_keystrokes(key);
    }
    panic!("{key} never came to {wanted:?}");
}

/// The index of the row of `path` that shows this row of its diff.
fn row_of(
    pane: &Entity<ReviewPane>,
    path: &PathBuf,
    wanted: Row,
    cx: &mut VisualTestContext,
) -> usize {
    rows(pane, cx)
        .iter()
        .position(
            |row| matches!(row, PaneRow::Diff { path: at, row } if at == path && *row == wanted),
        )
        .unwrap_or_else(|| panic!("no row {wanted:?} in {path:?}"))
}

/// Whether `text` is an id as the Keeper makes them: 32 lowercase hexadecimal digits.
fn is_an_id(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Presses `c` where the reviewer is, and holds the box that opens to what the row says:
/// on this line, of this side, with its text, in this version.
fn comment_on(at: &Position, pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> CommentBox {
    assert_eq!(position(pane, cx), *at);
    assert_eq!(comment_box(pane, cx), None);
    cx.simulate_keystrokes("c");
    let opened = comment_box(pane, cx).unwrap_or_else(|| panic!("no comment box on {at:?}"));
    let text = pane
        .read_with(cx, |pane, _| pane.text(at))
        .expect("the pane holds the line");
    assert_eq!(opened.review, review_id());
    assert_eq!(opened.at, *at);
    assert_eq!(opened.text, "");
    assert!(is_an_id(opened.thread.as_str()) && is_an_id(opened.comment.as_str()));
    let anchor = opened.anchor.clone().expect("a new thread has its anchor");
    assert_eq!(anchor.path, at.path);
    assert_eq!(
        anchor.side,
        match at.side {
            Side::Old => LinkSide::Old,
            Side::New => LinkSide::New,
        }
    );
    // The Link counts lines from 1, the pane from 0.
    assert_eq!(
        anchor.lines,
        Lines {
            first: at.line + 1,
            last: at.line + 1
        }
    );
    assert_eq!(anchor.text, text);
    assert_eq!(
        anchor.tree,
        match at.side {
            Side::Old => Some(COMMIT.to_owned()),
            Side::New => None,
        }
    );
    assert!(drawn("review-comment-box", cx).is_some());
    opened
}

/// Closes the box with `escape`, which leaves the reviewer where they were.
fn close_box(at: &Position, pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("escape");
    assert_eq!(comment_box(pane, cx), None);
    assert_eq!(drawn("review-comment-box", cx), None);
    assert_eq!(position(pane, cx), *at);
}

#[gpui_kit::test]
fn a_comment_box_opens_on_any_line_of_a_diff_removed_lines_included_in_the_unified_and_the_split_view(
    cx: &mut TestAppContext,
) {
    let (stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = &stage.pane;
    let at = |side, line| Position {
        path: lib(),
        side,
        line,
    };

    // Unified view, a removed line, a context line and an added line in turn: the first
    // by a click on its row, the second by `k`, the third by `j`.
    let removed = at(Side::Old, 4);
    let row = row_of(pane, &lib(), Row::Removed { old: 4 }, &mut cx);
    let middle = bounds_of_row(row, &mut cx).center();
    cx.simulate_click(middle, Modifiers::none());
    comment_on(&removed, pane, &mut cx);
    close_box(&removed, pane, &mut cx);

    let context = at(Side::New, 3);
    walk_to(&context, "k", pane, &mut cx);
    comment_on(&context, pane, &mut cx);
    close_box(&context, pane, &mut cx);

    let added = at(Side::New, 10);
    walk_to(&added, "j", pane, &mut cx);
    comment_on(&added, pane, &mut cx);
    close_box(&added, pane, &mut cx);

    // And back up by `k`, to a removed line of a change that is also an addition.
    let replaced = at(Side::Old, 9);
    walk_to(&replaced, "k", pane, &mut cx);
    comment_on(&replaced, pane, &mut cx);
    close_box(&replaced, pane, &mut cx);

    // The split view: a click on each half of a changed row, then `c`.
    cx.simulate_keystrokes("s");
    let changed = row_of(
        pane,
        &lib(),
        Row::Changed {
            old: Some(9),
            new: Some(8),
        },
        &mut cx,
    );
    for (half, side, line) in [("old", Side::Old, 9), ("new", Side::New, 8)] {
        let half = drawn(&format!("review-row-{changed}-{half}"), &mut cx)
            .expect("the half of the row is laid out");
        cx.simulate_click(half.center(), Modifiers::none());
        let on = at(side, line);
        comment_on(&on, pane, &mut cx);
        close_box(&on, pane, &mut cx);
    }
    cx.simulate_keystrokes("s");

    // Any line: the last of a file of 200 lines, below the rows in view, brought into
    // view by the wheel and reached by a click on its row.
    let last = rows(pane, &mut cx).len() - 1;
    assert!(
        drawn(&format!("review-row-{last}"), &mut cx).is_none(),
        "the last row is below the rows in view"
    );
    let over_the_rows = bounds_of_row(5, &mut cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    let middle = bounds_of_row(last, &mut cx).center();
    cx.simulate_click(middle, Modifiers::none());
    let end = Position {
        path: long_path(),
        side: Side::New,
        line: 199,
    };
    let opened = comment_on(&end, pane, &mut cx);
    assert_eq!(opened.anchor.expect("an anchor").text, "line 199");
    close_box(&end, pane, &mut cx);
}

/// The request that saves the draft `opened` describes with this text, as a new thread
/// does at every save.
fn new_thread_save(opened: &CommentBox, text: &str) -> Request {
    Request::SaveDraft {
        review: opened.review.clone(),
        thread: opened.thread.clone(),
        comment: opened.comment.clone(),
        on: DraftOn::NewThread {
            anchor: opened.anchor.clone().expect("a new thread has its anchor"),
        },
        text: text.to_owned(),
    }
}

/// The text of the one comment of the one thread the pane holds for the review, or why
/// there is not one.
fn the_draft_of_the_pane(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Comment {
    pane.read_with(cx, |pane, _| {
        let review = pane.review().expect("the pane holds a review");
        assert_eq!(review.threads.len(), 1);
        assert_eq!(review.threads[0].comments.len(), 1);
        review.threads[0].comments[0].clone()
    })
}

#[gpui_kit::test]
fn text_typed_in_a_comment_box_is_saved_while_the_reviewer_types_without_leaving_the_box(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let added = Position {
        path: lib(),
        side: Side::New,
        line: 10,
    };
    walk_to(&added, "j", &pane, &mut cx);
    let opened = comment_on(&added, &pane, &mut cx);

    // Three keys, one at a time, and no answer: the box stays open with the text so far.
    for text in ["h", "he", "hey"] {
        cx.simulate_input(&text[text.len() - 1..]);
        let shown = comment_box(&pane, &mut cx).expect("the box is still open");
        assert_eq!(shown.text, text);
    }
    // Each key was saved at once, with the whole text, under the same ids.
    let mut asked = Vec::new();
    for text in ["h", "he", "hey"] {
        asked.push(stage.keeper.expect(new_thread_save(&opened, text), &mut cx));
    }
    for id in asked {
        stage.keeper.reply(id, Ok(Reply::Done));
    }
    cx.run_until_parked();
    assert!(comment_box(&pane, &mut cx).is_some());

    // A key after the answers lands in the same box and is saved the same way.
    cx.simulate_input("!");
    let asked = stage
        .keeper
        .expect(new_thread_save(&opened, "hey!"), &mut cx);
    stage.keeper.reply(asked, Ok(Reply::Done));

    // Shift-Enter makes a new line and leaves the box open, and so does the next key.
    cx.simulate_keystrokes("shift-enter");
    let asked = stage
        .keeper
        .expect(new_thread_save(&opened, "hey!\n"), &mut cx);
    stage.keeper.reply(asked, Ok(Reply::Done));
    cx.simulate_input("z");
    let asked = stage
        .keeper
        .expect(new_thread_save(&opened, "hey!\nz"), &mut cx);
    stage.keeper.reply(asked, Ok(Reply::Done));
    let shown = comment_box(&pane, &mut cx).expect("the box is still open");
    assert_eq!(shown.text, "hey!\nz");

    // Enter closes the box. The pane holds the draft as typed with no answer of the
    // Keeper but the Dones, and draws it.
    cx.simulate_keystrokes("enter");
    assert_eq!(comment_box(&pane, &mut cx), None);
    let comment_id = opened.comment.clone();
    let draft = Comment {
        id: comment_id.clone(),
        author: Author::Reviewer,
        text: "hey!\nz".to_owned(),
        round: None,
    };
    let held = pane.read_with(&cx, |pane, _| pane.review().cloned());
    assert_eq!(
        held,
        Some(Review {
            threads: vec![Thread {
                id: opened.thread.clone(),
                anchor: opened.anchor.clone().expect("an anchor"),
                resolved: false,
                addressed: false,
                comments: vec![draft.clone()],
            }],
            ..no_agent_review()
        })
    );
    assert!(drawn(&format!("review-comment-{comment_id}:draft"), &mut cx).is_some());

    // The same on an edit: `i` opens the draft, its next key is saved as a draft of a
    // thread that exists, and Escape closes the box with the text kept.
    cx.simulate_keystrokes("i");
    let editing = comment_box(&pane, &mut cx).expect("the draft is open to edit");
    assert_eq!(editing.comment, comment_id);
    assert_eq!(editing.thread, opened.thread);
    assert_eq!(editing.anchor, None);
    assert_eq!(editing.text, "hey!\nz");
    cx.simulate_input("x");
    let asked = stage.keeper.expect(
        Request::SaveDraft {
            review: review_id(),
            thread: opened.thread.clone(),
            comment: comment_id.clone(),
            on: DraftOn::Thread,
            text: "hey!\nzx".to_owned(),
        },
        &mut cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Done));
    cx.simulate_keystrokes("escape");
    assert_eq!(comment_box(&pane, &mut cx), None);
    assert_eq!(the_draft_of_the_pane(&pane, &mut cx).text, "hey!\nzx");
}

/// The id with this digit repeated, as the tests of the threads name them.
fn thread_id(digit: char) -> ThreadId {
    ThreadId::new(digit.to_string().repeat(32))
}

fn comment_id(digit: char) -> CommentId {
    CommentId::new(digit.to_string().repeat(32))
}

/// A comment of the reviewer that was sent in this round.
fn sent(digit: char, text: &str, round: u32) -> Comment {
    Comment {
        id: comment_id(digit),
        author: Author::Reviewer,
        text: text.to_owned(),
        round: Some(round),
    }
}

/// A draft of the reviewer.
fn draft(digit: char, text: &str) -> Comment {
    Comment {
        id: comment_id(digit),
        author: Author::Reviewer,
        text: text.to_owned(),
        round: None,
    }
}

/// A thread as the Keeper holds it, in the version the changes are counted from, on the
/// line `line` of `path` counted from 1.
fn thread(digit: char, path: &str, side: LinkSide, line: u32, comments: Vec<Comment>) -> Thread {
    Thread {
        id: thread_id(digit),
        anchor: Anchor {
            tree: Some(COMMIT.to_owned()),
            path: path.into(),
            side,
            lines: Lines {
                first: line,
                last: line,
            },
            text: "the code".to_owned(),
        },
        resolved: false,
        addressed: false,
        comments,
    }
}

/// The reviews of workspace `1` with these threads in its one review.
fn reviews_with(threads: Vec<Thread>) -> WorkspaceReviews {
    let any_sent = threads
        .iter()
        .flat_map(|thread| &thread.comments)
        .any(|comment| comment.round.is_some());
    WorkspaceReviews {
        workspace: id('1'),
        reviews: vec![Review {
            threads,
            rounds: if any_sent {
                vec![Round {
                    number: 1,
                    state: "5".repeat(40),
                    sent_by: SentBy::Copied,
                }]
            } else {
                Vec::new()
            },
            ..no_agent_review()
        }],
        viewed: Vec::new(),
    }
}

/// Makes the Keeper hold these reviews and tell it, and waits until the pane shows them.
fn give(stage: &mut Stage, reviews: &WorkspaceReviews, cx: &mut VisualTestContext) {
    stage.keeper.set_reviews(reviews.clone());
    stage.keeper.tell(Fact::ReviewsChanged {
        workspace: reviews.workspace.clone(),
    });
    wait_until(cx, "the reviews the Keeper holds", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.reviews() == Some(reviews))
    });
}

/// The selector of the row of a thread, and of a comment of it.
fn drawn_thread(thread: char, cx: &mut VisualTestContext) -> bool {
    drawn(&format!("review-thread-{}", thread_id(thread)), cx).is_some()
}

fn drawn_comment(comment: char, state: &str, cx: &mut VisualTestContext) -> bool {
    drawn(
        &format!("review-comment-{}:{state}", comment_id(comment)),
        cx,
    )
    .is_some()
}

fn notice(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<String> {
    pane.read_with(cx, |pane, _| pane.notice())
}

#[gpui_kit::test]
fn the_pane_reads_the_reviews_of_the_workspace_on_screen_and_again_when_the_keeper_tells_they_changed(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    // Asked once the changes were here, and answered with a review of no agent.
    wait_until(&mut cx, "the first reviews", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.reviews().is_some())
    });
    assert_eq!(stage.keeper.reviews_asked(), 1);
    let first = stage
        .pane
        .read_with(&cx, |pane, _| pane.reviews().cloned())
        .expect("reviews");
    assert_eq!(first.workspace, id('1'));
    assert_eq!(first.reviews, [no_agent_review()]);
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.review().cloned()),
        Some(no_agent_review())
    );

    // The Keeper tells of a change of the reviews of another workspace: nothing is asked.
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('9') });
    cx.run_until_parked();
    assert_eq!(stage.keeper.reviews_asked(), 1);

    // Of this one: the reviews are read again and shown.
    let threads = reviews_with(vec![thread(
        'a',
        "src/lib.rs",
        LinkSide::Old,
        5,
        vec![sent('b', "why remove this", 1)],
    )]);
    give(&mut stage, &threads, &mut cx);
    assert_eq!(stage.keeper.reviews_asked(), 2);
}

#[gpui_kit::test]
fn a_keeper_that_refuses_the_reviews_leaves_the_pane_with_no_review_and_no_notice_and_working(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    stage.keeper.refuse_reviews(Some(Refusal::Failed {
        reason: "not wired".to_owned(),
    }));
    let changes = since_main(Vec::new(), vec![file("src/lib.rs", FileStatus::Modified)]);
    stage
        .keeper
        .show(&changes, |_| texts(LIB_OLD, LIB_NEW), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    let first_line = bounds_of_row(2, &mut cx).center();
    cx.simulate_click(first_line, Modifiers::none());
    wait_until(&mut cx, "the refusal", |_| {
        stage.keeper.reviews_asked() == 1
    });
    cx.run_until_parked();
    let (reviews, review, notice_now) = stage.pane.read_with(&cx, |pane, _| {
        (
            pane.reviews().cloned(),
            pane.review().cloned(),
            pane.notice(),
        )
    });
    assert_eq!((reviews, review, notice_now), (None, None, None));

    // The pane goes on: the reviewer walks the lines, and a comment is refused for want of
    // a review, which the notice says.
    let before = position(&stage.pane, &mut cx);
    cx.simulate_keystrokes("j");
    assert_ne!(position(&stage.pane, &mut cx), before);
    cx.simulate_keystrokes("c");
    assert_eq!(comment_box(&stage.pane, &mut cx), None);
    assert!(notice(&stage.pane, &mut cx).is_some());
    assert!(drawn("review-notice", &mut cx).is_some());
}

#[gpui_kit::test]
fn a_thread_is_drawn_after_the_row_that_shows_its_last_line_with_a_row_for_each_comment(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    // A thread on a removed line, one on an added line with a reply, and one on a line no
    // row shows: line 1 of `src/lib.rs` is among the unchanged lines left out.
    let reviews = reviews_with(vec![
        thread(
            'a',
            "src/lib.rs",
            LinkSide::Old,
            5,
            vec![sent('1', "why remove this", 1)],
        ),
        thread(
            'b',
            "src/lib.rs",
            LinkSide::New,
            11,
            vec![sent('2', "is this needed", 1), draft('3', "my reply")],
        ),
        thread(
            'c',
            "src/lib.rs",
            LinkSide::New,
            1,
            vec![sent('4', "an old remark", 1)],
        ),
    ]);
    give(&mut stage, &reviews, &mut cx);

    let all = rows(&pane, &mut cx);
    let removed = row_of(&pane, &lib(), Row::Removed { old: 4 }, &mut cx);
    let added = row_of(&pane, &lib(), Row::Added { new: 10 }, &mut cx);
    assert_eq!(
        all[removed + 1..removed + 3],
        [
            PaneRow::Thread {
                thread: thread_id('a')
            },
            PaneRow::Comment {
                thread: thread_id('a'),
                comment: comment_id('1')
            },
        ]
    );
    assert_eq!(
        all[added + 1..added + 4],
        [
            PaneRow::Thread {
                thread: thread_id('b')
            },
            PaneRow::Comment {
                thread: thread_id('b'),
                comment: comment_id('2')
            },
            PaneRow::Comment {
                thread: thread_id('b'),
                comment: comment_id('3')
            },
        ]
    );
    // The thread that no row shows is after the last file, under a heading.
    let end = all.len();
    assert_eq!(
        all[end - 3..],
        [
            PaneRow::ThreadsElsewhere,
            PaneRow::Thread {
                thread: thread_id('c')
            },
            PaneRow::Comment {
                thread: thread_id('c'),
                comment: comment_id('4')
            },
        ]
    );
    assert_eq!(
        all.iter()
            .filter(|row| matches!(row, PaneRow::Thread { .. }))
            .count(),
        3,
        "a thread is shown once"
    );

    // What is in view is drawn under the selectors that name it.
    assert!(drawn_thread('a', &mut cx));
    assert!(drawn_comment('1', "round-1", &mut cx));
    assert!(drawn_thread('b', &mut cx));
    assert!(drawn_comment('2', "round-1", &mut cx));
    assert!(drawn_comment('3', "draft", &mut cx));
    assert!(!drawn_thread('c', &mut cx), "it is below the rows in view");

    // The rows of a thread are no stops of `j` and `k`: from the removed line the next
    // line is the next row of the diff, as it is with no thread, and back.
    let on_removed = bounds_of_row(removed, &mut cx).center();
    cx.simulate_click(on_removed, Modifiers::none());
    cx.simulate_keystrokes("j");
    assert_eq!(
        position(&pane, &mut cx),
        Position {
            path: lib(),
            side: Side::New,
            line: 4
        }
    );
    cx.simulate_keystrokes("k");
    assert_eq!(
        position(&pane, &mut cx),
        Position {
            path: lib(),
            side: Side::Old,
            line: 4
        }
    );

    let over_the_rows = bounds_of_row(5, &mut cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    assert!(drawn("review-threads-elsewhere", &mut cx).is_some());
    assert!(drawn_thread('c', &mut cx));
    assert!(drawn_comment('4', "round-1", &mut cx));
}

#[gpui_kit::test]
fn a_reply_opens_on_a_thread_whose_last_comment_was_sent_and_on_no_other(cx: &mut TestAppContext) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    // A thread with a comment that was sent, on the added line; one with nothing sent
    // yet, on the removed line.
    let reviews = reviews_with(vec![
        thread(
            'a',
            "src/lib.rs",
            LinkSide::New,
            11,
            vec![sent('1', "is this needed", 1)],
        ),
        thread(
            'b',
            "src/lib.rs",
            LinkSide::Old,
            5,
            vec![draft('2', "still writing")],
        ),
    ]);
    give(&mut stage, &reviews, &mut cx);

    let added = Position {
        path: lib(),
        side: Side::New,
        line: 10,
    };
    walk_to(&added, "j", &pane, &mut cx);
    cx.simulate_keystrokes("o");
    let opened = comment_box(&pane, &mut cx).expect("a reply box");
    assert_eq!(opened.thread, thread_id('a'));
    assert_ne!(opened.comment, comment_id('1'));
    assert!(is_an_id(opened.comment.as_str()));
    assert_eq!(opened.anchor, None);
    assert_eq!(opened.review, review_id());
    assert_eq!(opened.text, "");

    // Its first key is saved as a draft of a thread that exists, and the pane holds the
    // reply after the comment that was sent.
    cx.simulate_input("x");
    let asked = stage.keeper.expect(
        Request::SaveDraft {
            review: review_id(),
            thread: thread_id('a'),
            comment: opened.comment.clone(),
            on: DraftOn::Thread,
            text: "x".to_owned(),
        },
        &mut cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Done));
    let held = pane
        .read_with(&cx, |pane, _| pane.review().cloned())
        .expect("a review");
    assert_eq!(
        held.threads[0].comments,
        [
            sent('1', "is this needed", 1),
            draft_with(&opened.comment, "x")
        ]
    );
    // The box takes room from the rows: they are drawn once it is closed.
    cx.simulate_keystrokes("escape");
    assert!(drawn_comment('1', "round-1", &mut cx));
    assert!(drawn(&format!("review-comment-{}:draft", opened.comment), &mut cx).is_some());

    // A thread that holds a draft takes no second one: the draft is edited.
    assert_eq!(notice(&pane, &mut cx), None);
    cx.simulate_keystrokes("o");
    assert_eq!(comment_box(&pane, &mut cx), None);
    assert!(notice(&pane, &mut cx).is_some());
    assert!(drawn("review-notice", &mut cx).is_some());
    // The next action of the reviewer takes the notice away.
    cx.simulate_keystrokes("k");
    assert_eq!(notice(&pane, &mut cx), None);
    assert_eq!(drawn("review-notice", &mut cx), None);

    // A thread with nothing sent has nothing to reply to.
    let removed = Position {
        path: lib(),
        side: Side::Old,
        line: 4,
    };
    walk_to(&removed, "k", &pane, &mut cx);
    cx.simulate_keystrokes("o");
    assert_eq!(comment_box(&pane, &mut cx), None);
    assert!(drawn("review-notice", &mut cx).is_some());

    // And a line with no thread has none.
    cx.simulate_keystrokes("j");
    cx.simulate_keystrokes("o");
    assert_eq!(comment_box(&pane, &mut cx), None);
    assert!(drawn("review-notice", &mut cx).is_some());
}

/// A draft of the reviewer under this id.
fn draft_with(id: &CommentId, text: &str) -> Comment {
    Comment {
        id: id.clone(),
        author: Author::Reviewer,
        text: text.to_owned(),
        round: None,
    }
}

#[gpui_kit::test]
fn m_and_shift_m_walk_the_threads_in_the_order_of_the_rows_and_say_where_there_is_none(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    // Threads on two lines of one file and one on another file; a fourth that no row
    // shows, which has no line to stand on.
    let reviews = reviews_with(vec![
        thread(
            'a',
            "src/lib.rs",
            LinkSide::Old,
            5,
            vec![sent('1', "one", 1)],
        ),
        thread(
            'b',
            "src/lib.rs",
            LinkSide::New,
            9,
            vec![sent('2', "two", 1)],
        ),
        thread(
            'c',
            "src/long.rs",
            LinkSide::New,
            200,
            vec![sent('3', "three", 1)],
        ),
        thread(
            'd',
            "src/lib.rs",
            LinkSide::New,
            1,
            vec![sent('4', "four", 1)],
        ),
    ]);
    give(&mut stage, &reviews, &mut cx);
    let at = |path: PathBuf, side, line| Position { path, side, line };

    // The reviewer starts on the removed line, where the first thread is: the next is the
    // one on the new side of the changed pair, then the one in the other file.
    assert_eq!(position(&pane, &mut cx), at(lib(), Side::Old, 4));
    cx.simulate_keystrokes("m");
    assert_eq!(position(&pane, &mut cx), at(lib(), Side::New, 8));
    cx.simulate_keystrokes("m");
    assert_eq!(position(&pane, &mut cx), at(long_path(), Side::New, 199));
    // The view follows the reviewer to a line far below.
    let row = pane
        .read_with(&cx, |pane, _| pane.position_row())
        .expect("a row");
    assert!(pane.read_with(&cx, |pane, _| pane.rows_in_view().contains(&row)));

    // Past the last thread that stands on a line, nothing moves and the notice says so.
    cx.simulate_keystrokes("m");
    assert_eq!(position(&pane, &mut cx), at(long_path(), Side::New, 199));
    assert!(notice(&pane, &mut cx).is_some());
    assert!(drawn("review-notice", &mut cx).is_some());

    // And back.
    cx.simulate_keystrokes("shift-m");
    assert_eq!(notice(&pane, &mut cx), None);
    assert_eq!(position(&pane, &mut cx), at(lib(), Side::New, 8));
    cx.simulate_keystrokes("shift-m");
    assert_eq!(position(&pane, &mut cx), at(lib(), Side::Old, 4));
    cx.simulate_keystrokes("shift-m");
    assert_eq!(position(&pane, &mut cx), at(lib(), Side::Old, 4));
    assert!(drawn("review-notice", &mut cx).is_some());
}

#[gpui_kit::test]
fn m_with_no_thread_at_all_moves_nothing_and_says_so(cx: &mut TestAppContext) {
    let (stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = &stage.pane;
    let before = position(pane, &mut cx);
    cx.simulate_keystrokes("m");
    assert_eq!(position(pane, &mut cx), before);
    assert!(drawn("review-notice", &mut cx).is_some());
    cx.simulate_keystrokes("shift-m");
    assert_eq!(position(pane, &mut cx), before);
    assert!(drawn("review-notice", &mut cx).is_some());
}

#[gpui_kit::test]
fn a_comment_on_a_renamed_file_names_the_old_side_by_the_old_path_in_the_version_counted_from(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        Vec::new(),
        vec![file(
            "src/new_name.rs",
            FileStatus::Renamed {
                from: "src/old_name.rs".into(),
            },
        )],
    );
    stage.keeper.show(
        &changes,
        |_| texts("keep\nold\nend\n", "keep\nnew\nend\n"),
        &mut cx,
    );
    wait_for_files(&stage.pane, &mut cx);
    let pane = stage.pane.clone();
    let first_line = bounds_of_row(2, &mut cx).center();
    cx.simulate_click(first_line, Modifiers::none());
    let renamed = PathBuf::from("src/new_name.rs");
    let at = |side, line| Position {
        path: renamed.clone(),
        side,
        line,
    };

    // The old side: the file as it was named in the version the changes are counted from.
    let old = at(Side::Old, 1);
    walk_to(&old, "j", &pane, &mut cx);
    cx.simulate_keystrokes("c");
    let opened = comment_box(&pane, &mut cx).expect("a box on the old side");
    let anchor = opened.anchor.expect("an anchor");
    assert_eq!(anchor.path, PathBuf::from("src/old_name.rs"));
    assert_eq!(anchor.tree, Some(COMMIT.to_owned()));
    assert_eq!(anchor.side, LinkSide::Old);
    assert_eq!(anchor.text, "old");
    cx.simulate_keystrokes("escape");

    // The new side: the file as it is, which the Keeper records.
    let new = at(Side::New, 1);
    walk_to(&new, "j", &pane, &mut cx);
    cx.simulate_keystrokes("c");
    let opened = comment_box(&pane, &mut cx).expect("a box on the new side");
    let anchor = opened.anchor.expect("an anchor");
    assert_eq!(anchor.path, renamed);
    assert_eq!(anchor.tree, None);
    assert_eq!(anchor.side, LinkSide::New);
    assert_eq!(anchor.text, "new");
    cx.simulate_keystrokes("escape");

    // A thread of the old side that the Keeper holds under the old path sits on the
    // removed line, and a thread of the new side under the new path on the added one.
    let reviews = reviews_with(vec![
        thread(
            'a',
            "src/old_name.rs",
            LinkSide::Old,
            2,
            vec![sent('1', "why", 1)],
        ),
        thread(
            'b',
            "src/new_name.rs",
            LinkSide::New,
            2,
            vec![sent('2', "ok", 1)],
        ),
    ]);
    give(&mut stage, &reviews, &mut cx);
    let all = rows(&pane, &mut cx);
    let removed = row_of(&pane, &renamed, Row::Removed { old: 1 }, &mut cx);
    let added = row_of(&pane, &renamed, Row::Added { new: 1 }, &mut cx);
    assert_eq!(
        all[removed + 1],
        PaneRow::Thread {
            thread: thread_id('a')
        }
    );
    assert_eq!(
        all[added + 1],
        PaneRow::Thread {
            thread: thread_id('b')
        }
    );
    // And the reviewer on the old line has that thread under their keys.
    walk_to(&old, "k", &pane, &mut cx);
    cx.simulate_keystrokes("m");
    assert_eq!(position(&pane, &mut cx), new);
    cx.simulate_keystrokes("shift-m");
    assert_eq!(position(&pane, &mut cx), old);
}

#[gpui_kit::test]
fn the_keys_of_the_pane_typed_in_the_box_are_text_and_run_no_action(cx: &mut TestAppContext) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let added = Position {
        path: lib(),
        side: Side::New,
        line: 10,
    };
    walk_to(&added, "j", &pane, &mut cx);
    let opened = comment_on(&added, &pane, &mut cx);

    // `c` would open a box, `d d` delete a draft, `j` move: in the box they are letters.
    for keys in ["c", "d d", "j"] {
        cx.simulate_keystrokes(keys);
    }
    let shown = comment_box(&pane, &mut cx).expect("the box is open");
    assert_eq!(shown.text, "cddj");
    assert_eq!(shown.comment, opened.comment);
    assert_eq!(position(&pane, &mut cx), added);
    // Each letter was a save of this draft, and no other request was made.
    for text in ["c", "cd", "cdd", "cddj"] {
        let asked = stage.keeper.expect(new_thread_save(&opened, text), &mut cx);
        stage.keeper.reply(asked, Ok(Reply::Done));
    }
}

#[gpui_kit::test]
fn a_draft_typed_while_the_reviews_are_on_their_way_is_still_there_when_they_come(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let added = Position {
        path: lib(),
        side: Side::New,
        line: 10,
    };
    walk_to(&added, "j", &pane, &mut cx);
    let opened = comment_on(&added, &pane, &mut cx);
    wait_until(&mut cx, "the first reviews", |_| {
        stage.keeper.reviews_asked() == 1
    });

    // The Keeper is slow: it was asked for the reviews, and two keys follow before it
    // answers. Its answer, which holds no draft, is of what it knew when it was asked.
    stage.keeper.hold_reviews();
    stage
        .keeper
        .tell(Fact::ReviewsChanged { workspace: id('1') });
    wait_until(&mut cx, "the question", |_| {
        stage.keeper.reviews_asked() == 2
    });
    cx.simulate_input("h");
    cx.simulate_input("i");
    for text in ["h", "hi"] {
        let asked = stage.keeper.expect(new_thread_save(&opened, text), &mut cx);
        stage.keeper.reply(asked, Ok(Reply::Done));
    }
    let mut answer = reviews_with(Vec::new());
    answer.viewed = vec![PathBuf::from("src/lib.rs")];
    stage.keeper.set_reviews(answer);
    stage.keeper.release_reviews();
    wait_until(&mut cx, "the late answer", |cx| {
        pane.read_with(cx, |pane, _| {
            pane.reviews().is_some_and(|r| !r.viewed.is_empty())
        })
    });
    // The answer is shown, and the text typed since it was asked for is over it.
    let held = pane
        .read_with(&cx, |pane, _| pane.review().cloned())
        .expect("a review");
    assert_eq!(held.threads.len(), 1);
    assert_eq!(held.threads[0].id, opened.thread);
    assert_eq!(
        held.threads[0].comments,
        [draft_with(&opened.comment, "hi")]
    );

    // A question asked after the saves has them in its answer: the Keeper's text counts
    // then, and the pane does not make the draft twice. The box is not touched by it.
    let kept = reviews_with(vec![Thread {
        comments: vec![draft_with(&opened.comment, "h")],
        id: opened.thread.clone(),
        anchor: Anchor {
            tree: Some(COMMIT.to_owned()),
            ..opened.anchor.clone().expect("an anchor")
        },
        resolved: false,
        addressed: false,
    }]);
    give(&mut stage, &kept, &mut cx);
    assert_eq!(
        comment_box(&pane, &mut cx).map(|shown| shown.text),
        Some("hi".to_owned())
    );
    // The next key is saved whole, and the pane holds it over the answer.
    cx.simulate_input("!");
    let asked = stage
        .keeper
        .expect(new_thread_save(&opened, "hi!"), &mut cx);
    stage.keeper.reply(asked, Ok(Reply::Done));
    let held = pane
        .read_with(&cx, |pane, _| pane.review().cloned())
        .expect("a review");
    assert_eq!(held.threads.len(), 1);
    assert_eq!(
        held.threads[0].comments,
        [draft_with(&opened.comment, "hi!")]
    );
}

#[gpui_kit::test]
fn d_d_deletes_the_draft_on_the_reviewers_line_and_does_nothing_where_there_is_none(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let reviews = reviews_with(vec![
        thread(
            'a',
            "src/lib.rs",
            LinkSide::Old,
            5,
            vec![draft('1', "to delete")],
        ),
        thread(
            'b',
            "src/lib.rs",
            LinkSide::New,
            11,
            vec![sent('2', "sent", 1)],
        ),
    ]);
    give(&mut stage, &reviews, &mut cx);

    // On the line of the draft: the request, and the Keeper's refusal is said.
    assert_eq!(
        position(&pane, &mut cx),
        Position {
            path: lib(),
            side: Side::Old,
            line: 4
        }
    );
    cx.simulate_keystrokes("d d");
    let asked = stage.keeper.expect(
        Request::DeleteDraft {
            review: review_id(),
            comment: comment_id('1'),
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Err(Refusal::NotADraft {
            comment: comment_id('1'),
        }),
    );
    wait_until(&mut cx, "the notice", |cx| {
        pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(drawn("review-notice", &mut cx).is_some());

    // The Keeper deletes it and tells: the thread leaves the rows.
    let gone = reviews_with(vec![reviews.reviews[0].threads[1].clone()]);
    give(&mut stage, &gone, &mut cx);
    assert!(!drawn_thread('a', &mut cx));

    // On a line with a sent comment only, and on one with no thread, nothing is asked.
    let added = Position {
        path: lib(),
        side: Side::New,
        line: 10,
    };
    walk_to(&added, "j", &pane, &mut cx);
    cx.simulate_keystrokes("d d");
    assert!(notice(&pane, &mut cx).is_some());
    cx.simulate_keystrokes("k");
    cx.simulate_keystrokes("d d");
    assert!(notice(&pane, &mut cx).is_some());
    // The next request of the Window is the first save of a comment, not a deletion.
    cx.simulate_keystrokes("c");
    cx.simulate_input("a");
    let (_, next) = stage.keeper.request(&mut cx);
    assert!(matches!(next, Request::SaveDraft { .. }), "{next:?}");
}

#[gpui_kit::test]
fn a_save_the_keeper_refuses_is_said_in_the_notice(cx: &mut TestAppContext) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let added = Position {
        path: lib(),
        side: Side::New,
        line: 10,
    };
    walk_to(&added, "j", &pane, &mut cx);
    let opened = comment_on(&added, &pane, &mut cx);
    cx.simulate_input("a");
    let asked = stage.keeper.expect(new_thread_save(&opened, "a"), &mut cx);
    stage.keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "the drive is full".to_owned(),
        }),
    );
    wait_until(&mut cx, "the notice", |cx| {
        pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(
        notice(&pane, &mut cx)
            .expect("a notice")
            .contains("the drive is full")
    );
    // The box stays open, with the text typed, and the pane works on.
    assert_eq!(
        comment_box(&pane, &mut cx).map(|shown| shown.text),
        Some("a".to_owned())
    );
}

#[gpui_kit::test]
fn another_workspace_on_screen_shows_its_own_reviews_and_closes_the_box_of_the_one_before(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) =
        open_on_two_files_of(vec![workspace('1', "feature"), workspace('2', "other")], cx);
    let pane = stage.pane.clone();
    let reviews = reviews_with(vec![thread(
        'a',
        "src/lib.rs",
        LinkSide::Old,
        5,
        vec![sent('1', "of workspace one", 1)],
    )]);
    give(&mut stage, &reviews, &mut cx);
    let removed = Position {
        path: lib(),
        side: Side::Old,
        line: 4,
    };
    assert_eq!(position(&pane, &mut cx), removed);
    cx.simulate_keystrokes("o");
    assert!(comment_box(&pane, &mut cx).is_some());

    // Workspace 2 comes on screen: nothing of workspace 1 stays, the box included, until
    // the Keeper answers for 2.
    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    cx.run_until_parked();
    let (shown, held, boxed) = pane.read_with(&cx, |pane, _| {
        (
            pane.reviews().cloned(),
            pane.review().cloned(),
            pane.comment_box(),
        )
    });
    assert_eq!((shown, held, boxed), (None, None, None));
    assert!(!drawn_thread('a', &mut cx));

    let mut changes = since_main(Vec::new(), vec![file("src/lib.rs", FileStatus::Modified)]);
    changes.workspace = id('2');
    stage
        .keeper
        .show(&changes, |_| texts(LIB_OLD, LIB_NEW), &mut cx);
    wait_until(&mut cx, "the reviews of workspace 2", |cx| {
        pane.read_with(cx, |pane, _| {
            pane.reviews().is_some_and(|r| r.workspace == id('2'))
        })
    });
    let held = pane
        .read_with(&cx, |pane, _| pane.review().cloned())
        .expect("a review");
    assert!(held.threads.is_empty());
}

/// What is on the clipboard of the test platform, as text.
fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui_kit::test]
fn the_copy_control_puts_the_review_as_text_on_the_clipboard_and_the_copied_comments_show_as_sent(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    // Something of the test's own is on the clipboard: only the copy may replace it.
    cx.write_to_clipboard(ClipboardItem::new_string("before".to_owned()));
    let drafts = reviews_with(vec![
        thread(
            'a',
            "src/lib.rs",
            LinkSide::Old,
            5,
            vec![draft('1', "why remove this")],
        ),
        thread(
            'b',
            "src/lib.rs",
            LinkSide::New,
            11,
            vec![draft('2', "and this")],
        ),
    ]);
    give(&mut stage, &drafts, &mut cx);
    assert!(drawn_comment('1', "draft", &mut cx) && drawn_comment('2', "draft", &mut cx));

    // The key asks the Keeper to copy the review of the pane. Nothing is on the clipboard
    // before the Keeper has answered.
    cx.simulate_keystrokes("y");
    let asked = stage.keeper.expect(
        Request::CopyReview {
            review: review_id(),
        },
        &mut cx,
    );
    cx.run_until_parked();
    assert_eq!(clipboard(&mut cx).as_deref(), Some("before"));

    // The Keeper's text, whatever it is, is what lands on the clipboard.
    let text = "Answer each thread by its id.\n\n## Thread a\nwhy remove this\n";
    stage.keeper.reply(
        asked,
        Ok(Reply::Copied {
            text: text.to_owned(),
            round: 1,
        }),
    );
    wait_until(&mut cx, "the review on the clipboard", |cx| {
        clipboard(cx).as_deref() == Some(text)
    });
    // The round is the Keeper's to name: until it tells, the comments are drafts still.
    assert!(drawn_comment('1', "draft", &mut cx) && drawn_comment('2', "draft", &mut cx));
    assert_eq!(notice(&pane, &mut cx), None);

    // The Keeper tells that the reviews changed and holds both comments at round 1.
    let sent_in_one = reviews_with(vec![
        thread(
            'a',
            "src/lib.rs",
            LinkSide::Old,
            5,
            vec![sent('1', "why remove this", 1)],
        ),
        thread(
            'b',
            "src/lib.rs",
            LinkSide::New,
            11,
            vec![sent('2', "and this", 1)],
        ),
    ]);
    give(&mut stage, &sent_in_one, &mut cx);
    let shown = pane.read_with(&cx, |pane, _| pane.review().cloned());
    assert_eq!(shown, Some(sent_in_one.reviews[0].clone()));
    for comment in ['1', '2'] {
        assert!(drawn_comment(comment, "round-1", &mut cx));
        assert!(!drawn_comment(comment, "draft", &mut cx));
    }

    // Nothing is left to send: the Keeper refuses, the clipboard stays as it was, and the
    // notice says so.
    cx.simulate_keystrokes("y");
    let asked = stage.keeper.expect(
        Request::CopyReview {
            review: review_id(),
        },
        &mut cx,
    );
    stage.keeper.reply(
        asked,
        Err(Refusal::NothingToSend {
            review: review_id(),
        }),
    );
    wait_until(&mut cx, "the notice", |cx| {
        pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(drawn("review-notice", &mut cx).is_some());
    assert_eq!(clipboard(&mut cx).as_deref(), Some(text));
}

#[gpui_kit::test]
fn copying_closes_the_box_whose_draft_the_copy_sends(cx: &mut TestAppContext) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let added = Position {
        path: lib(),
        side: Side::New,
        line: 10,
    };
    walk_to(&added, "j", &pane, &mut cx);
    let opened = comment_on(&added, &pane, &mut cx);
    cx.simulate_input("a");
    let asked = stage.keeper.expect(new_thread_save(&opened, "a"), &mut cx);
    stage.keeper.reply(asked, Ok(Reply::Done));
    // A click on a row gives the keys of the pane the focus; the box stays open.
    let row = bounds_of_row(2, &mut cx).center();
    cx.simulate_click(row, Modifiers::none());
    assert!(comment_box(&pane, &mut cx).is_some());

    cx.simulate_keystrokes("y");
    // The save came first, so the copy has it; a sent comment is not edited, so more
    // typing into the box would only be refused.
    stage.keeper.expect(
        Request::CopyReview {
            review: review_id(),
        },
        &mut cx,
    );
    assert_eq!(comment_box(&pane, &mut cx), None);
    assert!(drawn("review-comment-box", &mut cx).is_none());
}

#[gpui_kit::test]
fn the_copy_of_a_pane_with_no_reviews_yet_asks_nothing_and_says_so(cx: &mut TestAppContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    // The changes are not here, so the reviews were never asked for.
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.reviews().cloned()),
        None
    );
    let focus = stage.focus.clone();
    cx.update(|window, cx| focus.focus(window, cx));
    cx.simulate_keystrokes("y");
    assert!(notice(&stage.pane, &mut cx).is_some());
    // The next thing the Window asks is the changes, not a copy.
    let (_, next) = stage.keeper.request(&mut cx);
    assert!(matches!(next, Request::GetChanges { .. }), "{next:?}");
}

#[gpui_kit::test]
fn r_marks_the_file_the_reviewer_is_in_viewed_and_a_second_r_takes_the_mark_away(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let heading = |path: &PathBuf, cx: &mut VisualTestContext| {
        rows(&pane, cx)
            .iter()
            .position(|row| matches!(row, PaneRow::File { path: at, .. } if at == path))
            .expect("the heading of the file")
    };
    let (lib_heading, long_heading) = (heading(&lib(), &mut cx), heading(&long_path(), &mut cx));
    let viewed = |index: usize, cx: &mut VisualTestContext| {
        drawn(&format!("review-viewed-{index}"), cx).is_some()
    };
    assert!(!viewed(lib_heading, &mut cx) && !viewed(long_heading, &mut cx));

    // The reviewer is in lib.rs: the file is marked for the workspace on screen.
    cx.simulate_keystrokes("r");
    let asked = stage.keeper.expect(
        Request::SetViewed {
            workspace: id('1'),
            path: lib(),
            viewed: true,
        },
        &mut cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Done));
    // Nothing is drawn until the Keeper tells the reviews changed.
    cx.run_until_parked();
    assert!(!viewed(lib_heading, &mut cx));
    let mut marked = reviews_with(Vec::new());
    marked.viewed = vec![lib()];
    give(&mut stage, &marked, &mut cx);
    assert!(viewed(lib_heading, &mut cx));
    assert!(!viewed(long_heading, &mut cx));

    // A second `r` on the same file takes the mark away.
    cx.simulate_keystrokes("r");
    let asked = stage.keeper.expect(
        Request::SetViewed {
            workspace: id('1'),
            path: lib(),
            viewed: false,
        },
        &mut cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Done));
    give(&mut stage, &reviews_with(Vec::new()), &mut cx);
    assert!(!viewed(lib_heading, &mut cx));

    // In another file `r` marks that one, whatever else is marked.
    give(&mut stage, &marked, &mut cx);
    let in_long = Position {
        path: long_path(),
        side: Side::Old,
        line: 197,
    };
    walk_to(&in_long, "j", &pane, &mut cx);
    cx.simulate_keystrokes("r");
    let asked = stage.keeper.expect(
        Request::SetViewed {
            workspace: id('1'),
            path: long_path(),
            viewed: true,
        },
        &mut cx,
    );
    // The Keeper refuses: the notice says why, and the heading is not marked.
    stage.keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "the worktree is gone".to_owned(),
        }),
    );
    wait_until(&mut cx, "the notice", |cx| {
        pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(
        notice(&pane, &mut cx)
            .expect("a notice")
            .contains("the worktree is gone")
    );
    assert!(!viewed(long_heading, &mut cx));
}

#[gpui_kit::test]
fn shift_r_marks_the_change_reviewed_and_the_comparison_row_shows_it_once_the_keeper_tells(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    assert!(drawn("review-reviewed", &mut cx).is_none());

    cx.simulate_keystrokes("shift-r");
    let asked = stage.keeper.expect(
        Request::MarkReviewed {
            review: review_id(),
        },
        &mut cx,
    );
    stage.keeper.reply(asked, Ok(Reply::Done));
    cx.run_until_parked();
    assert!(drawn("review-reviewed", &mut cx).is_none());

    let mut reviewed = reviews_with(Vec::new());
    reviewed.reviews[0].reviewed = Some("7".repeat(40));
    give(&mut stage, &reviewed, &mut cx);
    assert!(drawn("review-reviewed", &mut cx).is_some());
    assert_eq!(
        pane.read_with(&cx, |pane, _| pane
            .review()
            .and_then(|r| r.reviewed.clone())),
        Some("7".repeat(40))
    );
    // The comparison row is where it is drawn: the first.
    let first = bounds_of_row(0, &mut cx);
    let mark = drawn("review-reviewed", &mut cx).expect("drawn");
    assert!(first.contains(&mark.center()));
}

#[gpui_kit::test]
fn shift_c_opens_a_field_for_a_message_and_enter_commits_with_it(cx: &mut TestAppContext) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let open = |cx: &mut VisualTestContext| pane.read_with(cx, |pane, _| pane.commit_field_open());
    assert!(!open(&mut cx) && drawn("review-commit-field", &mut cx).is_none());

    cx.simulate_keystrokes("shift-c");
    assert!(open(&mut cx));
    assert!(drawn("review-commit-field", &mut cx).is_some());
    // The field has the focus: the letters are its text, and not the keys of the pane,
    // which would open a comment box on `c` and `i`.
    cx.simulate_input("Fix the thing");
    assert_eq!(comment_box(&pane, &mut cx), None);
    assert!(open(&mut cx));
    cx.simulate_keystrokes("enter");
    let asked = stage.keeper.expect(
        Request::Commit {
            workspace: id('1'),
            message: "Fix the thing".to_owned(),
        },
        &mut cx,
    );
    assert!(!open(&mut cx));
    assert!(drawn("review-commit-field", &mut cx).is_none());

    // Nothing was uncommitted: the notice says so.
    stage
        .keeper
        .reply(asked, Err(Refusal::NothingToCommit { workspace: id('1') }));
    wait_until(&mut cx, "the notice", |cx| {
        pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(drawn("review-notice", &mut cx).is_some());

    // The keys of the pane work again: the focus came back to the rows.
    cx.simulate_keystrokes("c");
    assert!(comment_box(&pane, &mut cx).is_some());
}

#[gpui_kit::test]
fn a_commit_that_is_made_says_nothing_and_one_git_refused_says_why(cx: &mut TestAppContext) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let commit_with = |message: &str, cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("shift-c");
        cx.simulate_input(message);
        cx.simulate_keystrokes("enter");
    };
    let message = |text: &str| Request::Commit {
        workspace: id('1'),
        message: text.to_owned(),
    };

    commit_with("One", &mut cx);
    let asked = stage.keeper.expect(message("One"), &mut cx);
    stage.keeper.reply(
        asked,
        Ok(Reply::Committed {
            commit: "c".repeat(40),
        }),
    );
    cx.run_until_parked();
    assert_eq!(notice(&pane, &mut cx), None);

    commit_with("Two", &mut cx);
    let asked = stage.keeper.expect(message("Two"), &mut cx);
    stage.keeper.reply(
        asked,
        Err(Refusal::Failed {
            reason: "pre-commit hook failed: lint".to_owned(),
        }),
    );
    wait_until(&mut cx, "the notice", |cx| {
        pane.read_with(cx, |pane, _| pane.notice().is_some())
    });
    assert!(
        notice(&pane, &mut cx)
            .expect("a notice")
            .contains("pre-commit hook failed: lint")
    );
}

#[gpui_kit::test]
fn escape_closes_the_commit_field_and_asks_nothing_and_a_blank_message_keeps_it_open(
    cx: &mut TestAppContext,
) {
    let (mut stage, _changes, mut cx) = open_on_two_files(cx);
    let pane = stage.pane.clone();
    let open = |cx: &mut VisualTestContext| pane.read_with(cx, |pane, _| pane.commit_field_open());

    cx.simulate_keystrokes("shift-c");
    cx.simulate_input("Not this");
    cx.simulate_keystrokes("escape");
    assert!(!open(&mut cx));
    assert!(drawn("review-commit-field", &mut cx).is_none());

    // A field with nothing but blanks stays open: git would refuse the message.
    cx.simulate_keystrokes("shift-c");
    cx.simulate_input("  ");
    cx.simulate_keystrokes("enter");
    assert!(open(&mut cx));
    assert!(notice(&pane, &mut cx).is_some());
    // The key again keeps the field, and its text.
    cx.simulate_keystrokes("escape");

    // Neither asked the Keeper for anything: the next request is a comment's first save.
    cx.simulate_keystrokes("c");
    cx.simulate_input("a");
    let (_, next) = stage.keeper.request(&mut cx);
    assert!(matches!(next, Request::SaveDraft { .. }), "{next:?}");
}

#[gpui_kit::test]
fn another_workspace_on_screen_closes_the_commit_field_of_the_one_before(cx: &mut TestAppContext) {
    let (stage, _changes, mut cx) =
        open_on_two_files_of(vec![workspace('1', "feature"), workspace('2', "other")], cx);
    let pane = stage.pane.clone();
    cx.simulate_keystrokes("shift-c");
    cx.simulate_input("For workspace one");
    assert!(pane.read_with(&cx, |pane, _| pane.commit_field_open()));

    stage
        .models
        .workspaces
        .update(&mut cx, |list, cx| list.select(&id('2'), cx));
    cx.run_until_parked();
    // A commit goes to the workspace the field was opened on, so it is not kept for
    // the one that came.
    assert!(!pane.read_with(&cx, |pane, _| pane.commit_field_open()));
    assert!(drawn("review-commit-field", &mut cx).is_none());
}
