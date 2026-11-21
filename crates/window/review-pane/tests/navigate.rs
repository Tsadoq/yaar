//! Component test: code intelligence in the diff, on GPUI's test platform. Keys, clicks
//! and the wheel go in; the pane is read back through the model it shows. The test plays
//! the Keeper on the other end of a socket pair and the Navigator at the pane's port: it
//! reads the asks the pane really makes and tells answers in the shapes `port.rs` defines,
//! and computes nothing the Navigator computes.
//!
//! A name's place on screen: the text of a line is drawn by an element whose selector is
//! `review-text-<row>-<side>:<text>`, and the font of the pane is of fixed width, so the
//! middle of the character at offset `k` is `left + (k + 0.5) * cell`, the cell being the
//! element's width over the text's characters.
//!
//! A negative clause ("asks nothing") is read after a barrier: an ask the test causes
//! after it, which the pane makes through the same port, so that it arrives after
//! everything the pane asked before it.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{
    Bounds, Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point,
};
use yaar_diff::{Row, Side};
use yaar_link::{FileStatus, History, HistoryEntry, SessionId, TurnId, WorkspaceId};
use yaar_navigator::{Answered, Ask, AskId, Language, Location, Place, Told, Tree};
use yaar_review_pane::navigate::{GUESSED, HOVER_DELAY};
use yaar_review_pane::{HoverShown, PaneRow, Position, ReferencesShown, ReviewPane};

use common::navigator::NavigatorEnd;
use common::{
    COMMIT, Stage, bounds_of_row, commit, drawn, file, id, open, since_main, texts, wait_for_files,
    wait_until, workspace,
};

/// The new side of the file the tests of the hover read: an added line holds the name
/// under test.
const NEW: &str =
    "fn main() {\n    let sides = vec![1, 2];\n    let total = perimeter(sides);\n}\n";
const OLD: &str = "fn main() {\n    let sides = vec![1, 2];\n}\n";
const ADDED: &str = "    let total = perimeter(sides);";
const SIGNATURE: &str = "fn perimeter(sides: &[u32]) -> u32";

fn workspace_id() -> WorkspaceId {
    id('1')
}

/// The id of a commit or a state that is this digit forty times.
fn version(digit: char) -> String {
    digit.to_string().repeat(40)
}

/// The language the tests tell the pane about, as the Navigator tells it once at its start.
fn rust() -> Language {
    Language {
        name: "rust".to_owned(),
        title: "Rust".to_owned(),
        extensions: vec!["rs".to_owned()],
    }
}

/// Opens the pane on the change of `path`, from `old` to `new`, waits for its lines, and
/// connects the Navigator, which tells the language of the file. The first line of the
/// diff is clicked: the pane has the focus, so that its keys work.
fn open_on(
    path: &str,
    status: FileStatus,
    old: &str,
    new: &str,
    cx: &mut TestAppContext,
) -> (Stage, NavigatorEnd, VisualTestContext) {
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(Vec::new(), vec![file(path, status)]);
    stage.keeper.show(&changes, |_| texts(old, new), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    let navigator = connect_navigator(&stage, &mut cx);
    (stage, navigator, cx)
}

/// Connects the Navigator to the pane's models; it tells the languages it knows.
fn connect_navigator(stage: &Stage, cx: &mut VisualTestContext) -> NavigatorEnd {
    let navigator = NavigatorEnd::connect(&stage.models, cx);
    navigator.tell(Told::Languages(vec![rust()]), cx);
    navigator
}

/// The index of the row of the diff that is this one.
fn row_of(pane: &Entity<ReviewPane>, wanted: Row, cx: &mut VisualTestContext) -> usize {
    pane.read_with(cx, |pane, _| {
        pane.rows()
            .iter()
            .position(|row| matches!(row, PaneRow::Diff { row, .. } if *row == wanted))
    })
    .unwrap_or_else(|| panic!("no row {wanted:?}"))
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

/// A click on `name` in the text of a row.
fn click_name(
    row: usize,
    side: &str,
    text: &str,
    name: &str,
    modifiers: Modifiers,
    cx: &mut VisualTestContext,
) {
    let at = name_point(row, side, text, name, cx);
    cx.simulate_click(at, modifiers);
}

fn name(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<String> {
    pane.read_with(cx, |pane, _| pane.name_at())
}

fn position(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Position {
    pane.read_with(cx, |pane, _| pane.position().cloned())
        .expect("the reviewer is on a line")
}

fn hover(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<HoverShown> {
    pane.read_with(cx, |pane, _| pane.hover().cloned())
}

fn references(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<ReferencesShown> {
    pane.read_with(cx, |pane, _| pane.references().cloned())
}

fn notice(pane: &Entity<ReviewPane>, cx: &mut VisualTestContext) -> Option<String> {
    pane.read_with(cx, |pane, _| pane.notice())
}

/// The place of `name` in `text`, line `line` from 1, as the pane has to ask for it.
fn place(tree: Tree, path: &str, line: u32, text: &str, name: &str, shown: &[&str]) -> Place {
    Place {
        workspace: workspace_id(),
        tree,
        path: PathBuf::from(path),
        line,
        column: text.find(name).expect("the line holds the name") as u32,
        name: name.to_owned(),
        shown: shown.iter().map(PathBuf::from).collect(),
    }
}

/// The next ask, which has to be a hover.
fn hover_ask(navigator: &mut NavigatorEnd, cx: &mut VisualTestContext) -> (AskId, Place) {
    match navigator.ask(cx) {
        Ask::Hover { id, place } => (id, place),
        other => panic!("a hover was expected, and the pane asked {other:?}"),
    }
}

/// The next ask, which has to be a hover, answered with a text, so that moving on to
/// another line cancels nothing; gives where it was asked.
fn hover_answered(navigator: &mut NavigatorEnd, cx: &mut VisualTestContext) -> Place {
    let (id, place) = hover_ask(navigator, cx);
    navigator.tell(
        Told::Hover {
            id,
            answer: answer(Some("text"), false),
        },
        cx,
    );
    place
}

fn answer(text: Option<&str>, guessed: bool) -> Answered<Option<String>> {
    Answered {
        value: text.map(str::to_owned),
        guessed,
    }
}

fn location(path: &str, line: u32, text: &str) -> Location {
    Location {
        path: PathBuf::from(path),
        line,
        column: 4,
        text: text.to_owned(),
    }
}

/// The three places the references of the tests lie at.
fn three() -> Vec<Location> {
    vec![
        location(
            "/gits/feature/src/geometry.rs",
            1,
            "pub fn perimeter(sides: &[u32]) -> u32 {",
        ),
        location("/gits/feature/src/lib.rs", 3, ADDED),
        location(
            "/gits/feature/src/report.rs",
            5,
            "    perimeter(&shape.sides)",
        ),
    ]
}

fn bounds(selector: &str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    drawn(selector, cx).unwrap_or_else(|| panic!("{selector} is not drawn"))
}

// RP-21
#[gpui_kit::test]
fn hovering_a_name_shows_what_the_navigator_reports_and_marks_an_answer_found_by_text_match(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_on("src/lib.rs", FileStatus::Modified, OLD, NEW, cx);
    let pane = &stage.pane;
    // The reviewer comes to the diff on the first changed line, on its first name.
    assert_eq!(name(pane, &mut cx).as_deref(), Some("let"));
    let added = row_of(pane, Row::Added { new: 2 }, &mut cx);

    // A click on a name puts the reviewer on that line and name.
    click_name(added, "new", ADDED, "perimeter", Modifiers::none(), &mut cx);
    assert_eq!(name(pane, &mut cx).as_deref(), Some("perimeter"));
    assert_eq!(
        position(pane, &mut cx),
        Position {
            path: PathBuf::from("src/lib.rs"),
            side: Side::New,
            line: 2,
        }
    );
    // The mark of the name is drawn under the name, and no wider.
    let mark = bounds("review-name", &mut cx);
    let at = name_point(added, "new", ADDED, "perimeter", &mut cx);
    assert!(mark.contains(&at), "{mark:?} does not hold {at:?}");
    let cell = bounds(&format!("review-text-{added}-new:{ADDED}"), &mut cx)
        .size
        .width
        / ADDED.chars().count() as f32;
    // The layout is rounded to whole pixels.
    assert!(
        (mark.size.width - cell * 9.).abs() < gpui_kit::px(1.5),
        "{mark:?} is not nine cells of {cell:?}"
    );

    // `shift-k` asks the hover of that name, of the files on disk.
    cx.simulate_keystrokes("shift-k");
    let (first, asked) = hover_ask(&mut navigator, &mut cx);
    let at_perimeter = place(
        Tree::Worktree,
        "src/lib.rs",
        3,
        ADDED,
        "perimeter",
        &["src/lib.rs"],
    );
    assert_eq!(asked, at_perimeter);
    // While it is on its way the pane says so.
    assert_eq!(
        hover(pane, &mut cx),
        Some(HoverShown {
            place: at_perimeter.clone(),
            text: None,
            guessed: false,
            waiting: true,
        })
    );
    assert!(drawn("review-hover-waiting", &mut cx).is_some());

    // What the Navigator reports is shown under the rows, with no mark of a guess.
    navigator.tell(
        Told::Hover {
            id: first,
            answer: answer(Some(SIGNATURE), false),
        },
        &mut cx,
    );
    let shown = HoverShown {
        place: at_perimeter.clone(),
        text: Some(SIGNATURE.to_owned()),
        guessed: false,
        waiting: false,
    };
    assert_eq!(hover(pane, &mut cx), Some(shown));
    let text = bounds(&format!("review-hover-text:{SIGNATURE}"), &mut cx);
    assert!(bounds_of_row(added, &mut cx).bottom() <= text.top());
    assert!(drawn("review-hover-waiting", &mut cx).is_none());
    assert!(drawn("review-hover-guessed", &mut cx).is_none());

    // An answer found by text match, while no server is ready, carries the mark.
    cx.simulate_keystrokes("shift-k");
    let (second, _) = hover_ask(&mut navigator, &mut cx);
    navigator.tell(
        Told::Hover {
            id: second,
            answer: answer(Some("src/geometry.rs:1: pub fn perimeter"), true),
        },
        &mut cx,
    );
    let marked = hover(pane, &mut cx).expect("the hover is shown");
    assert!(marked.guessed && !marked.waiting);
    assert!(drawn("review-hover-guessed", &mut cx).is_some());
    assert!(drawn(&format!("review-hover-guessed-words:{GUESSED}"), &mut cx).is_some());
    assert!(
        drawn(
            "review-hover-text:src/geometry.rs:1: pub fn perimeter",
            &mut cx
        )
        .is_some()
    );

    // An answer with nothing to say is said so.
    cx.simulate_keystrokes("shift-k");
    let (third, _) = hover_ask(&mut navigator, &mut cx);
    navigator.tell(
        Told::Hover {
            id: third,
            answer: answer(None, false),
        },
        &mut cx,
    );
    assert_eq!(hover(pane, &mut cx).map(|shown| shown.text), Some(None));
    assert!(drawn("review-hover-nothing", &mut cx).is_some());
    assert!(drawn("review-hover-guessed", &mut cx).is_none());
}

// RP-21, the mouse resting on a name
#[gpui_kit::test]
fn the_mouse_resting_on_a_name_asks_its_hover_after_the_delay(cx: &mut TestAppContext) {
    let (stage, mut navigator, mut cx) = open_on("src/lib.rs", FileStatus::Modified, OLD, NEW, cx);
    let pane = &stage.pane;
    let added = row_of(pane, Row::Added { new: 2 }, &mut cx);
    click_name(added, "new", ADDED, "perimeter", Modifiers::none(), &mut cx);

    // The mouse comes to rest on `sides`, a name of the line other than the reviewer's.
    let on_sides = name_point(added, "new", ADDED, "sides", &mut cx);
    cx.simulate_mouse_move(on_sides, None, Modifiers::none());
    // One millisecond short of the delay: nothing was asked. The barrier is an ask of
    // the reviewer's own, which comes after anything the resting asked.
    cx.executor()
        .advance_clock(HOVER_DELAY - Duration::from_millis(1));
    cx.simulate_keystrokes("shift-k");
    let (by_key, asked) = hover_ask(&mut navigator, &mut cx);
    assert_eq!(asked.name, "perimeter", "the mouse asked before its time");

    // The delay passes: the hover of the name under the mouse is asked, and replaces the
    // one asked by key, which was not answered and is cancelled.
    cx.executor().advance_clock(Duration::from_millis(1));
    let (by_mouse, asked) = hover_ask(&mut navigator, &mut cx);
    assert_eq!(
        asked,
        place(
            Tree::Worktree,
            "src/lib.rs",
            3,
            ADDED,
            "sides",
            &["src/lib.rs"]
        )
    );
    assert_eq!(navigator.ask(&mut cx), Ask::Cancel { id: by_key });
    // The reviewer stays where they were: the mouse only asks.
    assert_eq!(name(pane, &mut cx).as_deref(), Some("perimeter"));
    navigator.tell(
        Told::Hover {
            id: by_mouse,
            answer: answer(Some("sides: &[u32]"), false),
        },
        &mut cx,
    );
    assert_eq!(
        hover(pane, &mut cx).map(|shown| (shown.place.name, shown.text)),
        Some(("sides".to_owned(), Some("sides: &[u32]".to_owned())))
    );

    // A mouse that moves on to another name before the delay asks for the second only.
    let on_total = name_point(added, "new", ADDED, "total", &mut cx);
    cx.simulate_mouse_move(on_total, None, Modifiers::none());
    cx.executor()
        .advance_clock(HOVER_DELAY - Duration::from_millis(1));
    let on_let = name_point(added, "new", ADDED, "let", &mut cx);
    cx.simulate_mouse_move(on_let, None, Modifiers::none());
    cx.executor()
        .advance_clock(HOVER_DELAY - Duration::from_millis(1));
    cx.simulate_keystrokes("shift-k");
    let (_, asked) = hover_ask(&mut navigator, &mut cx);
    assert_eq!(
        asked.name, "perimeter",
        "the mouse asked for a name it left"
    );
    cx.executor().advance_clock(Duration::from_millis(1));
    let (_, asked) = hover_ask(&mut navigator, &mut cx);
    assert_eq!(asked.name, "let");
}

// RP-21, the keys that move the name, and what moving does to the hover
#[gpui_kit::test]
fn l_and_h_move_the_name_and_moving_drops_the_hover_and_cancels_the_one_on_its_way(
    cx: &mut TestAppContext,
) {
    let (stage, mut navigator, mut cx) = open_on("src/lib.rs", FileStatus::Modified, OLD, NEW, cx);
    let pane = &stage.pane;
    let added = row_of(pane, Row::Added { new: 2 }, &mut cx);
    click_name(added, "new", ADDED, "perimeter", Modifiers::none(), &mut cx);

    let names = ["let", "total", "perimeter", "sides"];
    // Name by name to the end of the line, where `l` stays, and back to its start.
    cx.simulate_keystrokes("l");
    assert_eq!(name(pane, &mut cx).as_deref(), Some("sides"));
    cx.simulate_keystrokes("l");
    assert_eq!(name(pane, &mut cx).as_deref(), Some("sides"));
    for wanted in names.iter().rev().skip(1) {
        cx.simulate_keystrokes("h");
        assert_eq!(name(pane, &mut cx).as_deref(), Some(*wanted));
    }
    cx.simulate_keystrokes("h");
    assert_eq!(name(pane, &mut cx).as_deref(), Some("let"));
    // The mark follows the name.
    let mark = bounds("review-name", &mut cx);
    assert!(mark.contains(&name_point(added, "new", ADDED, "let", &mut cx)));

    // Moving to another name drops the hover that was shown for the first.
    cx.simulate_keystrokes("shift-k");
    let (first, _) = hover_ask(&mut navigator, &mut cx);
    navigator.tell(
        Told::Hover {
            id: first,
            answer: answer(Some("let"), false),
        },
        &mut cx,
    );
    assert!(hover(pane, &mut cx).is_some());
    cx.simulate_keystrokes("l");
    assert_eq!(hover(pane, &mut cx), None);
    assert!(drawn("review-hover", &mut cx).is_none());

    // A hover still on its way when the reviewer moves to another line is cancelled.
    cx.simulate_keystrokes("shift-k");
    let (waiting, _) = hover_ask(&mut navigator, &mut cx);
    cx.simulate_keystrokes("j");
    assert_eq!(navigator.ask(&mut cx), Ask::Cancel { id: waiting });
    assert_eq!(hover(pane, &mut cx), None);
    // On the next line, which is the unchanged `}`, the name is its first.
    assert_eq!(name(pane, &mut cx), None, "`}}` holds no name");

    // Escape takes a hover away, and asks nothing.
    cx.simulate_keystrokes("k");
    cx.simulate_keystrokes("shift-k");
    let (shown, _) = hover_ask(&mut navigator, &mut cx);
    navigator.tell(
        Told::Hover {
            id: shown,
            answer: answer(Some("a"), false),
        },
        &mut cx,
    );
    assert!(hover(pane, &mut cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(hover(pane, &mut cx), None);
    assert_eq!(notice(pane, &mut cx), None);
}

// RP-24
#[gpui_kit::test]
fn the_references_to_a_name_are_listed_and_any_of_them_opens(cx: &mut TestAppContext) {
    let (stage, mut navigator, mut cx) = open_on("src/lib.rs", FileStatus::Modified, OLD, NEW, cx);
    let pane = &stage.pane;
    let added = row_of(pane, Row::Added { new: 2 }, &mut cx);
    click_name(added, "new", ADDED, "perimeter", Modifiers::none(), &mut cx);
    let at_perimeter = place(
        Tree::Worktree,
        "src/lib.rs",
        3,
        ADDED,
        "perimeter",
        &["src/lib.rs"],
    );

    // `g r` asks for the references of the name the reviewer is on.
    cx.simulate_keystrokes("g r");
    let Ask::References { id: first, place } = navigator.ask(&mut cx) else {
        panic!("the references were not asked")
    };
    assert_eq!(place, at_perimeter);
    assert_eq!(
        references(pane, &mut cx),
        Some(ReferencesShown {
            place: at_perimeter.clone(),
            locations: Vec::new(),
            guessed: false,
            waiting: true,
            chosen: 0,
            definitions: false,
        })
    );
    assert!(drawn("review-references-waiting", &mut cx).is_some());

    // The three places told are listed under the rows, each with its path and line.
    navigator.tell(
        Told::References {
            id: first,
            answer: Answered {
                value: three(),
                guessed: false,
            },
        },
        &mut cx,
    );
    let listed = references(pane, &mut cx).expect("the list is shown");
    assert_eq!(listed.locations, three());
    assert!(!listed.waiting && !listed.guessed && !listed.definitions);
    assert_eq!(listed.chosen, 0);
    let words = [
        "src/geometry.rs:1 pub fn perimeter(sides: &[u32]) -> u32 {",
        "src/lib.rs:3 let total = perimeter(sides);",
        "src/report.rs:5 perimeter(&shape.sides)",
    ];
    for (index, words) in words.iter().enumerate() {
        let entry = bounds(&format!("review-reference-{index}"), &mut cx);
        let drawn_words = bounds(&format!("review-reference-words-{index}:{words}"), &mut cx);
        assert!(entry.contains(&drawn_words.center()), "{index}");
    }
    assert!(drawn("review-references-guessed", &mut cx).is_none());
    let chosen = bounds("review-reference-chosen", &mut cx);
    assert!(bounds("review-reference-0", &mut cx).contains(&chosen.center()));
    // The list takes the focus: the pane's own keys do not work in it, the list's do.
    cx.update(|window, _| assert!(!stage.focus.is_focused(window)));

    // `j` chooses the next, and `k` the one before.
    cx.simulate_keystrokes("j");
    assert_eq!(references(pane, &mut cx).map(|shown| shown.chosen), Some(1));
    let chosen = bounds("review-reference-chosen", &mut cx);
    assert!(bounds("review-reference-1", &mut cx).contains(&chosen.center()));
    cx.simulate_keystrokes("down");
    assert_eq!(references(pane, &mut cx).map(|shown| shown.chosen), Some(2));
    cx.simulate_keystrokes("down");
    assert_eq!(references(pane, &mut cx).map(|shown| shown.chosen), Some(2));
    cx.simulate_keystrokes("k");
    assert_eq!(references(pane, &mut cx).map(|shown| shown.chosen), Some(1));
    cx.simulate_keystrokes("up up");
    assert_eq!(references(pane, &mut cx).map(|shown| shown.chosen), Some(0));
    cx.simulate_keystrokes("j");

    // `enter` opens the chosen one: the file is asked for, and the file view shows it.
    cx.simulate_keystrokes("enter");
    let Ask::Open {
        id: opening,
        location,
    } = navigator.ask(&mut cx)
    else {
        panic!("the file was not asked for")
    };
    assert_eq!(location, three()[1]);
    navigator.tell(
        Told::Opened {
            id: opening,
            file: Ok(opened("/gits/feature/src/lib.rs", "src/lib.rs", 8)),
        },
        &mut cx,
    );
    let view = pane.read_with(&cx, |pane, _| pane.file_view().cloned());
    assert_eq!(view.map(|view| view.line), Some(3));
    // The list closed with the file view's opening, and the rows are not drawn.
    assert_eq!(references(pane, &mut cx), None);
    // `escape` goes back to the diff.
    cx.simulate_keystrokes("escape");
    assert_eq!(
        pane.read_with(&cx, |pane, _| pane.file_view().cloned()),
        None
    );

    // A click on an entry opens it, too.
    cx.simulate_keystrokes("g r");
    let Ask::References { id: again, .. } = navigator.ask(&mut cx) else {
        panic!("the references were not asked again")
    };
    navigator.tell(
        Told::References {
            id: again,
            answer: Answered {
                value: three(),
                guessed: false,
            },
        },
        &mut cx,
    );
    let third = bounds("review-reference-2", &mut cx).center();
    cx.simulate_click(third, Modifiers::none());
    let Ask::Open { location, .. } = navigator.ask(&mut cx) else {
        panic!("the file was not asked for")
    };
    assert_eq!(location, three()[2]);
}

/// A file of this many lines, `line 1` to `line <lines>`, as the Navigator opens it.
fn opened(path: &str, relative: &str, lines: usize) -> yaar_navigator::OpenedFile {
    yaar_navigator::OpenedFile {
        path: PathBuf::from(path),
        kind: yaar_navigator::FileKind::Worktree {
            workspace: workspace_id(),
            relative: PathBuf::from(relative),
        },
        text: (1..=lines).map(|n| format!("line {n}\n")).collect(),
    }
}

// RP-24, a list found by text match
#[gpui_kit::test]
fn a_list_of_references_is_marked_only_when_it_was_found_by_text_match(cx: &mut TestAppContext) {
    let (stage, mut navigator, mut cx) = open_on("src/lib.rs", FileStatus::Modified, OLD, NEW, cx);
    let pane = &stage.pane;
    let added = row_of(pane, Row::Added { new: 2 }, &mut cx);
    click_name(added, "new", ADDED, "perimeter", Modifiers::none(), &mut cx);

    for guessed in [true, false] {
        cx.simulate_keystrokes("g r");
        let Ask::References { id, .. } = navigator.ask(&mut cx) else {
            panic!("the references were not asked")
        };
        navigator.tell(
            Told::References {
                id,
                answer: Answered {
                    value: three(),
                    guessed,
                },
            },
            &mut cx,
        );
        assert_eq!(
            references(pane, &mut cx).map(|shown| shown.guessed),
            Some(guessed)
        );
        assert_eq!(
            drawn("review-references-guessed", &mut cx).is_some(),
            guessed
        );
        assert_eq!(
            drawn(
                &format!("review-references-guessed-words:{GUESSED}"),
                &mut cx
            )
            .is_some(),
            guessed
        );
        // Escape closes the list and gives the focus back to the rows.
        cx.simulate_keystrokes("escape");
        assert_eq!(references(pane, &mut cx), None);
        cx.update(|window, _| assert!(stage.focus.is_focused(window)));
    }
}

// RP-24, with none
#[gpui_kit::test]
fn with_no_reference_the_notice_says_none_was_found(cx: &mut TestAppContext) {
    let (stage, mut navigator, mut cx) = open_on("src/lib.rs", FileStatus::Modified, OLD, NEW, cx);
    let pane = &stage.pane;
    let added = row_of(pane, Row::Added { new: 2 }, &mut cx);
    click_name(added, "new", ADDED, "perimeter", Modifiers::none(), &mut cx);
    cx.simulate_keystrokes("g r");
    let Ask::References { id, .. } = navigator.ask(&mut cx) else {
        panic!("the references were not asked")
    };
    navigator.tell(
        Told::References {
            id,
            answer: Answered {
                value: Vec::new(),
                guessed: false,
            },
        },
        &mut cx,
    );
    assert_eq!(references(pane, &mut cx), None);
    assert_eq!(
        notice(pane, &mut cx).as_deref(),
        Some("No references to perimeter were found")
    );
    // The rows have the focus: the pane's keys work.
    cx.simulate_keystrokes("l");
    assert_eq!(name(pane, &mut cx).as_deref(), Some("sides"));
}

// RP-25
#[gpui_kit::test]
fn on_a_removed_line_hover_and_go_to_definition_ask_for_the_old_version_in_both_layouts(
    cx: &mut TestAppContext,
) {
    let old = "fn run() {\n    old_call(x);\n    keep();\n}\n";
    let new = "fn run() {\n    keep();\n    fresh_call(y);\n}\n";
    let (stage, mut navigator, mut cx) = open_on("src/run.rs", FileStatus::Modified, old, new, cx);
    let pane = &stage.pane;
    // The model was told the old side of the change shown: the version it counts from.
    // Nothing makes the model's own asks arrive before the test looks: it waits for the
    // fact, which stays true once it is.
    wait_until(&mut cx, "the old side told", |cx| {
        navigator.standing(cx).contains(&Ask::OldSide {
            workspace: workspace_id(),
            version: Some(COMMIT.to_owned()),
        })
    });

    let removed_text = "    old_call(x);";
    let removed = row_of(pane, Row::Removed { old: 1 }, &mut cx);
    click_name(
        removed,
        "old",
        removed_text,
        "old_call",
        Modifiers::none(),
        &mut cx,
    );
    assert_eq!(name(pane, &mut cx).as_deref(), Some("old_call"));
    let in_the_old_version = place(
        Tree::Version(COMMIT.to_owned()),
        "src/run.rs",
        2,
        removed_text,
        "old_call",
        &["src/run.rs"],
    );
    cx.simulate_keystrokes("shift-k");
    let asked = hover_answered(&mut navigator, &mut cx);
    assert_eq!(asked, in_the_old_version);
    cx.simulate_keystrokes("g d");
    let Ask::Definition { place: asked, .. } = navigator.ask(&mut cx) else {
        panic!("the definition was not asked")
    };
    assert_eq!(asked, in_the_old_version);

    // The control half: an added line of the same change asks the files on disk.
    let added_text = "    fresh_call(y);";
    let added = row_of(pane, Row::Added { new: 2 }, &mut cx);
    click_name(
        added,
        "new",
        added_text,
        "fresh_call",
        Modifiers::none(),
        &mut cx,
    );
    cx.simulate_keystrokes("shift-k");
    let asked = hover_answered(&mut navigator, &mut cx);
    assert_eq!(
        asked,
        place(
            Tree::Worktree,
            "src/run.rs",
            3,
            added_text,
            "fresh_call",
            &["src/run.rs"]
        )
    );

    // The split view: the old half of the changed row, by a click on its name.
    cx.simulate_keystrokes("s");
    let changed = row_of(
        pane,
        Row::Changed {
            old: Some(1),
            new: None,
        },
        &mut cx,
    );
    let old_half = bounds(&format!("review-row-{changed}-old"), &mut cx);
    let at = name_point(changed, "old", removed_text, "old_call", &mut cx);
    assert!(old_half.contains(&at));
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(name(pane, &mut cx).as_deref(), Some("old_call"));
    cx.simulate_keystrokes("shift-k");
    let asked = hover_answered(&mut navigator, &mut cx);
    assert_eq!(asked, in_the_old_version);
    cx.simulate_keystrokes("g d");
    let Ask::Definition { place: asked, .. } = navigator.ask(&mut cx) else {
        panic!("the definition was not asked")
    };
    assert_eq!(asked, in_the_old_version);
}

// RP-25, a renamed file
#[gpui_kit::test]
fn a_removed_line_of_a_renamed_file_is_asked_by_the_path_it_had(cx: &mut TestAppContext) {
    let old = "fn run() {\n    old_call(x);\n}\n";
    let new = "fn run() {\n    new_call(x);\n}\n";
    let status = FileStatus::Renamed {
        from: PathBuf::from("src/was.rs"),
    };
    let (stage, mut navigator, mut cx) = open_on("src/now.rs", status, old, new, cx);
    let pane = &stage.pane;
    let removed = row_of(pane, Row::Removed { old: 1 }, &mut cx);
    click_name(
        removed,
        "old",
        "    old_call(x);",
        "old_call",
        Modifiers::none(),
        &mut cx,
    );
    cx.simulate_keystrokes("shift-k");
    let asked = hover_answered(&mut navigator, &mut cx);
    assert_eq!(
        asked,
        place(
            Tree::Version(COMMIT.to_owned()),
            "src/was.rs",
            2,
            "    old_call(x);",
            "old_call",
            &["src/now.rs"]
        )
    );
    let added = row_of(pane, Row::Added { new: 1 }, &mut cx);
    click_name(
        added,
        "new",
        "    new_call(x);",
        "new_call",
        Modifiers::none(),
        &mut cx,
    );
    cx.simulate_keystrokes("shift-k");
    let asked = hover_answered(&mut navigator, &mut cx);
    assert_eq!(asked.path, PathBuf::from("src/now.rs"));
    assert_eq!(asked.tree, Tree::Worktree);
}

/// A history of one turn, which ended in the state `s1`, counted from `start`.
fn one_turn(start: &str, s1: &str) -> History {
    History {
        workspace: workspace_id(),
        start: Some(start.to_owned()),
        entries: vec![HistoryEntry::Turn {
            turn: TurnId::new("a".repeat(32)),
            session: SessionId::new("5".repeat(32)),
            state: Some(s1.to_owned()),
        }],
    }
}

// RP-25, a change that ends at a version
#[gpui_kit::test]
fn in_a_change_that_ends_at_a_version_the_new_side_carries_its_text_and_the_old_side_its_version(
    cx: &mut TestAppContext,
) {
    use yaar_link::{Fact, Reply, Request};
    let (mut stage, mut cx) = open(vec![workspace('1', "feature")], cx);
    let changes = since_main(
        vec![commit('b', "Edit")],
        vec![file("src/run.rs", FileStatus::Modified)],
    );
    stage
        .keeper
        .show(&changes, |_| texts("x\n", "y\n"), &mut cx);
    wait_for_files(&stage.pane, &mut cx);
    let (start, s1) = (version('8'), version('5'));
    let history = one_turn(&start, &s1);
    let asked = stage.keeper.history_asked();
    stage.keeper.set_history(history.clone());
    stage
        .keeper
        .tell(Fact::Workspaces(vec![common::repository(vec![workspace(
            '1', "feature",
        )])]));
    wait_until(&mut cx, "the history in the pane", |cx| {
        stage.keeper.history_asked() > asked
            && stage
                .pane
                .read_with(cx, |pane, _| pane.history() == Some(&history))
    });
    let mut navigator = connect_navigator(&stage, &mut cx);

    // The turn is selected by a click on its entry in the tree.
    let entry = bounds("review-history-0", &mut cx).center();
    cx.simulate_click(entry, Modifiers::none());
    let compared = stage.keeper.expect(
        Request::CompareVersions {
            workspace: workspace_id(),
            from: start.clone(),
            to: Some(s1.clone()),
        },
        &mut cx,
    );
    stage.keeper.reply(
        compared,
        Ok(Reply::Compared(vec![file(
            "src/run.rs",
            FileStatus::Modified,
        )])),
    );
    let read = stage.keeper.expect(
        Request::ReadVersions {
            workspace: workspace_id(),
            old: Some(start.clone()),
            old_path: PathBuf::from("src/run.rs"),
            new: Some(s1.clone()),
            path: PathBuf::from("src/run.rs"),
        },
        &mut cx,
    );
    stage.keeper.reply(
        read,
        texts("let old_one = 1;\nkeep();\n", "let new_one = 2;\nkeep();\n").map(Reply::File),
    );
    wait_until(&mut cx, "the files of the turn", |cx| {
        stage.pane.read_with(cx, |pane, _| {
            pane.rows().iter().any(|row| {
                matches!(
                    row,
                    PaneRow::Diff {
                        row: Row::Added { .. },
                        ..
                    }
                )
            })
        })
    });
    let pane = &stage.pane;
    wait_until(&mut cx, "the old side of the turn told", |cx| {
        navigator.standing(cx).last()
            == Some(&Ask::OldSide {
                workspace: workspace_id(),
                version: Some(start.clone()),
            })
    });

    // An added line asks as the new side of the version the change ends at, with the whole
    // text of the file there; a removed one the version before.
    let added = row_of(pane, Row::Added { new: 0 }, &mut cx);
    let removed = row_of(pane, Row::Removed { old: 0 }, &mut cx);
    click_name(
        added,
        "new",
        "let new_one = 2;",
        "new_one",
        Modifiers::none(),
        &mut cx,
    );
    cx.simulate_keystrokes("shift-k");
    let asked = hover_answered(&mut navigator, &mut cx);
    assert_eq!(
        asked,
        place(
            Tree::Past {
                version: s1.clone(),
                text: "let new_one = 2;\nkeep();\n".into(),
            },
            "src/run.rs",
            1,
            "let new_one = 2;",
            "new_one",
            &["src/run.rs"]
        )
    );
    click_name(
        removed,
        "old",
        "let old_one = 1;",
        "old_one",
        Modifiers::none(),
        &mut cx,
    );
    cx.simulate_keystrokes("shift-k");
    let asked = hover_answered(&mut navigator, &mut cx);
    assert_eq!(asked.tree, Tree::Version(start));
    assert_eq!(asked.name, "old_one");
}

// The place of a name, as a position of the diff gives it
#[gpui_kit::test]
fn the_place_of_a_name_is_read_from_a_position_of_the_diff(cx: &mut TestAppContext) {
    let (stage, _navigator, cx) = open_on("src/lib.rs", FileStatus::Modified, OLD, NEW, cx);
    let pane = &stage.pane;
    let on = Position {
        path: PathBuf::from("src/lib.rs"),
        side: Side::New,
        line: 2,
    };
    let held = pane.read_with(&cx, |pane, cx| pane.place_at(&on, cx));
    // The reviewer is on the first name of the first changed line until they move.
    assert_eq!(
        held,
        Some(place(
            Tree::Worktree,
            "src/lib.rs",
            3,
            ADDED,
            "let",
            &["src/lib.rs"]
        ))
    );
    // A line of no name has no place.
    let closing = Position {
        path: PathBuf::from("src/lib.rs"),
        side: Side::New,
        line: 3,
    };
    assert_eq!(
        pane.read_with(&cx, |pane, cx| pane.place_at(&closing, cx)),
        None
    );
}
