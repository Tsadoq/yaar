//! Component test: the read-only file view of the review pane, on GPUI's test platform. Keys,
//! clicks and the wheel go in; the pane is read back through the model it shows. The test
//! plays the Keeper on the other end of a socket pair and the Navigator at the pane's port:
//! it reads the asks the pane really makes and tells answers in the shapes `port.rs`
//! defines, and computes nothing the Navigator computes.
//!
//! A name's place on screen is read as `tests/navigate.rs` reads it: the text of a line is
//! drawn by an element whose selector is `review-text-<row>-<side>:<text>`, the font is of
//! fixed width, and the middle of the character at offset `k` is `left + (k + 0.5) * cell`.
//!
//! A negative clause ("is not opened", "asks nothing") is read after a barrier: an ask the
//! test causes after it, which the pane makes through the same port, so that it arrives
//! after everything the pane asked before it.

mod common;

use std::path::PathBuf;

use gpui_kit::{
    Entity, Modifiers, Pixels, Point, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext,
    point, px,
};
use yaar_diff::{Row, Side};
use yaar_link::{
    Anchor, Author, Comment, CommentId, DraftOn, Fact, FileStatus, Lines, Request, Review,
    Side as LinkSide, Thread, ThreadId, ThreadState,
};
use yaar_navigator::{
    Answered, Ask, FileKind, Language, Location, OpenedFile, Place, ServerState, State, Told, Tree,
};
use yaar_review_pane::navigate::GUESSED;
use yaar_review_pane::{FileShown, PaneRow, Position, ReferencesShown, ReviewPane};

use common::navigator::NavigatorEnd;
use common::{
    Stage, bounds_of_row, drawn, file, id, no_agent_review, open, review_id, reviews_of,
    since_main, texts, wait_for_files, wait_until, workspace,
};

fn workspace_id() -> yaar_link::WorkspaceId {
    id('1')
}

/// The text of the file the diff is of: 120 lines `let v<n> = <n>;`, each tenth from the
/// fifth on rewritten in the new version, so that the diff is long and has many changes.
fn old_text() -> String {
    (1..=120)
        .map(|n| format!("    let v{n} = {n};\n"))
        .collect()
}

fn new_text() -> String {
    (1..=120)
        .map(|n| match n % 10 {
            5 => format!("    let w{n} = compute({n});\n"),
            _ => format!("    let v{n} = {n};\n"),
        })
        .collect()
}

/// A file of this many lines, `line 1` to `line <lines>`, as the Navigator opens it.
fn opened_of(kind: FileKind, path: &str, lines: usize) -> OpenedFile {
    OpenedFile {
        path: PathBuf::from(path),
        kind,
        text: (1..=lines).map(|n| format!("line {n}\n")).collect(),
    }
}

fn in_the_worktree(relative: &str) -> FileKind {
    FileKind::Worktree {
        workspace: workspace_id(),
        relative: PathBuf::from(relative),
    }
}

fn location(path: &str, line: u32) -> Location {
    Location {
        path: PathBuf::from(path),
        line,
        column: 0,
        text: format!("line {line}"),
    }
}

fn rust() -> Language {
    Language {
        name: "rust".to_owned(),
        title: "Rust".to_owned(),
        extensions: vec!["rs".to_owned()],
    }
}

/// Opens the pane on the long change of `src/lib.rs`, connects the Navigator, waits for
/// the reviews, and scrolls down so that the first row is out of view.
fn open_long(cx: &mut TestAppContext) -> (Stage, NavigatorEnd, VisualTestContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(Vec::new(), vec![file("src/lib.rs", FileStatus::Modified)]);
    stage
        .keeper
        .show(&changes, |_| texts(&old_text(), &new_text()), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    wait_until(&mut cx, "the reviews", |cx| {
        stage.pane.read_with(cx, |pane, _| pane.review().is_some())
    });
    let navigator = NavigatorEnd::connect(&stage.models, &mut cx);
    navigator.tell(Told::Languages(vec![rust()]), &mut cx);
    let over_the_rows = bounds_of_row(3, &mut cx).center();
    cx.simulate_event(ScrollEvent {
        position: over_the_rows,
        delta: ScrollDelta::Pixels(point(px(0.), px(-900.))),
        ..ScrollEvent::default()
    });
    cx.run_until_parked();
    (stage, navigator, cx)
}

/// Clicks the name `compute` on the first added line in view.
fn click_compute(stage: &Stage, cx: &mut VisualTestContext) -> (usize, String) {
    let (row, text) = stage.pane.read_with(cx, |pane, _| {
        let in_view = pane.rows_in_view();
        let row = in_view
            .clone()
            .find(|&index| {
                matches!(
                    pane.rows()[index],
                    PaneRow::Diff {
                        row: Row::Added { .. },
                        ..
                    }
                )
            })
            .expect("an added line is in view");
        let PaneRow::Diff {
            row: Row::Added { new },
            path,
        } = pane.rows()[row].clone()
        else {
            unreachable!()
        };
        let text = pane
            .text(&Position {
                path,
                side: Side::New,
                line: new,
            })
            .expect("the text of the line");
        (row, text)
    });
    let at = name_point(row, "new", &text, "compute", cx);
    cx.simulate_click(at, Modifiers::none());
    (row, text)
}

/// Where the first byte of `name` in `text` is drawn, in the element of the text of the
/// row, by the rule in the header.
fn name_point(
    row: usize,
    side: &str,
    text: &str,
    name: &str,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let selector = format!("review-text-{row}-{side}:{text}");
    let bounds = drawn(&selector, cx).unwrap_or_else(|| panic!("{selector} is not drawn"));
    let chars = text.chars().count() as f32;
    let cell = bounds.size.width / chars;
    let offset = text[..text.find(name).expect("the line holds the name")]
        .chars()
        .count() as f32;
    point(bounds.left() + cell * (offset + 0.5), bounds.center().y)
}

fn file_view(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<FileShown> {
    pane.read_with(cx, |pane, _| pane.file_view().cloned())
}

fn references(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<ReferencesShown> {
    pane.read_with(cx, |pane, _| pane.references().cloned())
}

fn position(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<Position> {
    pane.read_with(cx, |pane, _| pane.position().cloned())
}

fn name(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<String> {
    pane.read_with(cx, |pane, _| pane.name_at())
}

/// Presses `g d`, which asks the definition; the Navigator tells these locations; when the
/// reviewer is sent to one, the pane asks for its file, which the Navigator tells.
fn go_to_definition(
    locations: Vec<Location>,
    file: Option<OpenedFile>,
    navigator: &mut NavigatorEnd,
    cx: &mut VisualTestContext,
) -> Place {
    cx.simulate_keystrokes("g d");
    let Ask::Definition { id, place } = navigator.ask(cx) else {
        panic!("the definition was not asked")
    };
    navigator.tell(
        Told::Definition {
            id,
            answer: Answered {
                value: locations.clone(),
                guessed: false,
            },
        },
        cx,
    );
    if let Some(file) = file {
        let Ask::Open { id, location } = navigator.ask(cx) else {
            panic!("the file was not asked for")
        };
        assert_eq!(location, locations[0]);
        navigator.tell(Told::Opened { id, file: Ok(file) }, cx);
    }
    place
}

/// Whether any row of the diff is drawn.
fn a_row_is_drawn(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> bool {
    let rows = pane.read_with(cx, |pane, _| pane.rows().len());
    (0..rows).any(|index| drawn(&format!("review-row-{index}"), cx).is_some())
}

// RP-22
#[gpui_kit::test]
fn going_to_a_definition_opens_the_file_and_the_return_control_brings_the_reviewer_back(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_long(cx);
    let pane = &stage.pane;
    let (row, _) = click_compute(&stage, &mut cx);
    assert!(pane.read_with(&cx, |pane, _| pane.rows_in_view().start) > 0);
    let before = (
        position(pane, &mut cx),
        name(pane, &mut cx),
        common::bounds_of_row(row, &mut cx),
        pane.read_with(&cx, |pane, _| pane.rows_in_view()),
    );
    assert_eq!(before.1.as_deref(), Some("compute"));

    let target = location("/gits/feature/src/lib.rs", 12);
    let opened = opened_of(
        in_the_worktree("src/lib.rs"),
        "/gits/feature/src/lib.rs",
        30,
    );
    for back in ["escape", "the control"] {
        // `g d` asks the definition of the name; the one place told is opened.
        let asked = go_to_definition(
            vec![target.clone()],
            Some(opened.clone()),
            &mut navigator,
            &mut cx,
        );
        assert_eq!(asked.tree, Tree::Worktree);
        assert_eq!(asked.name, "compute");

        // The target is in a file view, in the place of the rows, at its line.
        assert_eq!(
            file_view(pane, &mut cx),
            Some(FileShown {
                file: opened.clone(),
                line: 12
            })
        );
        assert!(drawn("review-file-view", &mut cx).is_some());
        assert!(!a_row_is_drawn(pane, &mut cx), "a row of the diff is drawn");
        let at = drawn("review-file-at", &mut cx).expect("the reviewer's line is marked");
        let line = drawn("review-file-line-12", &mut cx).expect("line 12 is drawn");
        assert!(line.contains(&at.center()), "{at:?} is not in {line:?}");
        assert!(drawn("review-file-text-12:line 12", &mut cx).is_some());
        assert!(drawn("review-file-path:src/lib.rs", &mut cx).is_some());
        assert!(drawn("review-file-back", &mut cx).is_some());
        assert!(
            drawn(
                &format!("review-file-back:{}", yaar_review_pane::file_view::BACK),
                &mut cx
            )
            .is_some()
        );

        // The return control brings the reviewer back to where they were, as they were.
        match back {
            "escape" => cx.simulate_keystrokes("escape"),
            _ => {
                let control = drawn("review-file-back", &mut cx)
                    .expect("the control is drawn")
                    .center();
                cx.simulate_click(control, Modifiers::none());
            }
        }
        assert_eq!(file_view(pane, &mut cx), None, "back by {back}");
        assert_eq!(position(pane, &mut cx), before.0);
        assert_eq!(name(pane, &mut cx), before.1);
        assert_eq!(pane.read_with(&cx, |pane, _| pane.rows_in_view()), before.3);
        assert_eq!(common::bounds_of_row(row, &mut cx), before.2);
        assert!(drawn("review-file-view", &mut cx).is_none());
        cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    }

    // A definition followed from inside the file view opens the next, and back is to the
    // diff, not to the first file view.
    go_to_definition(vec![target.clone()], Some(opened), &mut navigator, &mut cx);
    cx.simulate_keystrokes("j");
    assert_eq!(file_view(pane, &mut cx).map(|shown| shown.line), Some(13));
    cx.simulate_keystrokes("k k");
    assert_eq!(file_view(pane, &mut cx).map(|shown| shown.line), Some(11));
    cx.simulate_keystrokes("j");
    let next = opened_of(
        in_the_worktree("src/other.rs"),
        "/gits/feature/src/other.rs",
        10,
    );
    let asked = go_to_definition(
        vec![location("/gits/feature/src/other.rs", 3)],
        Some(next.clone()),
        &mut navigator,
        &mut cx,
    );
    // The name asked is the one of the line of the file view, in that file.
    assert_eq!(asked.path, PathBuf::from("src/lib.rs"));
    assert_eq!(asked.line, 12);
    assert_eq!(asked.name, "line");
    assert_eq!(
        file_view(pane, &mut cx),
        Some(FileShown {
            file: next,
            line: 3
        })
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(file_view(pane, &mut cx), None);
    assert_eq!(position(pane, &mut cx), before.0);
    // The name is the one the reviewer left in the diff, not the one the first file view
    // had: the place left is kept by the first file view and by no other.
    assert_eq!(name(pane, &mut cx), before.1);
    assert_eq!(pane.read_with(&cx, |pane, _| pane.rows_in_view()), before.3);
    assert_eq!(common::bounds_of_row(row, &mut cx), before.2);
}

// RP-22, the way back when the rows changed while the reviewer was away
#[gpui_kit::test]
fn back_leaves_the_reviewers_row_where_it_was_on_screen_when_threads_came_above_it_meanwhile(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut navigator, mut cx) = open_long(cx);
    let (row, _) = click_compute(&stage, &mut cx);
    let clicked = stage
        .pane
        .read_with(&cx, |pane, _| pane.rows()[row].clone());
    let before = common::bounds_of_row(row, &mut cx);

    go_to_definition(
        vec![location("/gits/feature/src/lib.rs", 12)],
        Some(opened_of(
            in_the_worktree("src/lib.rs"),
            "/gits/feature/src/lib.rs",
            30,
        )),
        &mut navigator,
        &mut cx,
    );
    assert!(file_view(&stage.pane, &mut cx).is_some());

    // While the file view is open the Keeper tells a thread on a line far above what the
    // reviewer left in view: its rows come into the diff above it.
    let mut above = thread('4', anchor(None, "src/lib.rs", LinkSide::New, 2));
    above.anchor.text = "    let v2 = 2;".to_owned();
    let rows_before = stage.pane.read_with(&cx, |pane, _| pane.rows().len());
    let review = Review {
        threads: vec![above.clone()],
        ..no_agent_review()
    };
    stage.keeper.set_reviews(reviews_of(vec![review]));
    stage.keeper.tell(Fact::ReviewsChanged {
        workspace: workspace_id(),
    });
    wait_until(&mut cx, "the thread above, in the rows", |cx| {
        stage
            .pane
            .read_with(cx, |pane, _| pane.rows().len() > rows_before)
    });
    // The file view is still what is shown.
    assert!(file_view(&stage.pane, &mut cx).is_some());

    cx.simulate_keystrokes("escape");
    let now = stage.pane.read_with(&cx, |pane, _| {
        pane.rows().iter().position(|held| *held == clicked)
    });
    let now = now.expect("the row the reviewer left is still among the rows");
    assert!(now > row, "the thread's rows came above it");
    // The reviewer's row is drawn where it was: the view went down by the rows that came.
    assert_eq!(common::bounds_of_row(now, &mut cx), before);
    assert_eq!(name(&stage.pane, &mut cx).as_deref(), Some("compute"));
}

// RP-22, several definitions
#[gpui_kit::test]
fn several_definitions_are_listed_and_not_opened(cx: &mut TestAppContext) {
    let (stage, mut navigator, mut cx) = open_long(cx);
    let pane = &stage.pane;
    click_compute(&stage, &mut cx);
    let two = vec![
        location("/gits/feature/src/a.rs", 3),
        location("/gits/feature/src/b.rs", 9),
    ];
    go_to_definition(two.clone(), None, &mut navigator, &mut cx);
    let listed = references(pane, &mut cx).expect("the definitions are listed");
    assert_eq!(listed.locations, two);
    assert!(listed.definitions && !listed.guessed && !listed.waiting);
    assert!(drawn("review-references-title:Definitions of compute", &mut cx).is_some());
    assert_eq!(file_view(pane, &mut cx), None);
    // Nothing was asked to open: the list is closed, and the next ask the pane makes, the
    // barrier, is a hover.
    cx.simulate_keystrokes("escape");
    assert_eq!(references(pane, &mut cx), None);
    cx.simulate_keystrokes("shift-k");
    assert!(matches!(navigator.ask(&mut cx), Ask::Hover { .. }));

    // `enter` on a listed definition opens it.
    go_to_definition(two.clone(), None, &mut navigator, &mut cx);
    cx.simulate_keystrokes("j enter");
    let Ask::Open { location, .. } = navigator.ask(&mut cx) else {
        panic!("the file was not asked for")
    };
    assert_eq!(location, two[1]);
}

// A definition found by text match, as of its own: FR-068 asks every such answer marked
#[gpui_kit::test]
fn a_definition_found_by_text_match_is_listed_with_its_mark_even_when_there_is_one(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_long(cx);
    let pane = &stage.pane;
    click_compute(&stage, &mut cx);
    cx.simulate_keystrokes("g d");
    let Ask::Definition { id, .. } = navigator.ask(&mut cx) else {
        panic!("the definition was not asked")
    };
    let only = vec![location("/gits/feature/src/lib.rs", 12)];
    navigator.tell(
        Told::Definition {
            id,
            answer: Answered {
                value: only.clone(),
                guessed: true,
            },
        },
        &mut cx,
    );
    // Listed as references are, with the mark and its words, and no file view hides that
    // it may be wrong.
    let listed = references(pane, &mut cx).expect("the definition is listed");
    assert_eq!(listed.locations, only);
    assert!(listed.guessed && listed.definitions);
    assert!(drawn("review-references-guessed", &mut cx).is_some());
    assert!(
        drawn(
            &format!("review-references-guessed-words:{GUESSED}"),
            &mut cx
        )
        .is_some()
    );
    assert_eq!(file_view(pane, &mut cx), None);
    // The barrier: nothing was asked to open before this hover.
    cx.simulate_keystrokes("escape shift-k");
    assert!(matches!(navigator.ask(&mut cx), Ask::Hover { .. }));
}

// RP-22, a file that cannot be read, and a definition with none
#[gpui_kit::test]
fn a_file_that_cannot_be_opened_and_a_definition_that_is_not_found_are_said_in_the_notice(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_long(cx);
    let pane = &stage.pane;
    click_compute(&stage, &mut cx);
    cx.simulate_keystrokes("g d");
    let Ask::Definition { id, .. } = navigator.ask(&mut cx) else {
        panic!("the definition was not asked")
    };
    navigator.tell(
        Told::Definition {
            id,
            answer: Answered {
                value: Vec::new(),
                guessed: false,
            },
        },
        &mut cx,
    );
    assert_eq!(
        pane.read_with(&cx, |pane, _| pane.notice()).as_deref(),
        Some("No definition of compute was found")
    );

    cx.simulate_keystrokes("g d");
    let Ask::Definition { id, .. } = navigator.ask(&mut cx) else {
        panic!("the definition was not asked")
    };
    navigator.tell(
        Told::Definition {
            id,
            answer: Answered {
                value: vec![location("/elsewhere/binary.bin", 1)],
                guessed: false,
            },
        },
        &mut cx,
    );
    let Ask::Open { id, .. } = navigator.ask(&mut cx) else {
        panic!("the file was not asked for")
    };
    navigator.tell(
        Told::Opened {
            id,
            file: Err("The file is not UTF-8".to_owned()),
        },
        &mut cx,
    );
    assert_eq!(
        pane.read_with(&cx, |pane, _| pane.notice()).as_deref(),
        Some("The file is not UTF-8")
    );
    assert_eq!(file_view(pane, &mut cx), None);
}

/// The anchor a comment on `line` of `text` would have.
fn anchor(tree: Option<&str>, path: &str, side: LinkSide, line: u32) -> Anchor {
    Anchor {
        tree: tree.map(str::to_owned),
        path: PathBuf::from(path),
        side,
        lines: Lines {
            first: line,
            last: line,
        },
        text: format!("line {line}"),
    }
}

/// Presses `c` on the file view's line, types `why`, and holds the draft the Keeper is
/// asked to save to what the row says.
fn comment_and_save(
    wanted: Anchor,
    stage: &mut Stage,
    cx: &mut VisualTestContext,
) -> yaar_review_pane::CommentBox {
    cx.simulate_keystrokes("c");
    let opened = stage
        .pane
        .read_with(cx, |pane, _| pane.comment_box())
        .expect("the comment box opened");
    assert_eq!(opened.anchor.as_ref(), Some(&wanted));
    // Typed a key at a time, as the reviewer does, each key saved at once with the whole
    // text so far.
    for typed in ["w", "h", "y"] {
        cx.simulate_input(typed);
    }
    for text in ["w", "wh", "why"] {
        let asked = stage.keeper.expect(
            Request::SaveDraft {
                review: review_id(),
                thread: opened.thread.clone(),
                comment: opened.comment.clone(),
                on: DraftOn::NewThread {
                    anchor: wanted.clone(),
                },
                text: text.to_owned(),
            },
            cx,
        );
        stage.keeper.reply(asked, Ok(yaar_link::Reply::Done));
    }
    opened
}

// RP-31
#[gpui_kit::test]
fn a_comment_is_written_on_a_line_of_a_file_reached_through_a_definition_or_a_reference(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut navigator, mut cx) = open_long(cx);
    click_compute(&stage, &mut cx);

    // A file of the worktree, at a line no row of the diff shows.
    let shown_lines: Vec<u32> = stage.pane.read_with(&cx, |pane, _| {
        pane.rows()
            .iter()
            .filter_map(|row| match row {
                PaneRow::Diff { row, .. } => row.line(Side::New),
                _ => None,
            })
            .collect()
    });
    assert!(!shown_lines.contains(&19), "line 20 is in the diff");
    let in_worktree = opened_of(
        in_the_worktree("src/lib.rs"),
        "/gits/feature/src/lib.rs",
        30,
    );
    go_to_definition(
        vec![location("/gits/feature/src/lib.rs", 20)],
        Some(in_worktree.clone()),
        &mut navigator,
        &mut cx,
    );
    let wanted = anchor(None, "src/lib.rs", LinkSide::New, 20);
    let opened = comment_and_save(wanted.clone(), &mut stage, &mut cx);
    // The draft is a thread on its line of the file view.
    assert!(drawn(&format!("review-thread-{}", opened.thread), &mut cx).is_some());
    assert!(drawn(&format!("review-comment-{}:draft", opened.comment), &mut cx).is_some());
    // Escape closes the box only: the file view is as it was.
    cx.simulate_keystrokes("escape");
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.comment_box()),
        None
    );
    assert_eq!(
        file_view(&stage.pane, &mut cx).map(|shown| (shown.file, shown.line)),
        Some((in_worktree, 20))
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(file_view(&stage.pane, &mut cx), None);

    // A line that is in the diff as well: the same anchor.
    let in_diff = opened_of(
        in_the_worktree("src/lib.rs"),
        "/gits/feature/src/lib.rs",
        30,
    );
    go_to_definition(
        vec![location("/gits/feature/src/lib.rs", 25)],
        Some(in_diff),
        &mut navigator,
        &mut cx,
    );
    assert!(shown_lines.contains(&24), "line 25 is not in the diff");
    comment_and_save(
        anchor(None, "src/lib.rs", LinkSide::New, 25),
        &mut stage,
        &mut cx,
    );
    cx.simulate_keystrokes("escape escape");
    assert_eq!(file_view(&stage.pane, &mut cx), None);

    // A file reached from the list of references: outside the worktree, and a version.
    let version = "7".repeat(40);
    let cases = [
        (
            FileKind::Outside,
            "/x/lib/dep.rs",
            anchor(None, "/x/lib/dep.rs", LinkSide::New, 4),
        ),
        (
            FileKind::Version {
                workspace: workspace_id(),
                version: version.clone(),
                relative: PathBuf::from("src/old.rs"),
            },
            "/cache/navigator/1/old/1/src/old.rs",
            anchor(Some(&version), "src/old.rs", LinkSide::Old, 4),
        ),
    ];
    for (kind, path, wanted) in cases {
        cx.simulate_keystrokes("g r");
        let Ask::References { id, .. } = navigator.ask(&mut cx) else {
            panic!("the references were not asked")
        };
        navigator.tell(
            Told::References {
                id,
                answer: Answered {
                    value: vec![location(path, 4)],
                    guessed: false,
                },
            },
            &mut cx,
        );
        cx.simulate_keystrokes("enter");
        let Ask::Open { id, .. } = navigator.ask(&mut cx) else {
            panic!("the file was not asked for")
        };
        navigator.tell(
            Told::Opened {
                id,
                file: Ok(opened_of(kind, path, 9)),
            },
            &mut cx,
        );
        assert_eq!(
            file_view(&stage.pane, &mut cx).map(|shown| shown.line),
            Some(4)
        );
        comment_and_save(wanted, &mut stage, &mut cx);
        cx.simulate_keystrokes("escape escape");
        assert_eq!(file_view(&stage.pane, &mut cx), None);
    }
}

fn thread(digit: char, anchor: Anchor) -> Thread {
    Thread {
        id: ThreadId::new(digit.to_string().repeat(32)),
        anchor,
        resolved: false,
        addressed: false,
        comments: vec![Comment {
            id: CommentId::new(digit.to_string().repeat(32)),
            author: Author::Reviewer,
            text: "from the Keeper".to_owned(),
            round: Some(1),
        }],
        state: ThreadState::Sent,
        now: None,
    }
}

// RP-31, the threads the Keeper tells
#[gpui_kit::test]
fn a_thread_of_the_review_is_drawn_on_its_line_of_the_file_view_while_the_line_holds_its_text(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut navigator, mut cx) = open_long(cx);
    click_compute(&stage, &mut cx);
    // A thread on line 20 of `src/lib.rs`, one on line 21 whose text is not the line's any
    // more, and one on a file outside the worktree.
    let state = "9".repeat(40);
    let on_line = thread('1', anchor(Some(&state), "src/lib.rs", LinkSide::New, 20));
    let mut changed = anchor(Some(&state), "src/lib.rs", LinkSide::New, 21);
    changed.text = "what the line said then".to_owned();
    let gone = thread('2', changed);
    let outside = thread('3', anchor(None, "/x/lib/dep.rs", LinkSide::New, 4));
    let review = Review {
        threads: vec![on_line.clone(), gone.clone(), outside.clone()],
        ..no_agent_review()
    };
    stage.keeper.set_reviews(reviews_of(vec![review]));
    stage.keeper.tell(Fact::ReviewsChanged {
        workspace: workspace_id(),
    });
    wait_until(&mut cx, "the threads", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.review().is_some_and(|r| r.threads.len() == 3)
        })
    });

    go_to_definition(
        vec![location("/gits/feature/src/lib.rs", 20)],
        Some(opened_of(
            in_the_worktree("src/lib.rs"),
            "/gits/feature/src/lib.rs",
            30,
        )),
        &mut navigator,
        &mut cx,
    );
    let at = drawn(&format!("review-thread-{}", on_line.id), &mut cx)
        .expect("the thread is drawn in the file view");
    let line = drawn("review-file-line-20", &mut cx).expect("line 20 is drawn");
    assert!(line.bottom() <= at.top() + px(1.), "{line:?} {at:?}");
    assert!(drawn(&format!("review-thread-{}", gone.id), &mut cx).is_none());
    assert!(drawn(&format!("review-thread-{}", outside.id), &mut cx).is_none());
    cx.simulate_keystrokes("escape");

    // The thread of the file outside is drawn in the file view of that file.
    cx.simulate_keystrokes("g r");
    let Ask::References { id, .. } = navigator.ask(&mut cx) else {
        panic!("the references were not asked")
    };
    navigator.tell(
        Told::References {
            id,
            answer: Answered {
                value: vec![location("/x/lib/dep.rs", 4)],
                guessed: false,
            },
        },
        &mut cx,
    );
    cx.simulate_keystrokes("enter");
    let Ask::Open { id, .. } = navigator.ask(&mut cx) else {
        panic!("the file was not asked for")
    };
    navigator.tell(
        Told::Opened {
            id,
            file: Ok(opened_of(FileKind::Outside, "/x/lib/dep.rs", 9)),
        },
        &mut cx,
    );
    assert!(drawn(&format!("review-thread-{}", outside.id), &mut cx).is_some());
    assert!(drawn(&format!("review-thread-{}", on_line.id), &mut cx).is_none());
}

// A file view takes no edit
#[gpui_kit::test]
fn the_file_view_holds_no_input_and_no_key_changes_what_it_shows(cx: &mut TestAppContext) {
    let (stage, mut navigator, mut cx) = open_long(cx);
    click_compute(&stage, &mut cx);
    let opened = opened_of(
        in_the_worktree("src/lib.rs"),
        "/gits/feature/src/lib.rs",
        30,
    );
    go_to_definition(
        vec![location("/gits/feature/src/lib.rs", 12)],
        Some(opened.clone()),
        &mut navigator,
        &mut cx,
    );
    for key in [
        "backspace",
        "delete",
        "enter",
        "space",
        "cmd-x",
        "cmd-v",
        "cmd-z",
        "ctrl-x",
        "ctrl-v",
        "ctrl-z",
        "shift-insert",
        "shift-delete",
    ] {
        cx.simulate_keystrokes(key);
        assert_eq!(
            file_view(&stage.pane, &mut cx).map(|shown| shown.file),
            Some(opened.clone()),
            "after the key {key}"
        );
    }
    cx.simulate_input("typed");
    assert_eq!(
        file_view(&stage.pane, &mut cx).map(|shown| shown.file),
        Some(opened)
    );
}

// The keys of the diff's own lines
#[gpui_kit::test]
fn the_keys_that_work_on_the_lines_of_the_diff_do_nothing_while_the_file_view_hides_them(
    cx: &mut TestAppContext,
) {
    let (mut stage, mut navigator, mut cx) = open_long(cx);
    click_compute(&stage, &mut cx);
    let left = position(&stage.pane, &mut cx);
    let opened = opened_of(
        in_the_worktree("src/lib.rs"),
        "/gits/feature/src/lib.rs",
        30,
    );
    go_to_definition(
        vec![location("/gits/feature/src/lib.rs", 12)],
        Some(opened),
        &mut navigator,
        &mut cx,
    );
    // Each of these would act on the line of the diff the reviewer left, which nobody can
    // see: move to a thread, mark the file viewed, resolve, attach, delete a draft.
    for key in [
        "m", "shift-m", "r", "x", "u", "t", "d d", "i", "o", "w", "s", "tab", "f",
    ] {
        cx.simulate_keystrokes(key);
    }
    assert_eq!(
        stage.pane.read_with(&cx, |pane, _| pane.notice()),
        None,
        "a key of the diff said something"
    );
    // The barrier: back in the diff, `y` sends `CopyReview`, which has to be the first
    // request any of the keys made.
    cx.simulate_keystrokes("escape");
    assert_eq!(position(&stage.pane, &mut cx), left);
    cx.simulate_keystrokes("y");
    let (_, request) = stage.keeper.request(&mut cx);
    assert!(
        matches!(request, Request::CopyReview { .. }),
        "a key of the diff made the pane ask {request:?}"
    );
    assert_eq!(
        stage
            .pane
            .read_with(&cx, |pane, _| (pane.view(), pane.layout())),
        (
            yaar_review_pane::View::Changes,
            yaar_review_pane::Layout::Unified
        )
    );
}

/// The state of a server, as the Navigator tells it, for the files on disk of workspace `1`
/// or for its old side in `old_side`.
fn server(language: &str, title: &str, old_side: Option<&str>, state: State) -> ServerState {
    ServerState {
        workspace: workspace_id(),
        language: language.to_owned(),
        title: title.to_owned(),
        old_side: old_side.map(str::to_owned),
        registered: false,
        state,
        server: None,
        program: None,
        pid: None,
        update: None,
    }
}

/// The state the line of the language server draws, if it draws one, with its words.
fn the_line(cx: &mut VisualTestContext) -> Option<&'static str> {
    ["ready", "starting", "missing", "asking", "downloading"]
        .into_iter()
        .find(|state| drawn(&format!("review-servers-state:{state}"), cx).is_some())
}

// The line of the language server over a file view (FR-057, FR-059)
#[gpui_kit::test]
fn while_a_file_view_is_open_the_line_of_the_language_server_is_that_of_its_file(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_long(cx);
    let pane = &stage.pane;
    let version = "a".repeat(40);
    let python = Language {
        name: "python".to_owned(),
        title: "Python".to_owned(),
        extensions: vec!["py".to_owned()],
    };
    navigator.tell(Told::Languages(vec![rust(), python]), &mut cx);
    // Each state is told for one place only, so that a line read from the wrong place
    // reads another state or none.
    navigator.tell(
        Told::Servers(vec![
            server("rust", "Rust", None, State::Ready),
            server("python", "Python", None, State::Starting),
            server(
                "rust",
                "Rust",
                Some(&version),
                State::Missing { offer: None },
            ),
        ]),
        &mut cx,
    );
    click_compute(&stage, &mut cx);
    assert_eq!(the_line(&mut cx), Some("ready"), "the line of the diff");

    let on_screen = |cx: &mut VisualTestContext| {
        pane.read_with(cx, |pane, cx| pane.server_on_screen(cx))
            .map(|server| (server.language, server.old_side, server.state))
    };
    let target = location("/gits/feature/src/lib.rs", 2);
    let kinds = [
        (
            "a file of the worktree",
            in_the_worktree("app/main.py"),
            "/gits/feature/app/main.py",
            "starting",
            Some(("python", None, State::Starting)),
        ),
        (
            "a file of an export reads its old side",
            FileKind::Version {
                workspace: workspace_id(),
                version: version.clone(),
                relative: PathBuf::from("src/lib.rs"),
            },
            "/cache/old/src/lib.rs",
            "missing",
            Some((
                "rust",
                Some(version.clone()),
                State::Missing { offer: None },
            )),
        ),
        (
            "a file outside the worktree reads the workspace on screen",
            FileKind::Outside,
            "/x/lib/dep.py",
            "starting",
            Some(("python", None, State::Starting)),
        ),
        // A file of no known language has no line, not the line of the diff it hides.
        (
            "a file of no known language",
            in_the_worktree("notes.txt"),
            "/gits/feature/notes.txt",
            "",
            None,
        ),
    ];
    for (what, kind, path, line, state) in kinds {
        go_to_definition(
            vec![target.clone()],
            Some(opened_of(kind, path, 5)),
            &mut navigator,
            &mut cx,
        );
        assert!(file_view(pane, &mut cx).is_some(), "{what}: no file view");
        assert_eq!(
            on_screen(&mut cx),
            state.clone().map(|(l, o, s)| (l.to_owned(), o, s)),
            "{what}"
        );
        match &state {
            Some(_) => assert_eq!(the_line(&mut cx), Some(line), "{what}: the line"),
            None => {
                assert_eq!(the_line(&mut cx), None, "{what}: a line is drawn");
                assert!(drawn("review-servers", &mut cx).is_none(), "{what}");
            }
        }
        // Back in the diff, the line is the diff's again.
        cx.simulate_keystrokes("escape");
        assert_eq!(file_view(pane, &mut cx), None);
        assert_eq!(the_line(&mut cx), Some("ready"), "{what}: back in the diff");
        assert_eq!(
            on_screen(&mut cx).map(|(language, ..)| language),
            Some("rust".to_owned())
        );
    }

    // The line follows what the Navigator tells while the file view stays open.
    go_to_definition(
        vec![target],
        Some(opened_of(
            in_the_worktree("app/main.py"),
            "/gits/feature/app/main.py",
            5,
        )),
        &mut navigator,
        &mut cx,
    );
    assert_eq!(the_line(&mut cx), Some("starting"));
    navigator.tell(
        Told::Servers(vec![
            server("rust", "Rust", None, State::Ready),
            server("python", "Python", None, State::Ready),
        ]),
        &mut cx,
    );
    assert_eq!(the_line(&mut cx), Some("ready"));
    assert!(drawn("review-servers-text:Python: ready", &mut cx).is_some());
    assert!(!cx.has_pending_prompt());
}
