//! Component test: the comment box and the threads of the Review pane on GPUI's test
//! platform, with no window on any screen. Keys, clicks and the wheel go in; the pane is
//! read back through the model it shows. The test plays the Keeper on the other end of a
//! socket pair: it answers what the Window asks for the changes and for each file, and
//! the helper of `common` answers the reviews.

mod common;

use std::path::PathBuf;

use gpui_kit::{
    Bounds, Entity, Modifiers, Pixels, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext,
    point, px,
};
use yaar_diff::{Row, Side};
use yaar_link::{
    Anchor, Author, Changes, Comment, CommentId, DraftOn, Fact, FileStatus, Lines, Reply, Request,
    RequestId, Review, Round, SentBy, Side as LinkSide, Thread, ThreadId, WorkspaceReviews,
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
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
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
