//! The text search on real folders: a git repository the test makes, and folders that are
//! none. Searches go through `TextSearch` and `is_ignored`, as the Navigator calls them.

mod support;

use std::path::PathBuf;

use support::{Scratch, write};
use yaar_text_search::{Found, SearchError, TextSearch, is_ignored};

/// The search the app runs: on half the cores.
fn search() -> TextSearch {
    TextSearch::new(TextSearch::half_the_cores())
}

/// A match as the tests state it: the path relative to the folder searched, then line,
/// column and text.
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

/// The paths, relative to `folder`, of the files `word` is found in, each once.
fn files_with(folder: &std::path::Path, word: &str) -> Vec<String> {
    let mut files: Vec<String> = found(folder, &search().all(folder, word).expect("a search"))
        .into_iter()
        .map(|(path, ..)| path)
        .collect();
    files.dedup();
    files
}

/// The repository of TEXT-01: one commit of `a.rs`, `ignored/` in `.gitignore`, then
/// files git has not seen, one of them listed in `.git/info/exclude`.
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
    // The row's sentence needs the file untracked, and ignored by nothing.
    assert!(
        scratch
            .git(&root, &["status", "--porcelain", "--untracked-files=all"])
            .contains("?? fresh.rs"),
        "fresh.rs is untracked"
    );

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

    // The two ways git leaves a file out, and the control half: the tracked name is found
    // by the same search, so the empty lists are not an empty search.
    for name in ["ignored_only_name", "excluded_name"] {
        assert_eq!(
            search().all(&root, name).expect("a search"),
            [],
            "{name} occurs only in a file git leaves out"
        );
    }
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
    // A match long before a NUL that lies beyond the first buffer the search reads: the
    // file is binary all the same, and none of its matches is given.
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
    // A nested repository has a `.git` too, at any depth, and one that is a file, as a
    // linked worktree's is.
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
    // An export of an old side: no `.git`, `.gitignore` files all the same.
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
    // The scratch folder, which holds the folder searched, ignores everything in it: a test
    // may run under any folder, and a worktree is searched on its own.
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
    // The loop ends only when the channel is closed: a search that never closed it would
    // fail this test by the harness's timeout, with no wait of ours.
    let mut list: Vec<Found> = receiver.iter().collect();
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
    // A path outside the folder is not ignored by its rules, though the rules of the
    // repository would leave it out when the folder is a part of it.
    let outside = scratch.path().join("outside.rs");
    assert!(!is_ignored(&root, &outside));
    assert!(!is_ignored(
        &root.join("sub"),
        &root.join("ignored/only_here.txt")
    ));
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
    write(&root, "src/.gitignore", b"gen_*\n");
    let files = [
        "a.rs",
        "x.log",
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
        searched.len() > 2 && searched.len() < files.len(),
        "the files hold both kinds"
    );
}
