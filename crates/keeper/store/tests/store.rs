//! Component tests: the Store through the calls the parts of the Keeper use, on a real
//! database file.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_store::{
    FILE_NAME, RepositoryId, RepositoryRecord, Store, StoreError, WorkspaceId, WorkspaceRecord,
};

/// A folder of the test's own, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!("yaar-store-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("make the scratch folder");
        Self(folder)
    }

    fn file(&self) -> PathBuf {
        self.0.join(FILE_NAME)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repository(id: &str, git_dir: &str) -> RepositoryRecord {
    RepositoryRecord {
        id: RepositoryId::new(id),
        git_dir: PathBuf::from(git_dir),
    }
}

fn workspace(id: &str, repository: &str, path: &str, base: Option<&str>) -> WorkspaceRecord {
    WorkspaceRecord {
        id: WorkspaceId::new(id),
        repository: RepositoryId::new(repository),
        path: PathBuf::from(path),
        base: base.map(str::to_owned),
    }
}

fn put_workspace(store: &Store, id: &str, repository: &str, path: &str) {
    store
        .put_workspace(
            &WorkspaceId::new(id),
            &RepositoryId::new(repository),
            Path::new(path),
        )
        .expect("put workspace");
}

#[test]
fn what_is_kept_is_found_again_in_the_order_it_was_first_kept() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    assert_eq!(store.repositories().expect("repositories"), vec![]);
    assert_eq!(store.workspaces().expect("workspaces"), vec![]);

    // Kept in an order that no sorting of ids or paths gives.
    let second = repository("r-b", "/repos/second/.git");
    let first = repository("r-a", "/repos/first/.git");
    store.put_repository(&second).expect("put repository");
    store.put_repository(&first).expect("put repository");
    put_workspace(&store, "w-2", "r-b", "/repos/second");
    put_workspace(&store, "w-3", "r-a", "/repos/first");
    put_workspace(&store, "w-1", "r-b", "/repos/second-linked");
    store
        .set_base(&WorkspaceId::new("w-3"), "origin/develop")
        .expect("set base");

    assert_eq!(
        store.repositories().expect("repositories"),
        vec![second, first]
    );
    assert_eq!(
        store.workspaces().expect("workspaces"),
        vec![
            workspace("w-2", "r-b", "/repos/second", None),
            workspace("w-3", "r-a", "/repos/first", Some("origin/develop")),
            workspace("w-1", "r-b", "/repos/second-linked", None),
        ]
    );
}

#[test]
fn a_store_opened_later_on_the_same_file_holds_what_the_earlier_one_was_given() {
    let scratch = Scratch::new();
    {
        let store = Store::open(&scratch.file()).expect("open");
        let clone = store.clone();
        store
            .put_repository(&repository("r-a", "/repos/a/.git"))
            .expect("put repository");
        put_workspace(&clone, "w-1", "r-a", "/repos/a");
        clone
            .set_base(&WorkspaceId::new("w-1"), "main")
            .expect("set base");
    }
    // With the last clone dropped the file is closed: it alone, copied to another
    // place with nothing beside it, is the whole database.
    let elsewhere = Scratch::new();
    std::fs::copy(scratch.file(), elsewhere.file()).expect("copy the file");

    for file in [scratch.file(), elsewhere.file()] {
        let store = Store::open(&file).expect("open again");
        assert_eq!(
            store.repositories().expect("repositories"),
            vec![repository("r-a", "/repos/a/.git")]
        );
        assert_eq!(
            store.workspaces().expect("workspaces"),
            vec![workspace("w-1", "r-a", "/repos/a", Some("main"))]
        );
    }
}

#[test]
fn keeping_what_is_known_brings_it_up_to_date_and_leaves_its_place_and_its_base() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    store
        .put_repository(&repository("r-a", "/old/a/.git"))
        .expect("put repository");
    store
        .put_repository(&repository("r-b", "/old/b/.git"))
        .expect("put repository");
    put_workspace(&store, "w-1", "r-a", "/old/a");
    put_workspace(&store, "w-2", "r-b", "/old/b");
    store
        .set_base(&WorkspaceId::new("w-1"), "release")
        .expect("set base");

    // The first repository moved, and its workspace with it, into the other one.
    store
        .put_repository(&repository("r-a", "/new/a/.git"))
        .expect("put repository");
    put_workspace(&store, "w-1", "r-b", "/new/a");

    assert_eq!(
        store.repositories().expect("repositories"),
        vec![
            repository("r-a", "/new/a/.git"),
            repository("r-b", "/old/b/.git")
        ]
    );
    assert_eq!(
        store.workspaces().expect("workspaces"),
        vec![
            workspace("w-1", "r-b", "/new/a", Some("release")),
            workspace("w-2", "r-b", "/old/b", None),
        ]
    );

    // A base set again replaces the one before.
    store
        .set_base(&WorkspaceId::new("w-1"), "0123abcd")
        .expect("set base");
    assert_eq!(
        store.workspaces().expect("workspaces")[0].base.as_deref(),
        Some("0123abcd")
    );
}

#[test]
fn a_base_for_a_workspace_the_store_does_not_hold_is_left_alone() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    store
        .put_repository(&repository("r-a", "/repos/a/.git"))
        .expect("put repository");
    put_workspace(&store, "w-1", "r-a", "/repos/a");

    store
        .set_base(&WorkspaceId::new("w-unknown"), "main")
        .expect("no failure");
    assert_eq!(
        store.workspaces().expect("workspaces"),
        vec![workspace("w-1", "r-a", "/repos/a", None)]
    );
}

#[test]
fn a_workspace_of_a_repository_the_store_does_not_hold_is_refused() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    let refused = store.put_workspace(
        &WorkspaceId::new("w-1"),
        &RepositoryId::new("r-unknown"),
        Path::new("/repos/a"),
    );
    assert!(
        matches!(refused, Err(StoreError::Database(_))),
        "{refused:?}"
    );
    assert_eq!(store.workspaces().expect("workspaces"), vec![]);
}

#[test]
fn a_path_that_is_not_text_is_kept_byte_for_byte() {
    let scratch = Scratch::new();
    let odd = PathBuf::from(OsStr::from_bytes(b"/repos/\xff\xfe odd\n'name\""));
    {
        let store = Store::open(&scratch.file()).expect("open");
        store
            .put_repository(&RepositoryRecord {
                id: RepositoryId::new("r-a"),
                git_dir: odd.join(".git"),
            })
            .expect("put repository");
        store
            .put_workspace(&WorkspaceId::new("w-1"), &RepositoryId::new("r-a"), &odd)
            .expect("put workspace");
    }
    let store = Store::open(&scratch.file()).expect("open again");
    assert_eq!(
        store.repositories().expect("repositories")[0].git_dir,
        odd.join(".git")
    );
    assert_eq!(store.workspaces().expect("workspaces")[0].path, odd);
}

#[test]
fn the_file_and_the_folders_above_it_are_made_when_they_are_not_there() {
    let scratch = Scratch::new();
    let file = scratch.0.join("app/data").join(FILE_NAME);
    let store = Store::open(&file).expect("open");
    store
        .put_repository(&repository("r-a", "/repos/a/.git"))
        .expect("put repository");
    assert!(file.is_file());
}

#[test]
fn a_file_that_is_no_database_is_refused_and_left_as_it_is() {
    let scratch = Scratch::new();
    let notes = b"These are notes somebody saved under the name of the database.\n".repeat(40);
    std::fs::write(scratch.file(), &notes).expect("write the file");

    match Store::open(&scratch.file()) {
        Err(StoreError::Open { path, reason }) => {
            assert_eq!(path, scratch.file());
            assert!(!reason.is_empty());
        }
        other => panic!("not refused as a file that cannot be opened: {other:?}"),
    }
    assert_eq!(std::fs::read(scratch.file()).expect("read the file"), notes);

    // Nor is a place where no file can be made.
    let under_a_file = scratch.file().join("below").join(FILE_NAME);
    assert!(matches!(
        Store::open(&under_a_file),
        Err(StoreError::Open { .. })
    ));
}

#[test]
fn clones_on_several_threads_reach_the_one_writer() {
    let scratch = Scratch::new();
    let store = Store::open(&scratch.file()).expect("open");
    store
        .put_repository(&repository("r-a", "/repos/a/.git"))
        .expect("put repository");

    std::thread::scope(|threads| {
        for thread in 0..6 {
            let store = store.clone();
            threads.spawn(move || {
                for n in 0..10 {
                    let id = format!("w-{thread}-{n}");
                    put_workspace(&store, &id, "r-a", &format!("/repos/a-{thread}-{n}"));
                    store
                        .set_base(&WorkspaceId::new(&id), &format!("base-{thread}-{n}"))
                        .expect("set base");
                    // What a call has returned is there for every later call.
                    let kept = store.workspaces().expect("workspaces");
                    let mine = kept.iter().find(|w| w.id.as_str() == id).expect("kept");
                    assert_eq!(mine.base, Some(format!("base-{thread}-{n}")));
                }
            });
        }
    });

    drop(store);
    let store = Store::open(&scratch.file()).expect("open again");
    let kept = store.workspaces().expect("workspaces");
    assert_eq!(kept.len(), 60);
    for thread in 0..6 {
        // Each thread's own are in the order it kept them.
        let mine: Vec<&str> = kept
            .iter()
            .map(|w| w.id.as_str())
            .filter(|id| id.starts_with(&format!("w-{thread}-")))
            .collect();
        let expected: Vec<String> = (0..10).map(|n| format!("w-{thread}-{n}")).collect();
        assert_eq!(mine, expected);
    }
}
