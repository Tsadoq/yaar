//! Searches on real folders: a git repository the test makes, and folders that are none.

mod support;

use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use support::{Scratch, write};
use yaar_text_search::{Found, SearchError, TextSearch, is_ignored};

const DEADLINE: Duration = Duration::from_secs(10);

fn search() -> TextSearch {
    TextSearch::new(TextSearch::half_the_cores())
}

fn found(folder: &std::path::Path, list: &[Found]) -> Vec<(String, u32, u32, String)> {
    list.iter()
        .map(|f| {
            let relative = f
                .path
                .strip_prefix(folder)
                .expect("a path inside the folder");
            (
                relative.to_string_lossy().into_owned(),
                f.line,
                f.column,
                f.text.clone(),
            )
        })
        .collect()
}

fn files_with(folder: &std::path::Path, word: &str) -> Vec<String> {
    let mut files: Vec<String> = found(folder, &search().all(folder, word).expect("a search"))
        .into_iter()
        .map(|(path, ..)| path)
        .collect();
    files.dedup();
    files
}

fn repository_of_the_row(scratch: &Scratch) -> PathBuf {
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, "a.rs", b"fn tracked_name() {}\n");
    write(&root, ".gitignore", b"ignored/\n");
    scratch.git(&root, &["add", "a.rs", ".gitignore"]);
    scratch.git(&root, &["commit", "--quiet", "-m", "first"]);
    write(&root, "fresh.rs", b"fn brand_new_name() {}\n");
    write(&root, "ignored/only_here.txt", b"ignored_only_name\n");
    write(&root, "excluded.txt", b"excluded_name\n");
    write(&root, ".git/info/exclude", b"excluded.txt\n");
    root
}

#[test]
fn a_name_defined_in_a_new_untracked_file_is_found_and_one_in_an_ignored_file_is_not() {
    let scratch = Scratch::new();
    let root = repository_of_the_row(&scratch);
    let status = scratch.git(&root, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(status.contains("?? fresh.rs"), "fresh.rs is untracked");
    for left_out in ["ignored/only_here.txt", "excluded.txt"] {
        assert!(
            !status.contains(left_out),
            "git leaves {left_out} out: {status}"
        );
    }

    let new = search().all(&root, "brand_new_name").expect("a search");
    assert_eq!(
        new,
        [Found {
            path: root.join("fresh.rs"),
            line: 1,
            column: 3,
            text: "fn brand_new_name() {}".to_owned(),
        }],
        "the name of the untracked file is found, where it stands"
    );

    for name in ["ignored_only_name", "excluded_name"] {
        assert_eq!(
            search().all(&root, name).expect("a search"),
            [],
            "{name} occurs only in a file git leaves out"
        );
    }
    // Control: the same search finds a tracked name, so the empty lists above are not an empty
    // search.
    let tracked = search().all(&root, "tracked_name").expect("a search");
    assert_eq!(
        found(&root, &tracked),
        [("a.rs".to_owned(), 1, 3, "fn tracked_name() {}".to_owned())],
        "the tracked name is found"
    );
}

#[test]
fn only_whole_words_match() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(
        &root,
        "words.txt",
        b"brand_new_name_more\nxbrand_new_name\nbrand_new_name2\n\
          (brand_new_name)\nbrand_new_name.field\nbrand-new_name brand_new_name\n",
    );
    let list = search().all(&root, "brand_new_name").expect("a search");
    assert_eq!(
        found(&root, &list)
            .into_iter()
            .map(|(_, line, column, _)| (line, column))
            .collect::<Vec<_>>(),
        [(4, 1), (5, 0), (6, 15)],
        "the lines where the name stands whole, and no other"
    );
}

#[test]
fn a_word_of_punctuation_is_looked_for_as_written() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(
        &root,
        "ops.txt",
        b"a.b\naxb\nx+y x+y\nxxy\nf(a)\nfaa\nh[0]\nh0\n$x\nq|r\n",
    );
    for (word, lines) in [
        ("a.b", vec![1]),
        ("x+y", vec![3, 3]),
        ("f(a)", vec![5]),
        ("h[0]", vec![7]),
        ("q|r", vec![10]),
    ] {
        let list = search().all(&root, word).expect("a search");
        assert_eq!(
            list.iter().map(|f| f.line).collect::<Vec<_>>(),
            lines,
            "{word}"
        );
    }
}

#[test]
fn a_match_is_given_with_its_line_its_column_in_bytes_and_the_line_without_its_end() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(
        &root,
        "lines.txt",
        "first\r\n\u{e9}\u{e9} needle one\nneedle two needle\nlast needle".as_bytes(),
    );
    let list = search().all(&root, "needle").expect("a search");
    assert_eq!(
        found(&root, &list),
        [
            (
                "lines.txt".to_owned(),
                2,
                5,
                "\u{e9}\u{e9} needle one".to_owned()
            ),
            ("lines.txt".to_owned(), 3, 0, "needle two needle".to_owned()),
            (
                "lines.txt".to_owned(),
                3,
                11,
                "needle two needle".to_owned()
            ),
            ("lines.txt".to_owned(), 4, 5, "last needle".to_owned()),
        ],
        "columns count bytes (two letters of two bytes before the first), each occurrence is \
         a match, and the last line needs no line end"
    );

    write(&root, "crlf.txt", b"needle\r\nneedle\r\n");
    let list = search().all(&root, "needle").expect("a search");
    assert!(
        list.iter()
            .filter(|f| f.path.ends_with("crlf.txt"))
            .all(|f| f.text == "needle"),
        "a carriage return before the line feed is part of the line end: {list:?}"
    );
}

#[test]
fn a_line_that_is_not_utf_8_is_given_lossily() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, "latin.txt", b"caf\xe9 needle\n");
    let list = search().all(&root, "needle").expect("a search");
    assert_eq!(
        found(&root, &list),
        [(
            "latin.txt".to_owned(),
            1,
            5,
            "caf\u{fffd} needle".to_owned()
        )],
        "the byte that is no UTF-8 is replaced, the column is still the file's"
    );
}

#[test]
fn a_file_with_a_nul_byte_is_not_searched_wherever_the_nul_stands() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, "early.bin", b"needle\n\0binary\n");
    // The NUL lies beyond the first buffer the search reads.
    let mut late = b"needle\n".to_vec();
    late.extend(std::iter::repeat_n(b'x', 1 << 20));
    late.extend(b"\n\0\n");
    write(&root, "late.bin", &late);
    write(&root, "text.txt", b"needle\n");
    assert_eq!(files_with(&root, "needle"), ["text.txt"]);
}

#[test]
fn hidden_files_are_searched_and_git_s_own_folder_never() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, ".hidden/file.txt", b"needle\n");
    write(&root, ".github/workflows/ci.yml", b"needle\n");
    write(&root, ".dotfile", b"needle\n");
    write(&root, ".git/inside.txt", b"needle\n");
    write(&root, "vendor/inner/.git/HEAD", b"needle\n");
    write(&root, "vendor/linked/.git", b"needle\n");
    write(&root, "vendor/inner/code.txt", b"needle\n");
    assert_eq!(
        files_with(&root, "needle"),
        [
            ".dotfile",
            ".github/workflows/ci.yml",
            ".hidden/file.txt",
            "vendor/inner/code.txt",
        ]
    );
}

#[test]
fn the_rules_of_a_gitignore_at_every_level_count_and_a_deeper_one_wins() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, ".gitignore", b"*.log\nbuild/\n");
    write(&root, "src/.gitignore", b"generated.rs\n!keep.log\n");
    write(&root, "top.log", b"needle\n");
    write(&root, "src/keep.log", b"needle\n");
    write(&root, "src/generated.rs", b"needle\n");
    write(&root, "src/real.rs", b"needle\n");
    write(&root, "other/generated.rs", b"needle\n");
    write(&root, "build/out.txt", b"needle\n");
    write(&root, "src/build/out.txt", b"needle\n");
    assert_eq!(
        files_with(&root, "needle"),
        ["other/generated.rs", "src/keep.log", "src/real.rs"]
    );
}

#[test]
fn a_folder_that_is_no_repository_is_searched_by_the_gitignore_files_it_holds() {
    let scratch = Scratch::new();
    let root = scratch.path().join("export");
    write(&root, ".gitignore", b"skipped.txt\n");
    write(&root, "sub/.gitignore", b"*.tmp\n");
    write(&root, "skipped.txt", b"needle\n");
    write(&root, "kept.txt", b"needle\n");
    write(&root, "sub/a.tmp", b"needle\n");
    write(&root, "sub/a.txt", b"needle\n");
    assert_eq!(files_with(&root, "needle"), ["kept.txt", "sub/a.txt"]);
}

#[test]
fn only_gits_rules_count_not_the_ignore_files_of_other_tools() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, ".ignore", b"by_ignore.txt\n");
    write(&root, ".rgignore", b"by_rgignore.txt\n");
    write(&root, "by_ignore.txt", b"needle\n");
    write(&root, "by_rgignore.txt", b"needle\n");
    assert_eq!(
        files_with(&root, "needle"),
        ["by_ignore.txt", "by_rgignore.txt"]
    );
}

#[test]
fn the_rules_of_the_folders_above_the_one_searched_do_not_count() {
    let scratch = Scratch::new();
    // The scratch folder ignores everything in it: a test may run under any folder.
    write(scratch.path(), ".gitignore", b"*\n");
    let root = scratch.path().join("export");
    write(&root, "kept.txt", b"needle\n");
    assert_eq!(files_with(&root, "needle"), ["kept.txt"]);
}

#[test]
fn the_list_is_ordered_by_path_then_line_then_column_on_any_number_of_threads() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    for folder in 0..6 {
        for file in 0..8 {
            write(
                &root,
                &format!("d{folder}/f{file}.txt"),
                b"needle needle\nnothing\nx needle\n",
            );
        }
    }
    let one = TextSearch::new(1).all(&root, "needle").expect("a search");
    assert_eq!(one.len(), 6 * 8 * 3);
    let mut sorted = one.clone();
    sorted.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
    assert_eq!(one, sorted, "ordered by path, then line, then column");
    for threads in [2, 4, TextSearch::half_the_cores()] {
        assert_eq!(
            TextSearch::new(threads)
                .all(&root, "needle")
                .expect("a search"),
            one,
            "{threads} threads give the same list"
        );
    }
}

#[test]
fn the_channel_gives_every_match_and_closes_when_every_file_was_searched() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    for file in 0..40 {
        write(&root, &format!("f{file}.txt"), b"needle\n");
    }
    let receiver = search().search(&root, "needle").expect("a search");
    // Drained until the channel closes: no sender left means every file was searched.
    let deadline = Instant::now() + DEADLINE;
    let mut list: Vec<Found> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(found) => list.push(found),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                panic!(
                    "the channel was not closed within {DEADLINE:?}; {} matches came",
                    list.len()
                )
            }
        }
    }
    list.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(list, search().all(&root, "needle").expect("a search"));
    assert_eq!(list.len(), 40);
}

#[test]
fn a_search_without_a_word_or_without_a_folder_is_refused_before_anything_is_searched() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, "file.txt", b"needle\n");
    for word in ["", "need\nle", "needle\n", "\n"] {
        assert_eq!(
            search().all(&root, word),
            Err(SearchError::NoWord),
            "{word:?}"
        );
        assert!(
            matches!(search().search(&root, word), Err(SearchError::NoWord)),
            "{word:?}"
        );
    }
    let missing = scratch.path().join("missing");
    assert!(matches!(
        search().all(&missing, "needle"),
        Err(SearchError::Folder(_))
    ));
    assert!(
        matches!(
            search().all(&root.join("file.txt"), "needle"),
            Err(SearchError::Folder(_))
        ),
        "a file is no folder"
    );
}

#[test]
fn is_ignored_says_what_the_search_leaves_out() {
    let scratch = Scratch::new();
    let root = repository_of_the_row(&scratch);
    write(&root, "sub/.gitignore", b"*.log\n!keep.log\n");
    write(&root, "sub/a.log", b"x\n");
    write(&root, "sub/keep.log", b"x\n");
    write(&root, "sub/deeper/b.log", b"x\n");

    for ignored in [
        "ignored/only_here.txt",
        "ignored",
        "excluded.txt",
        "sub/a.log",
        "sub/deeper/b.log",
        ".git",
        ".git/config",
    ] {
        assert!(is_ignored(&root, &root.join(ignored)), "{ignored}");
    }
    for kept in [
        "fresh.rs",
        "a.rs",
        ".gitignore",
        "sub/keep.log",
        "sub",
        "sub/new.rs",
    ] {
        assert!(!is_ignored(&root, &root.join(kept)), "{kept}");
    }
    // Outside the folder: not ignored, though the repository's rules would leave it out.
    let outside = scratch.path().join("outside.rs");
    assert!(!is_ignored(&root, &outside));
    assert!(!is_ignored(
        &root.join("sub"),
        &root.join("ignored/only_here.txt")
    ));
}

fn relative_to_the_working_folder(folder: &std::path::Path) -> PathBuf {
    let here = std::env::current_dir()
        .and_then(|here| here.canonicalize())
        .expect("the working folder");
    let mut relative: PathBuf = here.components().skip(1).map(|_| "..").collect();
    relative.extend(folder.components().skip(1));
    relative
}

#[test]
fn is_ignored_takes_a_relative_folder_with_the_absolute_paths_the_search_gives() {
    let scratch = Scratch::new();
    let root = repository_of_the_row(&scratch);
    let folder = relative_to_the_working_folder(&root);
    assert!(folder.is_relative(), "{folder:?} is relative");
    let found = search().all(&folder, "brand_new_name").expect("a search");
    assert_eq!(found.len(), 1, "the control: the search found the new file");
    assert!(found[0].path.is_absolute(), "{:?}", found[0].path);
    let absolute = found[0]
        .path
        .parent()
        .expect("the folder of the file")
        .to_owned();

    assert!(
        is_ignored(&folder, &absolute.join("ignored/only_here.txt")),
        "an ignored file, by its absolute path under a relative folder"
    );
    assert!(is_ignored(&folder, &absolute.join("excluded.txt")));
    assert!(
        !is_ignored(&folder, &found[0].path),
        "the file the search found is not ignored"
    );
    assert!(is_ignored(&folder, &folder.join("ignored/only_here.txt")));
    assert!(!is_ignored(&folder, &folder.join("fresh.rs")));
}

#[test]
fn is_ignored_agrees_with_what_the_search_finds_in_every_file() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(
        &root,
        ".gitignore",
        b"*.log\n/build\ndocs/*.md\n!docs/keep.md\n",
    );
    write(&root, ".git/info/exclude", b"private/\n");
    write(&root, "src/.gitignore", b"gen_*\n!keep.log\n");
    let files = [
        "a.rs",
        "x.log",
        "src/keep.log",
        "build/o.txt",
        "src/build/o.txt",
        "docs/a.md",
        "docs/keep.md",
        "docs/sub/c.md",
        "private/p.txt",
        "src/private/p.txt",
        "src/gen_one.rs",
        "src/real.rs",
        "src/deep/gen_two.rs",
        ".hidden/h.txt",
    ];
    for file in files {
        write(&root, file, b"needle\n");
    }
    let searched = files_with(&root, "needle");
    for file in files {
        assert_eq!(
            is_ignored(&root, &root.join(file)),
            !searched.iter().any(|s| s == file),
            "{file}: ignored exactly when the search does not find it ({searched:?})"
        );
    }
    assert!(
        searched.iter().any(|s| s == "src/keep.log"),
        "the deeper .gitignore's re-inclusion is searched ({searched:?})"
    );
    assert!(!is_ignored(&root, &root.join("src/keep.log")));
    assert!(
        searched.len() > 2 && searched.len() < files.len(),
        "the files hold both kinds"
    );
}

#[test]
fn is_ignored_agrees_with_the_search_in_a_linked_worktree_and_in_a_nested_repository() {
    let scratch = Scratch::new();
    let main = scratch.repository();
    scratch.init(&main);
    write(&main, "a.rs", b"fn tracked_name() {}\n");
    scratch.git(&main, &["add", "a.rs"]);
    scratch.git(&main, &["commit", "--quiet", "-m", "first"]);
    // A linked worktree shares the main repository's exclude file.
    write(&main, ".git/info/exclude", b"secret.txt\n");
    let linked = scratch.path().join("linked");
    scratch.git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "other",
            linked.to_str().expect("a path"),
        ],
    );
    assert!(
        linked.join(".git").is_file(),
        "a linked worktree's .git is a file"
    );
    let inner = linked.join("vendor/inner");
    write(&linked, "vendor/inner/placeholder", b"");
    scratch.init(&inner);
    write(&inner, ".git/info/exclude", b"hid.txt\n!secret.txt\n");
    let files = [
        "a.rs",
        "secret.txt",
        "src/secret.txt",
        "hid.txt",
        "vendor/inner/hid.txt",
        "vendor/inner/deep/hid.txt",
        "vendor/inner/secret.txt",
        "vendor/inner/code.txt",
    ];
    for file in files {
        write(&linked, file, b"needle\n");
    }

    let untracked = scratch.git(&linked, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(
        !untracked.contains("secret.txt") && untracked.contains("?? hid.txt"),
        "git leaves the shared exclude's name out of the worktree: {untracked}"
    );

    let searched = files_with(&linked, "needle");
    for file in files {
        assert_eq!(
            is_ignored(&linked, &linked.join(file)),
            !searched.iter().any(|s| s == file),
            "{file}: ignored exactly when the search does not find it ({searched:?})"
        );
    }
    for left_out in [
        "secret.txt",
        "src/secret.txt",
        "vendor/inner/hid.txt",
        "vendor/inner/deep/hid.txt",
    ] {
        assert!(is_ignored(&linked, &linked.join(left_out)), "{left_out}");
    }
    for kept in [
        "a.rs",
        "hid.txt",
        "vendor/inner/code.txt",
        "vendor/inner/secret.txt",
    ] {
        assert!(!is_ignored(&linked, &linked.join(kept)), "{kept}");
    }
}

#[test]
fn a_relative_folder_gives_matches_by_absolute_paths() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, "file.txt", b"needle\n");
    let here = std::env::current_dir().expect("the working folder");
    let up = here.components().count() - 1;
    let relative: PathBuf = std::iter::repeat_n("..", up)
        .map(PathBuf::from)
        .chain([root
            .strip_prefix("/")
            .expect("an absolute folder")
            .to_path_buf()])
        .collect();
    assert!(relative.is_relative());
    let list = search().all(&relative, "needle").expect("a search");
    assert_eq!(list.len(), 1);
    assert!(list[0].path.is_absolute(), "{:?}", list[0].path);
    assert_eq!(
        list[0].path.canonicalize().expect("the file"),
        root.join("file.txt")
    );
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_is_no_folder_to_a_rule_that_names_folders() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    write(&root, ".gitignore", b"linked/\n");
    write(&root, "real/file.txt", b"needle\n");
    std::os::unix::fs::symlink("real", root.join("linked")).expect("make the link");
    // A folder pattern leaves out no link, which is no folder.
    let status = scratch.git(&root, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(status.contains("?? linked"), "git keeps the link: {status}");
    assert!(!is_ignored(&root, &root.join("linked")));
    assert_eq!(files_with(&root, "needle"), ["real/file.txt"]);
}
