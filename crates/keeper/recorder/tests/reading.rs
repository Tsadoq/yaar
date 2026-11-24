//! Component tests: the changed paths and the contents of files, read through the surface
//! the rest of the Keeper uses, in real repositories the tests make.

mod support;

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use support::{
    Scratch, an_older_git, commit_all, git, git_bytes, git_that_keeps_a_list, git_with_input, init,
    is_alive, program, touch, write,
};
use yaar_recorder::{Change, ChangedPath, CommitId, Git, Worktree};

fn open(folder: &Path) -> Worktree {
    Worktree::open(&Git::find().expect("git on the search path"), folder).expect("open")
}

fn changed(path: &str, change: Change) -> ChangedPath {
    ChangedPath {
        path: PathBuf::from(path),
        change,
    }
}

fn renamed(path: &str, from: &str) -> ChangedPath {
    changed(
        path,
        Change::Renamed {
            from: PathBuf::from(from),
        },
    )
}

/// The fields of an answer git ends with zero bytes.
fn fields(answer: &[u8]) -> Vec<&[u8]> {
    answer
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect()
}

fn path_of(bytes: &[u8]) -> PathBuf {
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

/// What git itself says differs between `since` and the worktree: its diff of the
/// commit against the files, with its pairing of renames, and the files it lists as
/// untracked. Read by the test's own run of git, ordered as git orders paths.
fn gits_own_changes(repo: &Path, since: &str) -> Vec<ChangedPath> {
    let mut all = Vec::new();
    let diff = git_bytes(repo, &["diff", "--name-status", "-z", "-M", since]);
    let mut fields_of_diff = fields(&diff).into_iter();
    while let Some(status) = fields_of_diff.next() {
        let mut next = || path_of(fields_of_diff.next().expect("a path after a status"));
        let entry = match status[0] {
            b'A' => ChangedPath {
                path: next(),
                change: Change::Added,
            },
            b'M' => ChangedPath {
                path: next(),
                change: Change::Modified,
            },
            b'D' => ChangedPath {
                path: next(),
                change: Change::Deleted,
            },
            b'R' => {
                let from = next();
                ChangedPath {
                    path: next(),
                    change: Change::Renamed { from },
                }
            }
            other => panic!("a status this test does not make: {}", other as char),
        };
        all.push(entry);
    }
    let untracked = git_bytes(repo, &["ls-files", "--others", "--exclude-standard", "-z"]);
    all.extend(fields(&untracked).into_iter().map(|path| ChangedPath {
        path: path_of(path),
        change: Change::Added,
    }));
    all.sort_by(|a, b| a.path.as_os_str().cmp(b.path.as_os_str()));
    all
}

/// The id git gives the content of each side of each path its diff lists: old then new.
/// The new id is all zeros where the file on disk is not what the index holds.
fn gits_own_blobs(repo: &Path, since: &str) -> Vec<(PathBuf, String, String)> {
    let raw = git_bytes(repo, &["diff", "--raw", "-z", "--no-abbrev", "-M", since]);
    let mut blobs = Vec::new();
    let mut fields_of_raw = fields(&raw).into_iter();
    while let Some(header) = fields_of_raw.next() {
        let header = std::str::from_utf8(header).expect("a header of text");
        let parts: Vec<&str> = header.split(' ').collect();
        let (old, new, status) = (parts[2], parts[3], parts[4]);
        if status.starts_with('R') {
            fields_of_raw.next().expect("the old path of a rename");
        }
        let path = path_of(fields_of_raw.next().expect("a path"));
        blobs.push((path, old.to_owned(), new.to_owned()));
    }
    blobs
}

/// The id git would give this content.
fn blob_id(repo: &Path, content: &[u8]) -> String {
    git_with_input(repo, &["hash-object", "--stdin"], content)
}

// WS-01. The test repository has no `autocrlf` and no clean filter. With either, the app
// shows differences git's own diff would not: the new side of a file is its bytes on
// disk, read as they are, because running the filters would run programs the
// configuration names.
#[test]
fn the_changes_since_the_fork_point_are_those_of_gits_own_diff_with_what_is_uncommitted_and_untracked()
 {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "kept.txt", "kept\n");
    write(&repo, "edited.txt", "one\ntwo\nthree\n");
    write(&repo, "gone.txt", "soon gone\n");
    write(&repo, "notes.txt", "a note\n");
    write(
        &repo,
        "old-name.txt",
        "a file\nlong enough\nfor git\nto pair it\nwith its new name\n",
    );
    commit_all(&repo, "start");

    git(&repo, &["switch", "--quiet", "-c", "feat"]);
    write(&repo, "edited.txt", "one\n2\nthree\n");
    git(&repo, &["rm", "--quiet", "gone.txt"]);
    git(&repo, &["mv", "old-name.txt", "new-name.txt"]);
    write(&repo, "added.txt", "new on the branch\n");
    commit_all(&repo, "work on the branch");

    // The base moves on after the branch left it: counted from its tip, `kept.txt`
    // would show as changed and `main-only.txt` as deleted.
    git(&repo, &["switch", "--quiet", "main"]);
    write(&repo, "kept.txt", "kept, and changed on main\n");
    write(&repo, "main-only.txt", "never on the branch\n");
    let main = commit_all(&repo, "main moves on");
    git(&repo, &["switch", "--quiet", "feat"]);

    // Not committed: an edit that is not staged, a new file that is, two files git does
    // not track, and one it ignores.
    write(&repo, "notes.txt", "a note, edited\n");
    write(&repo, "staged.txt", "staged\n");
    git(&repo, &["add", "staged.txt"]);
    write(&repo, "untracked.txt", "untracked\n");
    write(&repo, "notes/todo.md", "untracked, in a folder\n");
    write(&repo, "build.log", "ignored\n");

    let worktree = open(&repo);
    let base = worktree.resolve("main").expect("resolve").expect("main");
    assert_eq!(base.as_str(), main);
    let fork = worktree
        .fork_point(&base)
        .expect("fork point")
        .expect("a shared commit");
    assert_eq!(fork.as_str(), git(&repo, &["merge-base", "HEAD", "main"]));
    assert_ne!(fork, base, "the base moved on after the branch left it");

    // The paths: written out by hand, and the same as git's own.
    let paths = worktree.changed_paths(Some(&fork)).expect("changed paths");
    assert_eq!(
        paths,
        vec![
            changed("added.txt", Change::Added),
            changed("edited.txt", Change::Modified),
            changed("gone.txt", Change::Deleted),
            renamed("new-name.txt", "old-name.txt"),
            // Ordered as git orders paths, byte by byte: `.` comes before `/`.
            changed("notes.txt", Change::Modified),
            changed("notes/todo.md", Change::Added),
            changed("staged.txt", Change::Added),
            changed("untracked.txt", Change::Added),
        ]
    );
    assert_eq!(paths, gits_own_changes(&repo, fork.as_str()));

    // The contents. The old side is the file in the fork point, under the path it had
    // there; the new side is the file on disk.
    let blobs = gits_own_blobs(&repo, fork.as_str());
    for entry in &paths {
        let old_path = match &entry.change {
            Change::Renamed { from } => from,
            _ => &entry.path,
        };
        let old = worktree.content_at(&fork, old_path).expect("old side");
        let new = worktree.content_on_disk(&entry.path).expect("new side");

        assert_eq!(old.is_none(), entry.change == Change::Added, "{entry:?}");
        assert_eq!(new.is_none(), entry.change == Change::Deleted, "{entry:?}");
        if let Some(new) = &new {
            let on_disk = std::fs::read(repo.join(&entry.path)).expect("read the file");
            assert_eq!(new, &on_disk, "{entry:?}");
        }

        // The same contents as git's own diff has: it names each side by the id of its
        // content.
        let listed = blobs.iter().find(|(path, _, _)| path == &entry.path);
        let Some((_, old_id, new_id)) = listed else {
            // git's diff does not hold what it does not track.
            assert!(["untracked.txt", "notes/todo.md"].contains(&entry.path.to_str().unwrap()));
            continue;
        };
        if let Some(old) = &old {
            assert_eq!(&blob_id(&repo, old), old_id, "{entry:?}");
        }
        if let Some(new) = &new {
            let id = blob_id(&repo, new);
            let path = entry.path.to_str().unwrap();
            assert_eq!(id, git(&repo, &["hash-object", "--", path]), "{entry:?}");
            if new_id.bytes().any(|digit| digit != b'0') {
                assert_eq!(&id, new_id, "{entry:?}");
            }
        }
    }

    // And the old sides say what was written above, not only what git agrees with.
    let old = |path: &str| {
        worktree
            .content_at(&fork, Path::new(path))
            .expect("old side")
    };
    assert_eq!(
        old("edited.txt").as_deref(),
        Some(&b"one\ntwo\nthree\n"[..])
    );
    assert_eq!(old("notes.txt").as_deref(), Some(&b"a note\n"[..]));
    assert_eq!(old("kept.txt").as_deref(), Some(&b"kept\n"[..]));
    assert_eq!(old("main-only.txt"), None);
}

#[test]
fn with_nothing_committed_every_file_on_disk_that_git_does_not_ignore_is_new() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "*.log\n");
    write(&repo, "plain.txt", "never staged\n");
    write(&repo, "staged.txt", "staged\n");
    write(&repo, "staged-and-gone.txt", "staged, then deleted\n");
    write(&repo, "build.log", "ignored\n");
    git(&repo, &["add", "staged.txt", "staged-and-gone.txt"]);
    std::fs::remove_file(repo.join("staged-and-gone.txt")).expect("delete");

    let worktree = open(&repo);
    assert_eq!(
        worktree.changed_paths(None).expect("changed paths"),
        vec![
            changed(".gitignore", Change::Added),
            changed("plain.txt", Change::Added),
            changed("staged.txt", Change::Added),
        ]
    );
}

#[test]
fn a_file_that_was_only_touched_is_no_change_and_gits_list_of_the_files_is_left_as_it_was() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "the same\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    touch(&repo, "a.txt");
    let index = repo.join(".git/index");
    let before = std::fs::read(&index).expect("read the index");

    let worktree = open(&repo);
    assert_eq!(worktree.changed_paths(Some(&head)).expect("paths"), vec![]);
    assert_eq!(std::fs::read(&index).expect("read the index"), before);

    // The same question, asked as the user's git asks it, does rewrite the list: that
    // is what the call above must not do.
    assert_eq!(git(&repo, &["diff", "--name-status", head.as_str()]), "");
    assert_ne!(std::fs::read(&index).expect("read the index"), before);
}

#[test]
fn no_hook_and_no_monitor_of_the_user_runs_when_the_changes_are_read() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "the same\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    touch(&repo, "a.txt");
    // Opened before the user defines any of them, as a Keeper that runs for days has.
    let worktree = open(&repo);

    // Three programs of the user that git starts when it reads the files and brings its
    // list of them up to date: a hook in the hooks folder, a hook the configuration
    // defines (git 2.54 and later; an older git knows no such hook), and a monitor of
    // the file system. Each writes its name down when it runs.
    let ran = scratch.join("ran");
    let note = |name: &str| format!("echo {name} >> '{}'", ran.display());
    program(&repo.join(".git/hooks/post-index-change"), &note("hook"));
    program(
        &scratch.join("monitor"),
        &format!("{}\nprintf 'token\\0/\\0'", note("monitor")),
    );
    let set = |key: &str, value: &str| {
        let config = repo.join(".git/config");
        let mut text = std::fs::read_to_string(&config).expect("read the configuration");
        text.push_str(&format!("{key}{value}\n"));
        std::fs::write(config, text).expect("write the configuration");
    };
    // Written into the file, not through git, which would start the programs itself.
    set(
        "[core]\n\tfsmonitor = ",
        scratch.join("monitor").to_str().unwrap(),
    );
    set(
        "[hook \"told\"]\n\tevent = post-index-change\n\tcommand = ",
        &note("told"),
    );

    assert_eq!(worktree.changed_paths(Some(&head)).expect("paths"), vec![]);
    assert_eq!(worktree.changed_paths(None).expect("paths").len(), 1);
    assert!(!ran.exists(), "ran: {:?}", std::fs::read_to_string(&ran));

    // The same reading by the user's own git starts them: they were there to be run.
    git(&repo, &["diff", "--name-status", head.as_str()]);
    let ran = std::fs::read_to_string(&ran).expect("the programs ran");
    assert!(
        ran.contains("hook") && ran.contains("monitor"),
        "ran: {ran:?}"
    );
}

#[test]
fn a_file_taken_out_of_gits_list_and_still_on_disk_is_one_changed_file() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "kept on disk\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    git(&repo, &["rm", "--quiet", "--cached", "a.txt"]);

    let worktree = open(&repo);
    assert_eq!(
        worktree.changed_paths(Some(&head)).expect("paths"),
        vec![changed("a.txt", Change::Modified)]
    );
}

#[test]
fn a_link_is_read_as_the_path_it_points_at_and_is_not_followed() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "target.txt", "what the link points at\n");
    std::os::unix::fs::symlink("target.txt", repo.join("link")).expect("link");
    let head = CommitId::new(commit_all(&repo, "start"));
    let worktree = open(&repo);
    assert_eq!(
        worktree.content_at(&head, Path::new("link")).expect("old"),
        Some(b"target.txt".to_vec())
    );
    assert_eq!(
        worktree.content_on_disk(Path::new("link")).expect("new"),
        Some(b"target.txt".to_vec())
    );

    // Pointed at a file outside the worktree, it is still its own words.
    write(
        scratch.path(),
        "outside/secret.txt",
        "not of the worktree\n",
    );
    std::fs::remove_file(repo.join("link")).expect("remove the link");
    std::os::unix::fs::symlink("../outside/secret.txt", repo.join("link")).expect("link");
    assert_eq!(
        worktree.content_on_disk(Path::new("link")).expect("new"),
        Some(b"../outside/secret.txt".to_vec())
    );
    assert_eq!(
        worktree.changed_paths(Some(&head)).expect("paths"),
        vec![changed("link", Change::Modified)]
    );
}

#[test]
fn a_path_that_leads_out_of_the_worktree_holds_no_file() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "inside.txt", "inside\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    write(
        scratch.path(),
        "outside/secret.txt",
        "not of the worktree\n",
    );
    std::os::unix::fs::symlink("../outside", repo.join("way-out")).expect("link");
    let outside = scratch.join("outside/secret.txt");
    assert!(
        repo.join("way-out/secret.txt").exists(),
        "the link leads there"
    );

    let worktree = open(&repo);
    for path in [
        Path::new("../outside/secret.txt"),
        Path::new("sub/../../outside/secret.txt"),
        outside.as_path(),
        Path::new("way-out/secret.txt"),
        Path::new(""),
    ] {
        assert_eq!(
            worktree.content_on_disk(path).expect("new"),
            None,
            "{path:?}"
        );
        assert_eq!(
            worktree.content_at(&head, path).expect("old"),
            None,
            "{path:?}"
        );
    }
    // A way of writing the path that stays inside is the path.
    assert_eq!(
        worktree
            .content_on_disk(Path::new("./inside.txt"))
            .expect("new"),
        Some(b"inside\n".to_vec())
    );
    assert_eq!(
        worktree
            .content_at(&head, Path::new("./inside.txt"))
            .expect("old"),
        Some(b"inside\n".to_vec())
    );
}

#[test]
fn a_repository_inside_the_worktree_is_one_path_and_is_not_looked_into() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    let inner = repo.join("vendor/inner");
    init(&inner);
    write(&inner, "deep/file.txt", "of the inner repository\n");
    commit_all(&inner, "inner");

    let worktree = open(&repo);
    assert_eq!(
        worktree.changed_paths(Some(&head)).expect("paths"),
        vec![changed("vendor/inner", Change::Added)]
    );
    // The path is a folder: there is no file to read on either side.
    assert_eq!(
        worktree
            .content_on_disk(Path::new("vendor/inner"))
            .expect("new"),
        None
    );
    assert_eq!(
        worktree
            .content_at(&head, Path::new("vendor/inner"))
            .expect("old"),
        None
    );
    // A folder of the commit is no file either.
    write(&repo, "folder/b.txt", "b\n");
    git(&repo, &["add", "folder"]);
    let head = CommitId::new(commit_all(&repo, "a folder"));
    assert_eq!(
        worktree
            .content_at(&head, Path::new("folder"))
            .expect("old"),
        None
    );
}

#[test]
fn a_path_with_a_space_a_quote_and_a_line_feed_is_read_as_it_is() {
    a_path_with_a_space_a_quote_and_a_line_feed_is_read_as_it_is_with(&Scratch::new());
}

#[test]
fn a_path_with_a_space_a_quote_and_a_line_feed_is_read_as_it_is_on_an_older_git() {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    a_path_with_a_space_a_quote_and_a_line_feed_is_read_as_it_is_with(&scratch);
}

fn a_path_with_a_space_a_quote_and_a_line_feed_is_read_as_it_is_with(scratch: &Scratch) {
    let repo = scratch.join("repo");
    init(&repo);
    let odd = "an \"odd\" name\nwith a second line.txt";
    write(&repo, odd, "before\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    write(&repo, odd, "after\n");
    write(&repo, "also new\n.txt", "new\n");

    let worktree = open(&repo);
    assert_eq!(
        worktree.changed_paths(Some(&head)).expect("paths"),
        vec![
            changed("also new\n.txt", Change::Added),
            changed(odd, Change::Modified),
        ]
    );
    assert_eq!(
        worktree.content_at(&head, Path::new(odd)).expect("old"),
        Some(b"before\n".to_vec())
    );
    assert_eq!(
        worktree.content_on_disk(Path::new(odd)).expect("new"),
        Some(b"after\n".to_vec())
    );
    // A name with a line feed that the commit lacks is told apart from one it has.
    assert_eq!(
        worktree
            .content_at(&head, Path::new("also new\n.txt"))
            .expect("old"),
        None
    );
    assert_eq!(
        worktree.content_at(&head, Path::new(odd)).expect("old"),
        Some(b"before\n".to_vec())
    );
}

#[test]
fn a_path_that_ends_in_a_carriage_return_is_read_as_it_is() {
    a_path_that_ends_in_a_carriage_return_is_read_as_it_is_with(&Scratch::new());
}

#[test]
fn a_path_that_ends_in_a_carriage_return_is_read_as_it_is_on_an_older_git() {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    a_path_that_ends_in_a_carriage_return_is_read_as_it_is_with(&scratch);
}

fn a_path_that_ends_in_a_carriage_return_is_read_as_it_is_with(scratch: &Scratch) {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "ends\r", "with a return\n");
    write(&repo, "ends", "without\n");
    let head = CommitId::new(commit_all(&repo, "start"));

    let worktree = open(&repo);
    assert_eq!(
        worktree
            .content_at(&head, Path::new("ends\r"))
            .expect("with"),
        Some(b"with a return\n".to_vec())
    );
    assert_eq!(
        worktree
            .content_at(&head, Path::new("ends"))
            .expect("without"),
        Some(b"without\n".to_vec())
    );
    // A name that is not there is told apart from the one that is, with the return too.
    assert_eq!(
        worktree
            .content_at(&head, Path::new("nothing\r"))
            .expect("none"),
        None
    );
}

#[test]
fn a_commit_git_does_not_know_and_a_path_the_commit_lacks_both_give_no_content() {
    a_commit_git_does_not_know_and_a_path_the_commit_lacks_both_give_no_content_with(
        &Scratch::new(),
    );
}

#[test]
fn a_commit_git_does_not_know_and_a_path_the_commit_lacks_both_give_no_content_on_an_older_git() {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    a_commit_git_does_not_know_and_a_path_the_commit_lacks_both_give_no_content_with(&scratch);
}

fn a_commit_git_does_not_know_and_a_path_the_commit_lacks_both_give_no_content_with(
    scratch: &Scratch,
) {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    let worktree = open(&repo);
    let unknown = CommitId::new("0123456789abcdef0123456789abcdef01234567");

    assert_eq!(
        worktree.content_at(&head, Path::new("b.txt")).expect("old"),
        None
    );
    assert_eq!(
        worktree
            .content_at(&unknown, Path::new("a.txt"))
            .expect("old"),
        None
    );
    // `resolve` tells the two apart.
    assert_eq!(
        worktree.resolve(head.as_str()).expect("resolve"),
        Some(head.clone())
    );
    assert_eq!(worktree.resolve(unknown.as_str()).expect("resolve"), None);

    // A commit is named by its full id here: any other name is a commit git does not
    // know, even one git would understand.
    assert_eq!(
        worktree
            .content_at(&CommitId::new("HEAD"), Path::new("a.txt"))
            .expect("old"),
        None
    );
    assert_eq!(
        worktree.content_at(&head, Path::new("a.txt")).expect("old"),
        Some(b"a\n".to_vec())
    );
}

/// The lines of the list that are starts of the process that hands out contents.
fn content_processes(list: &Path) -> Vec<String> {
    std::fs::read_to_string(list)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains("cat-file"))
        .map(|line| line.split(' ').next().expect("a process id").to_owned())
        .collect()
}

#[test]
fn many_reads_of_contents_start_one_git_process_which_ends_with_the_worktree() {
    many_reads_of_contents_start_one_git_process_which_ends_with_the_worktree_with(&Scratch::new());
}

#[test]
fn many_reads_of_contents_start_one_git_process_which_ends_with_the_worktree_on_an_older_git() {
    let scratch = Scratch::new();
    an_older_git(&scratch);
    many_reads_of_contents_start_one_git_process_which_ends_with_the_worktree_with(&scratch);
}

fn many_reads_of_contents_start_one_git_process_which_ends_with_the_worktree_with(
    scratch: &Scratch,
) {
    let repo = scratch.join("repo");
    init(&repo);
    for n in 0..20 {
        write(&repo, &format!("file-{n}.txt"), &format!("content {n}\n"));
    }
    let first = CommitId::new(commit_all(&repo, "start"));
    write(&repo, "file-0.txt", "content 0, changed\n");
    let second = CommitId::new(commit_all(&repo, "a change"));

    let (program, list) = git_that_keeps_a_list(scratch);
    let worktree = Worktree::open(&Git::at(program), &repo).expect("open");
    assert_eq!(
        content_processes(&list),
        Vec::<String>::new(),
        "none before a read"
    );
    for commit in [&first, &second] {
        for n in 0..20 {
            let content = worktree
                .content_at(commit, Path::new(&format!("file-{n}.txt")))
                .expect("content");
            let expected = if n == 0 && commit == &second {
                "content 0, changed\n".to_owned()
            } else {
                format!("content {n}\n")
            };
            assert_eq!(content, Some(expected.into_bytes()));
        }
        assert_eq!(
            worktree
                .content_at(commit, Path::new("none.txt"))
                .expect("content"),
            None
        );
    }
    let started = content_processes(&list);
    assert_eq!(started.len(), 1, "one process for 42 reads");
    assert!(is_alive(&started[0]), "it stays for the next read");

    drop(worktree);
    assert!(!is_alive(&started[0]), "it ended, and was waited for");
}

#[test]
fn a_process_for_contents_that_died_is_started_again() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, "a.txt", "a\n");
    let head = CommitId::new(commit_all(&repo, "start"));
    let (program, list) = git_that_keeps_a_list(&scratch);
    let worktree = Worktree::open(&Git::at(program), &repo).expect("open");
    let read = || {
        worktree
            .content_at(&head, Path::new("a.txt"))
            .expect("content")
    };

    assert_eq!(read(), Some(b"a\n".to_vec()));
    let first = content_processes(&list);
    let killed = std::process::Command::new("sh")
        .args(["-c", &format!("kill -9 {}", first[0])])
        .status()
        .expect("start the shell");
    assert!(killed.success());

    assert_eq!(read(), Some(b"a\n".to_vec()));
    assert_eq!(read(), Some(b"a\n".to_vec()));
    let started = content_processes(&list);
    assert_eq!(started.len(), 2, "one more process, for every later read");
    assert_ne!(started[0], started[1]);
}

#[test]
fn several_threads_read_through_one_worktree() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    for n in 0..8 {
        write(
            &repo,
            &format!("file-{n}.txt"),
            &format!("content {n}\n").repeat(500),
        );
    }
    let head = CommitId::new(commit_all(&repo, "start"));
    let worktree = open(&repo);

    std::thread::scope(|threads| {
        for n in 0..8 {
            let (worktree, head) = (&worktree, &head);
            threads.spawn(move || {
                for _ in 0..25 {
                    let path = format!("file-{n}.txt");
                    let expected = format!("content {n}\n").repeat(500).into_bytes();
                    assert_eq!(
                        worktree.content_at(head, Path::new(&path)).expect("old"),
                        Some(expected.clone())
                    );
                    assert_eq!(
                        worktree.content_on_disk(Path::new(&path)).expect("new"),
                        Some(expected)
                    );
                }
            });
        }
    });
}
