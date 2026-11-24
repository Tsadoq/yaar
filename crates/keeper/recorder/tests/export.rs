mod support;

use std::collections::BTreeMap;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use support::{Scratch, commit_all, git, init, write};
use yaar_recorder::{Git, RecordError, StateStore, Worktree};

fn found() -> Git {
    Git::find().expect("git on the search path")
}

fn open(folder: &Path) -> Worktree {
    Worktree::open(&found(), folder).expect("open")
}

fn store(scratch: &Scratch) -> StateStore {
    StateStore::open(&found(), &scratch.join("app/states/one.git")).expect("open the store")
}

#[derive(Debug, PartialEq, Eq)]
enum Entry {
    File { bytes: Vec<u8>, executable: bool },
    Link(PathBuf),
}

fn file(text: &str) -> Entry {
    Entry::File {
        bytes: text.as_bytes().to_vec(),
        executable: false,
    }
}

fn program(text: &str) -> Entry {
    Entry::File {
        bytes: text.as_bytes().to_vec(),
        executable: true,
    }
}

fn link(target: &str) -> Entry {
    Entry::Link(PathBuf::from(target))
}

fn entries(folder: &Path) -> BTreeMap<String, Entry> {
    fn walk(root: &Path, folder: &Path, into: &mut BTreeMap<String, Entry>) {
        for child in std::fs::read_dir(folder).expect("read the folder") {
            let child = child.expect("an entry").path();
            let kind = std::fs::symlink_metadata(&child).expect("the entry's kind");
            let name = child
                .strip_prefix(root)
                .expect("below the root")
                .to_str()
                .expect("a path of text")
                .to_owned();
            if kind.is_dir() {
                walk(root, &child, into);
            } else if kind.is_symlink() {
                into.insert(
                    name,
                    Entry::Link(std::fs::read_link(&child).expect("a link")),
                );
            } else {
                into.insert(
                    name,
                    Entry::File {
                        bytes: std::fs::read(&child).expect("read the file"),
                        executable: kind.permissions().mode() & 0o111 != 0,
                    },
                );
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

fn expected<const N: usize>(wanted: [(&str, Entry); N]) -> BTreeMap<String, Entry> {
    wanted
        .into_iter()
        .map(|(path, entry)| (path.to_owned(), entry))
        .collect()
}

fn repository(scratch: &Scratch) -> (PathBuf, String) {
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitignore", "ignored/\n");
    write(&repo, "run.sh", "#!/bin/sh\necho one\n");
    std::fs::set_permissions(repo.join("run.sh"), std::fs::Permissions::from_mode(0o755))
        .expect("make it run");
    write(&repo, "src/deep/lib.rs", "fn one() {}\n");
    symlink("src/deep/lib.rs", repo.join("to-lib")).expect("make a link");
    let commit = commit_all(&repo, "one");
    (repo, commit)
}

fn files_of_the_commit() -> BTreeMap<String, Entry> {
    expected([
        (".gitignore", file("ignored/\n")),
        ("run.sh", program("#!/bin/sh\necho one\n")),
        ("src/deep/lib.rs", file("fn one() {}\n")),
        ("to-lib", link("src/deep/lib.rs")),
    ])
}

#[test]
fn the_export_of_a_commit_writes_exactly_its_files_and_no_git_folder() {
    let scratch = Scratch::new();
    let (repo, commit) = repository(&scratch);
    write(&repo, "src/deep/lib.rs", "fn two() {}\n");
    write(&repo, "new.txt", "new\n");
    let worktree = open(&repo);
    let store = store(&scratch);

    let into = scratch.join("old");
    store
        .export(Some(&worktree), &commit, &into)
        .expect("export");
    assert_eq!(entries(&into), files_of_the_commit());
    assert!(!into.join(".git").exists());
}

#[test]
fn the_export_of_a_recorded_state_writes_the_files_as_they_were_then() {
    let scratch = Scratch::new();
    let (repo, _) = repository(&scratch);
    write(&repo, "src/deep/lib.rs", "fn two() {}\n");
    write(&repo, "untracked.txt", "not added\n");
    write(&repo, "ignored/cache.bin", "never in a state\n");
    let worktree = open(&repo);
    let store = store(&scratch);
    let state = store.record(&worktree).expect("record");
    write(&repo, "src/deep/lib.rs", "fn three() {}\n");
    std::fs::remove_dir_all(&repo).expect("remove the repository");

    let into = scratch.join("old");
    store
        .export(None, state.id.as_str(), &into)
        .expect("export");
    assert_eq!(
        entries(&into),
        expected([
            (".gitignore", file("ignored/\n")),
            ("run.sh", program("#!/bin/sh\necho one\n")),
            ("src/deep/lib.rs", file("fn two() {}\n")),
            ("to-lib", link("src/deep/lib.rs")),
            ("untracked.txt", file("not added\n")),
        ])
    );
}

#[test]
fn a_version_the_store_does_not_hold_with_no_worktree_is_an_error_and_into_is_not_there() {
    let scratch = Scratch::new();
    let (_, commit) = repository(&scratch);
    let store = store(&scratch);

    let into = scratch.join("old");
    let error = store.export(None, &commit, &into).expect_err("not held");
    assert!(matches!(error, RecordError::Store { .. }), "{error}");
    assert!(!into.exists());
}

#[test]
fn a_version_that_is_not_a_commit_of_the_store_or_the_repository_is_an_error_and_into_is_not_there()
{
    let scratch = Scratch::new();
    let (repo, _) = repository(&scratch);
    let tree = git(&repo, &["rev-parse", "HEAD^{tree}"]);
    let worktree = open(&repo);
    let store = store(&scratch);

    for version in [
        tree.as_str(),
        "1234567890abcdef1234567890abcdef12345678",
        "main",
        "HEAD",
        "1234567",
        "",
    ] {
        let into = scratch.join("old");
        assert!(
            store.export(Some(&worktree), version, &into).is_err(),
            "{version:?}"
        );
        assert!(!into.exists(), "{version:?}");
    }
}

#[test]
fn an_into_that_exists_is_an_error_and_is_left_as_it_was() {
    let scratch = Scratch::new();
    let (repo, commit) = repository(&scratch);
    let worktree = open(&repo);
    let store = store(&scratch);

    let into = scratch.join("old");
    write(&into, "mine.txt", "not the store's\n");
    let error = store
        .export(Some(&worktree), &commit, &into)
        .expect_err("taken");
    assert!(matches!(error, RecordError::Store { .. }), "{error}");
    assert_eq!(
        entries(&into),
        expected([("mine.txt", file("not the store's\n"))])
    );

    let file_there = scratch.join("a-file");
    std::fs::write(&file_there, "a file\n").expect("write");
    assert!(store.export(Some(&worktree), &commit, &file_there).is_err());
    assert_eq!(std::fs::read(&file_there).expect("read"), b"a file\n");
}

#[test]
fn an_into_whose_parent_is_not_there_is_an_error_and_makes_no_folder() {
    let scratch = Scratch::new();
    let (repo, commit) = repository(&scratch);
    let worktree = open(&repo);
    let store = store(&scratch);

    let into = scratch.join("above/old");
    let error = store
        .export(Some(&worktree), &commit, &into)
        .expect_err("no parent");
    assert!(matches!(error, RecordError::Store { .. }), "{error}");
    assert!(!scratch.join("above").exists());
}

#[test]
fn the_repository_and_the_temporary_folder_are_as_they_were_after_an_export() {
    let scratch = Scratch::new();
    let (repo, commit) = repository(&scratch);
    write(&repo, "src/deep/lib.rs", "fn two() {}\n");
    let second = commit_all(&repo, "two");
    write(&repo, "src/deep/lib.rs", "fn stashed() {}\n");
    git(&repo, &["stash", "push", "--quiet"]);
    write(&repo, "run.sh", "#!/bin/sh\necho changed\n");
    write(&repo, "untracked.txt", "not added\n");
    write(&repo, "ignored/cache.bin", "left out\n");
    git(
        &repo,
        &["worktree", "add", "--quiet", "-b", "other", "../second"],
    );
    let temp = scratch.join("tmp");
    std::fs::create_dir_all(&temp).expect("make the folder");
    // SAFETY: as in `Scratch::new`: no other thread exists yet in this test's process.
    unsafe { std::env::set_var("TMPDIR", &temp) };

    let read = |repo: &Path| {
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .env("GIT_OPTIONAL_LOCKS", "0")
                .args(args)
                .output()
                .expect("start git")
                .stdout
        };
        (
            git(&[
                "status",
                "--porcelain=v2",
                "--ignored",
                "--untracked-files=all",
            ]),
            git(&["worktree", "list", "--porcelain"]),
            git(&["for-each-ref"]),
            git(&["stash", "list"]),
            git(&["count-objects", "-v"]),
            std::fs::read(repo.join(".git/index")).expect("the list of files"),
        )
    };
    let before = read(&repo);
    assert!(!before.2.is_empty() && !before.3.is_empty() && !before.0.is_empty());

    let worktree = open(&repo);
    let store = store(&scratch);
    store
        .export(Some(&worktree), &commit, &scratch.join("old-1"))
        .expect("export a commit the store copies");
    store
        .export(Some(&worktree), &second, &scratch.join("old-2"))
        .expect("export another");
    let state = store.record(&worktree).expect("record");
    store
        .export(Some(&worktree), state.id.as_str(), &scratch.join("old-3"))
        .expect("export a state");

    assert_eq!(read(&repo), before);
    assert_eq!(std::fs::read_dir(&temp).expect("read").count(), 0);
}

#[test]
fn drivers_that_would_fail_run_no_program_and_the_files_are_written_as_git_stores_them() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    let mark = scratch.join("a-driver-ran");
    let global = scratch.join("gitconfig");
    // A long-running `process` driver takes part in staging too, so it is added after the commit.
    let config = |process: bool| {
        let process = |program: &str| {
            if process {
                format!("\tprocess = {program}\n")
            } else {
                String::new()
            }
        };
        format!(
            "[filter \"big\"]\n\tclean = cat\n\tsmudge = false\n{}\trequired = true\n\
             [filter \"odd.name=x\"]\n\tclean = cat\n\tsmudge = touch {}\n{}\trequired = true\n",
            process("false"),
            mark.display(),
            process(&format!("touch {}", mark.display())),
        )
    };
    std::fs::write(&global, config(false)).expect("write the configuration");
    // SAFETY: as in `Scratch::new`: no other thread exists yet in this test's process.
    unsafe { std::env::set_var("GIT_CONFIG_GLOBAL", &global) };
    git(&repo, &["config", "filter.local.clean", "cat"]);
    git(&repo, &["config", "filter.local.smudge", "false"]);
    git(&repo, &["config", "filter.local.required", "true"]);
    write(
        &repo,
        ".gitattributes",
        "*.bin filter=big\n*.odd filter=odd.name=x\n*.loc filter=local\n",
    );
    write(&repo, "a.bin", "version https://example.invalid/spec/v1\n");
    write(&repo, "b.odd", "an odd one\n");
    write(&repo, "c.loc", "a local one\n");
    write(&repo, "plain.txt", "plain\n");
    let commit = commit_all(&repo, "one");
    std::fs::write(&global, config(true)).expect("write the configuration");
    let worktree = open(&repo);
    let store = store(&scratch);

    let into = scratch.join("old");
    store
        .export(Some(&worktree), &commit, &into)
        .expect("export");
    assert_eq!(
        entries(&into),
        expected([
            (
                ".gitattributes",
                file("*.bin filter=big\n*.odd filter=odd.name=x\n*.loc filter=local\n")
            ),
            ("a.bin", file("version https://example.invalid/spec/v1\n")),
            ("b.odd", file("an odd one\n")),
            ("c.loc", file("a local one\n")),
            ("plain.txt", file("plain\n")),
        ])
    );
    assert!(!mark.exists(), "a driver ran its program");
}

#[test]
fn line_end_conversions_of_the_attributes_apply_as_in_a_checkout() {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    write(&repo, ".gitattributes", "*.txt text eol=crlf\n");
    write(&repo, "a.txt", "one\ntwo\n");
    let commit = commit_all(&repo, "one");
    let worktree = open(&repo);
    let store = store(&scratch);

    let into = scratch.join("old");
    store
        .export(Some(&worktree), &commit, &into)
        .expect("export");
    assert_eq!(
        entries(&into)["a.txt"],
        Entry::File {
            bytes: b"one\r\ntwo\r\n".to_vec(),
            executable: false
        }
    );
}

#[test]
fn settings_the_environment_hands_git_still_apply_when_drivers_are_turned_off() {
    settings_of_the_environment_apply_with_drivers_off(false);
}

#[test]
fn settings_the_environment_hands_git_still_apply_when_hooks_and_drivers_are_turned_off() {
    settings_of_the_environment_apply_with_drivers_off(true);
}

fn settings_of_the_environment_apply_with_drivers_off(hook: bool) {
    let scratch = Scratch::new();
    let repo = scratch.join("repo");
    init(&repo);
    let mark = scratch.join("a-driver-ran");
    let global = scratch.join("gitconfig");
    // SAFETY: as in `Scratch::new`: no other thread exists yet in this test's process.
    unsafe { std::env::set_var("GIT_CONFIG_GLOBAL", &global) };
    write(&repo, ".gitattributes", "*.bin filter=big\n");
    write(&repo, "a.bin", "stored\n");
    write(&repo, "a.txt", "one\ntwo\n");
    let commit = commit_all(&repo, "one");
    // A driver that must run would fail the commit, so it is added after. A hook goes in before
    // the store is opened, which reads once the hooks it turns off.
    let hook_section = if hook {
        format!(
            "[hook \"x\"]\n\tcommand = touch {}\n\tevent = pre-commit\n",
            scratch.join("a-hook-ran").display()
        )
    } else {
        String::new()
    };
    std::fs::write(
        &global,
        format!(
            "{hook_section}[filter \"big\"]\n\tsmudge = touch {}\n\trequired = true\n",
            mark.display()
        ),
    )
    .expect("write the configuration");
    let worktree = open(&repo);
    let store = store(&scratch);

    // SAFETY: as above.
    unsafe {
        std::env::set_var("GIT_CONFIG_COUNT", "1");
        std::env::set_var("GIT_CONFIG_KEY_0", "core.autocrlf");
        std::env::set_var("GIT_CONFIG_VALUE_0", "true");
    }

    let into = scratch.join("old");
    store
        .export(Some(&worktree), &commit, &into)
        .expect("export");
    let found = entries(&into);
    assert_eq!(found["a.txt"], file("one\r\ntwo\r\n"));
    assert_eq!(found["a.bin"], file("stored\r\n"));
    assert!(!mark.exists(), "a driver ran its program");
}
