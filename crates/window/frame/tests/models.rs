//! Component test: the shared models of the Window, through what a pane uses of them.
//! The test plays the Keeper on the other end of a socket pair: it answers what the
//! models ask and tells them facts, and reads back through the models.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{AppContext, Entity, Subscription, TestAppContext};
use yaar_frame::{AskError, KeeperLink, Models, WorkspaceChanges, WorkspaceList};
use yaar_link::{
    Base, BaseSource, ChangedFile, Changes, Comparison, Content, Fact, FileSides, FileStatus, Head,
    Refusal, Reply, Repository, RepositoryId, Request, Workspace, WorkspaceId,
};

use common::{KeeperEnd, answer, wait_until};

/// The full id of a commit, as the Link carries it.
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn id(digit: char) -> WorkspaceId {
    WorkspaceId::new(digit.to_string().repeat(32))
}

fn workspace(digit: char, branch: &str) -> Workspace {
    Workspace {
        id: id(digit),
        path: PathBuf::from(format!("/gits/{branch}")),
        head: Head::Branch(branch.to_owned()),
    }
}

fn repository(letter: char, workspaces: Vec<Workspace>) -> Repository {
    Repository {
        id: RepositoryId::new(letter.to_string().repeat(32)),
        name: format!("repository-{letter}"),
        workspaces,
    }
}

fn file(path: &str, status: FileStatus) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        status,
    }
}

fn changes(workspace: char, counted_from: Option<&str>, files: Vec<ChangedFile>) -> Changes {
    Changes {
        workspace: id(workspace),
        comparison: Comparison::SinceBase {
            base: Base {
                name: "main".to_owned(),
                source: BaseSource::Default,
            },
        },
        counted_from: counted_from.map(str::to_owned),
        commits: Vec::new(),
        files,
    }
}

fn get_changes(workspace: char) -> Request {
    Request::GetChanges {
        workspace: id(workspace),
    }
}

/// A list of workspaces fed by a Keeper the test plays, which has answered the first
/// question for the list with `listed`.
fn listing(listed: &[Repository], cx: &mut TestAppContext) -> (Entity<WorkspaceList>, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let mut keeper = KeeperEnd::attach(&link, cx);
    // Made when the Keeper is reached already: it asks at once.
    let list = cx.new(|cx| WorkspaceList::new(link, cx));
    let asked = keeper.expect(Request::ListWorkspaces, cx);
    keeper.reply(asked, Ok(Reply::Workspaces(listed.to_vec())));
    wait_for_list(&list, listed, cx);
    (list, keeper)
}

/// The models of a window whose Keeper the test plays, which has answered the first
/// question for the list with `listed`.
fn models_listing(listed: &[Repository], cx: &mut TestAppContext) -> (Models, KeeperEnd) {
    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach(&link, cx);
    let asked = keeper.expect(Request::ListWorkspaces, cx);
    keeper.reply(asked, Ok(Reply::Workspaces(listed.to_vec())));
    wait_for_list(&models.workspaces, listed, cx);
    (models, keeper)
}

fn wait_for_list(list: &Entity<WorkspaceList>, wanted: &[Repository], cx: &mut TestAppContext) {
    wait_until(cx, "the list of workspaces", |cx| {
        list.read_with(cx, |list, _| list.repositories() == wanted)
    });
}

fn on_screen(list: &Entity<WorkspaceList>, cx: &mut TestAppContext) -> Option<WorkspaceId> {
    list.read_with(cx, |list, _| list.selected().cloned())
}

fn wait_for_changes(model: &Entity<WorkspaceChanges>, wanted: &Changes, cx: &mut TestAppContext) {
    wait_until(cx, "the changes", |cx| {
        model.read_with(cx, |model, _| model.changes() == Some(wanted))
    });
}

/// What an observer of a model sees of it, each time the model says it changed: the way
/// a pane follows a model.
fn watch<T: 'static, Seen: 'static>(
    model: &Entity<T>,
    cx: &mut TestAppContext,
    see: impl Fn(&T) -> Seen + 'static,
) -> (Rc<RefCell<Vec<Seen>>>, Subscription) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let subscription = cx.update(|cx| {
        let seen = seen.clone();
        cx.observe(model, move |model, cx| {
            seen.borrow_mut().push(see(model.read(cx)))
        })
    });
    (seen, subscription)
}

#[gpui_kit::test]
fn the_list_is_asked_for_once_the_keeper_is_reached_and_follows_what_it_tells(
    cx: &mut TestAppContext,
) {
    let link = cx.new(|_| KeeperLink::new());
    let list = cx.new(|cx| WorkspaceList::new(link.clone(), cx));
    list.read_with(cx, |list, _| {
        assert!(list.repositories().is_empty());
        assert_eq!(list.selected(), None);
    });

    let mut keeper = KeeperEnd::attach(&link, cx);
    let asked = keeper.expect(Request::ListWorkspaces, cx);
    let one = vec![repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )];
    keeper.reply(asked, Ok(Reply::Workspaces(one.clone())));
    wait_for_list(&list, &one, cx);
    list.read_with(cx, |list, _| {
        // The first of the list is on screen until the user selects another.
        assert_eq!(list.selected(), Some(&id('1')));
        assert_eq!(list.workspace(&id('2')), Some(&one[0].workspaces[1]));
        assert_eq!(list.workspace(&id('9')), None);
    });

    // A list told unasked takes the place of the one before.
    let two = vec![
        one[0].clone(),
        repository('b', vec![workspace('3', "main")]),
    ];
    keeper.tell(Fact::Workspaces(two.clone()));
    wait_for_list(&list, &two, cx);
    assert_eq!(on_screen(&list, cx), Some(id('1')));

    // The workspace on screen leaves the list: the first of the list takes its place.
    let rest = vec![two[1].clone()];
    keeper.tell(Fact::Workspaces(rest.clone()));
    wait_for_list(&list, &rest, cx);
    assert_eq!(on_screen(&list, cx), Some(id('3')));

    keeper.tell(Fact::Workspaces(Vec::new()));
    wait_for_list(&list, &[], cx);
    assert_eq!(on_screen(&list, cx), None);

    list.update(cx, |list, cx| list.refresh(cx));
    keeper.expect(Request::ListWorkspaces, cx);
}

// Many times over: the app runs what is ready in an order of its own each time.
#[gpui_kit::test(iterations = 20)]
fn a_list_asked_for_before_a_newer_one_was_told_does_not_replace_it(cx: &mut TestAppContext) {
    let link = cx.new(|_| KeeperLink::new());
    let list = cx.new(|cx| WorkspaceList::new(link.clone(), cx));
    let mut keeper = KeeperEnd::attach(&link, cx);
    let asked = keeper.expect(Request::ListWorkspaces, cx);

    // What a Keeper sends when a folder is opened while a list is on its way: the answer
    // it made before the folder was opened, then the list with the folder in it.
    let older = vec![repository('a', vec![workspace('1', "main")])];
    let newer = vec![
        older[0].clone(),
        repository('b', vec![workspace('2', "main")]),
    ];
    keeper.reply(asked, Ok(Reply::Workspaces(older)));
    keeper.tell(Fact::Workspaces(newer.clone()));

    wait_for_list(&list, &newer, cx);
    // The answer was read before the fact, so whatever it still had to do is done now.
    cx.run_until_parked();
    list.read_with(cx, |list, _| assert_eq!(list.repositories(), newer));
}

#[gpui_kit::test]
fn the_workspace_on_screen_moves_through_the_list_in_its_order_and_wraps(cx: &mut TestAppContext) {
    let listed = [
        repository('a', vec![workspace('1', "main"), workspace('2', "feature")]),
        repository('b', Vec::new()),
        repository('c', vec![workspace('3', "main")]),
    ];
    let (list, _keeper) = listing(&listed, cx);
    assert_eq!(on_screen(&list, cx), Some(id('1')));

    list.update(cx, |list, cx| list.select(&id('3'), cx));
    assert_eq!(on_screen(&list, cx), Some(id('3')));
    list.update(cx, |list, cx| list.select(&id('9'), cx));
    assert_eq!(
        on_screen(&list, cx),
        Some(id('3')),
        "an id the list does not hold selects nothing"
    );

    let next = |cx: &mut TestAppContext| {
        list.update(cx, |list, cx| list.select_next(cx));
        on_screen(&list, cx)
    };
    assert_eq!(next(cx), Some(id('1')), "after the last comes the first");
    assert_eq!(next(cx), Some(id('2')));
    assert_eq!(next(cx), Some(id('3')));

    let previous = |cx: &mut TestAppContext| {
        list.update(cx, |list, cx| list.select_previous(cx));
        on_screen(&list, cx)
    };
    assert_eq!(previous(cx), Some(id('2')));
    assert_eq!(previous(cx), Some(id('1')));
    assert_eq!(
        previous(cx),
        Some(id('3')),
        "before the first comes the last"
    );
}

#[gpui_kit::test(iterations = 10)]
fn an_opened_folder_comes_on_screen_when_the_list_that_holds_it_arrives(cx: &mut TestAppContext) {
    let before = vec![repository('a', vec![workspace('1', "main")])];
    let (list, mut keeper) = listing(&before, cx);
    // At every change, whether the workspace on screen is one of the list.
    let (seen, _watching) = watch(&list, cx, |list| {
        list.selected()
            .is_none_or(|selected| list.workspace(selected).is_some())
    });

    let folder = PathBuf::from("/gits/opened");
    list.update(cx, |list, cx| list.open_folder(folder.clone(), cx));
    let asked = keeper.expect(Request::OpenFolder { path: folder }, cx);

    // The Keeper answers with the id before it tells the list that holds it.
    let opened = repository('b', vec![workspace('2', "opened")]);
    keeper.reply(
        asked,
        Ok(Reply::Opened {
            repository: opened.id.clone(),
            workspace: id('2'),
        }),
    );
    let after = vec![before[0].clone(), opened];
    keeper.tell(Fact::Workspaces(after.clone()));

    wait_until(cx, "the opened workspace on screen", |cx| {
        on_screen(&list, cx) == Some(id('2'))
    });
    list.read_with(cx, |list, _| assert_eq!(list.repositories(), after));
    assert!(
        seen.borrow().iter().all(|listed| *listed),
        "the workspace on screen was never one the list lacked"
    );
}

#[gpui_kit::test]
fn opening_what_the_list_already_holds_puts_it_on_screen_with_no_new_list(cx: &mut TestAppContext) {
    let listed = [repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )];
    let (list, mut keeper) = listing(&listed, cx);

    let folder = PathBuf::from("/gits/feature");
    list.update(cx, |list, cx| list.open_folder(folder.clone(), cx));
    let asked = keeper.expect(Request::OpenFolder { path: folder }, cx);
    keeper.reply(
        asked,
        Ok(Reply::Opened {
            repository: listed[0].id.clone(),
            workspace: id('2'),
        }),
    );
    wait_until(cx, "the opened workspace on screen", |cx| {
        on_screen(&list, cx) == Some(id('2'))
    });
}

#[gpui_kit::test]
fn a_refusal_says_why_until_the_user_asks_again(cx: &mut TestAppContext) {
    let listed = [repository('a', vec![workspace('1', "main")])];
    let (list, mut keeper) = listing(&listed, cx);
    let refusal = |cx: &mut TestAppContext| list.read_with(cx, |list, _| list.refusal().cloned());
    let wait_for_refusal = |wanted: &Refusal, cx: &mut TestAppContext| {
        wait_until(cx, "the refusal", |cx| {
            list.read_with(cx, |list, _| list.refusal() == Some(wanted))
        })
    };
    assert_eq!(refusal(cx), None);

    let folder = PathBuf::from("/tmp/plain folder");
    list.update(cx, |list, cx| list.open_folder(folder.clone(), cx));
    let asked = keeper.expect(
        Request::OpenFolder {
            path: folder.clone(),
        },
        cx,
    );
    let not_a_repository = Refusal::NotARepository { path: folder };
    keeper.reply(asked, Err(not_a_repository.clone()));
    wait_for_refusal(&not_a_repository, cx);
    assert_eq!(on_screen(&list, cx), Some(id('1')));

    // Asking again takes the old reason away at once, before any answer.
    list.update(cx, |list, cx| {
        list.set_base("no-such-branch".to_owned(), cx)
    });
    assert_eq!(refusal(cx), None);
    let asked = keeper.expect(
        Request::SetBase {
            workspace: id('1'),
            base: "no-such-branch".to_owned(),
        },
        cx,
    );
    let unknown = Refusal::UnknownRevision {
        revision: "no-such-branch".to_owned(),
    };
    keeper.reply(asked, Err(unknown.clone()));
    wait_for_refusal(&unknown, cx);

    // An answer of another kind than was asked for is a failure the user is told of.
    list.update(cx, |list, cx| list.set_base("main".to_owned(), cx));
    assert_eq!(refusal(cx), None);
    let asked = keeper.expect(
        Request::SetBase {
            workspace: id('1'),
            base: "main".to_owned(),
        },
        cx,
    );
    keeper.reply(asked, Ok(Reply::Workspaces(Vec::new())));
    wait_for_refusal(
        &Refusal::Failed {
            reason: AskError::Unexpected.to_string(),
        },
        cx,
    );
}

#[gpui_kit::test]
fn no_base_is_set_while_no_workspace_is_on_screen(cx: &mut TestAppContext) {
    let (list, mut keeper) = listing(&[], cx);

    list.update(cx, |list, cx| {
        list.set_base("main".to_owned(), cx);
        list.refresh(cx);
    });
    // Had a base been asked for, that request would have come first.
    keeper.expect(Request::ListWorkspaces, cx);
}

#[gpui_kit::test]
fn the_changes_of_the_workspace_on_screen_are_asked_for_and_kept_until_newer_ones_come(
    cx: &mut TestAppContext,
) {
    let listed = [repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )];
    let (models, mut keeper) = models_listing(&listed, cx);
    let model = models.changes.clone();

    let asked = keeper.expect(get_changes('1'), cx);
    model.read_with(cx, |model, _| {
        assert_eq!(model.workspace(), Some(&id('1')));
        assert_eq!(model.changes(), None);
        assert!(model.is_loading());
    });
    let first = changes('1', Some(COMMIT), vec![file("a.rs", FileStatus::Modified)]);
    keeper.reply(asked, Ok(Reply::Changes(first.clone())));
    wait_for_changes(&model, &first, cx);
    model.read_with(cx, |model, _| assert!(!model.is_loading()));

    // The changes of a workspace that is not on screen are not asked for: a request
    // for them would come before the one for the workspace that is.
    keeper.tell(Fact::ChangesChanged { workspace: id('2') });
    keeper.tell(Fact::ChangesChanged { workspace: id('1') });
    let asked = keeper.expect(get_changes('1'), cx);
    model.read_with(cx, |model, _| {
        assert_eq!(
            model.changes(),
            Some(&first),
            "the last answer shows meanwhile"
        );
        assert!(model.is_loading());
    });
    let second = changes('1', Some(COMMIT), vec![file("b.rs", FileStatus::Added)]);
    keeper.reply(asked, Ok(Reply::Changes(second.clone())));
    wait_for_changes(&model, &second, cx);

    model.update(cx, |model, cx| model.refresh(cx));
    keeper.expect(get_changes('1'), cx);
}

#[gpui_kit::test(iterations = 10)]
fn another_workspace_on_screen_never_shows_the_changes_of_the_one_before(cx: &mut TestAppContext) {
    let listed = [repository(
        'a',
        vec![workspace('1', "main"), workspace('2', "feature")],
    )];
    let (models, mut keeper) = models_listing(&listed, cx);
    let model = models.changes.clone();
    let asked = keeper.expect(get_changes('1'), cx);
    let of_first = changes('1', Some(COMMIT), Vec::new());
    keeper.reply(asked, Ok(Reply::Changes(of_first.clone())));
    wait_for_changes(&model, &of_first, cx);

    // At every change, whether the changes shown are those of the workspace on screen.
    let (seen, _watching) = watch(&model, cx, |model| {
        model
            .changes()
            .is_none_or(|changes| Some(&changes.workspace) == model.workspace())
    });

    // An answer for the first workspace is still on its way when the second comes on
    // screen.
    model.update(cx, |model, cx| model.refresh(cx));
    let late = keeper.expect(get_changes('1'), cx);
    models
        .workspaces
        .update(cx, |list, cx| list.select(&id('2'), cx));
    let asked = keeper.expect(get_changes('2'), cx);
    model.read_with(cx, |model, _| {
        assert_eq!(model.workspace(), Some(&id('2')));
        assert_eq!(model.changes(), None);
        assert!(model.is_loading());
    });

    keeper.reply(late, Ok(Reply::Changes(of_first)));
    let of_second = changes('2', Some(COMMIT), Vec::new());
    keeper.reply(asked, Ok(Reply::Changes(of_second.clone())));
    wait_for_changes(&model, &of_second, cx);
    cx.run_until_parked();
    model.read_with(cx, |model, _| assert_eq!(model.changes(), Some(&of_second)));
    assert!(
        seen.borrow().iter().all(|own| *own),
        "the changes shown were always those of the workspace on screen"
    );
}

#[gpui_kit::test]
fn refused_changes_say_why_until_the_keeper_answers(cx: &mut TestAppContext) {
    let listed = [repository('a', vec![workspace('1', "main")])];
    let (models, mut keeper) = models_listing(&listed, cx);
    let model = models.changes.clone();

    let asked = keeper.expect(get_changes('1'), cx);
    let no_git = Refusal::Failed {
        reason: "git was not found".to_owned(),
    };
    keeper.reply(asked, Err(no_git.clone()));
    wait_until(cx, "the refusal", |cx| {
        model.read_with(cx, |model, _| model.refusal() == Some(&no_git))
    });
    model.read_with(cx, |model, _| {
        assert!(!model.is_loading());
        assert_eq!(model.changes(), None);
    });

    keeper.tell(Fact::ChangesChanged { workspace: id('1') });
    let asked = keeper.expect(get_changes('1'), cx);
    let answered = changes('1', None, Vec::new());
    keeper.reply(asked, Ok(Reply::Changes(answered.clone())));
    wait_for_changes(&model, &answered, cx);
    model.read_with(cx, |model, _| assert_eq!(model.refusal(), None));
}

#[gpui_kit::test]
fn a_file_is_asked_for_in_the_commit_its_changes_are_counted_from(cx: &mut TestAppContext) {
    let listed = [repository('a', vec![workspace('1', "main")])];
    let (models, mut keeper) = models_listing(&listed, cx);
    let model = models.changes.clone();
    let ask = |file: &ChangedFile, cx: &mut TestAppContext| {
        model.update(cx, |model, cx| model.file(file, cx))
    };
    let read_file = |since: Option<&str>, old_path: &str, path: &str| Request::ReadFile {
        workspace: id('1'),
        since: since.map(str::to_owned),
        old_path: PathBuf::from(old_path),
        path: PathBuf::from(path),
    };
    let sides = FileSides {
        old: Content::Text(b"before\n".to_vec()),
        new: Content::Text(b"now\n".to_vec()),
    };

    // Before the changes are here there is no file of them to ask for, and nothing is
    // sent: such a request would be the next the Keeper reads below.
    let asked = keeper.expect(get_changes('1'), cx);
    let early = ask(&file("early.rs", FileStatus::Modified), cx);
    assert!(matches!(answer(early, cx), Err(AskError::Refused(_))));

    let added = file("src/new.rs", FileStatus::Added);
    let modified = file("src/lib.rs", FileStatus::Modified);
    let deleted = file("src/old.rs", FileStatus::Deleted);
    let renamed = file(
        "src/moved.rs",
        FileStatus::Renamed {
            from: PathBuf::from("src/was here.rs"),
        },
    );
    let shown = changes(
        '1',
        Some(COMMIT),
        vec![
            added.clone(),
            modified.clone(),
            deleted.clone(),
            renamed.clone(),
        ],
    );
    keeper.reply(asked, Ok(Reply::Changes(shown.clone())));
    wait_for_changes(&model, &shown, cx);

    // A new file has no old version, and none is asked for.
    let of_added = ask(&added, cx);
    let asked = keeper.expect(read_file(None, "src/new.rs", "src/new.rs"), cx);
    let only_new = FileSides {
        old: Content::Absent,
        new: Content::Text(b"new\n".to_vec()),
    };
    keeper.reply(asked, Ok(Reply::File(only_new.clone())));
    assert_eq!(answer(of_added, cx), Ok(only_new));

    let of_modified = ask(&modified, cx);
    let asked = keeper.expect(read_file(Some(COMMIT), "src/lib.rs", "src/lib.rs"), cx);
    keeper.reply(asked, Ok(Reply::File(sides.clone())));
    assert_eq!(answer(of_modified, cx), Ok(sides));

    // A refusal comes back as the Keeper worded it.
    let of_deleted = ask(&deleted, cx);
    let asked = keeper.expect(read_file(Some(COMMIT), "src/old.rs", "src/old.rs"), cx);
    let refusal = Refusal::UnknownRevision {
        revision: COMMIT.to_owned(),
    };
    keeper.reply(asked, Err(refusal.clone()));
    assert_eq!(answer(of_deleted, cx), Err(AskError::Refused(refusal)));

    // A renamed file is read at the path it had. And an answer of another kind than a
    // file is not passed on as one.
    let of_renamed = ask(&renamed, cx);
    let asked = keeper.expect(
        read_file(Some(COMMIT), "src/was here.rs", "src/moved.rs"),
        cx,
    );
    keeper.reply(asked, Ok(Reply::Done));
    assert_eq!(answer(of_renamed, cx), Err(AskError::Unexpected));

    // Where nothing is committed yet, the changes are counted from no commit and no
    // old version is asked for, whatever happened to the file.
    keeper.tell(Fact::ChangesChanged { workspace: id('1') });
    let asked = keeper.expect(get_changes('1'), cx);
    let uncommitted = changes('1', None, vec![modified.clone()]);
    keeper.reply(asked, Ok(Reply::Changes(uncommitted.clone())));
    wait_for_changes(&model, &uncommitted, cx);
    let _of_modified = ask(&modified, cx);
    keeper.expect(read_file(None, "src/lib.rs", "src/lib.rs"), cx);
}
