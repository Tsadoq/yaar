mod support;

use std::time::{Duration, Instant};

use support::{Scratch, write};
use yaar_text_search::TextSearch;

const FOLDERS: usize = 35;
const FILES: usize = 100;
const LINES: usize = 40;

const WORD_LINE: usize = 20;

const RUNS: usize = 7;

fn line(folder: usize, file: usize, line: usize) -> String {
    format!("    let value_{folder}_{file}_{line} = compute({line}, {file}) + offset_{folder};\n")
}

#[test]
fn the_full_list_of_a_repository_of_3500_files_arrives_within_100_ms_once_warm() {
    let scratch = Scratch::new();
    let root = scratch.repository();
    scratch.init(&root);
    let mut matches = 0;
    for folder in 0..FOLDERS {
        for file in 0..FILES {
            let mut text = String::new();
            let seventh = (folder * FILES + file).is_multiple_of(7);
            for number in 1..=LINES {
                if seventh && number == WORD_LINE {
                    text.push_str("    let found = needle_word;\n");
                    matches += 1;
                } else {
                    text.push_str(&line(folder, file, number));
                }
            }
            write(
                &root,
                &format!("dir{folder:02}/file{file:03}.rs"),
                text.as_bytes(),
            );
        }
    }
    scratch.git(&root, &["add", "--all"]);
    scratch.git(&root, &["commit", "--quiet", "-m", "the files"]);
    assert_eq!(matches, 500, "every seventh file of 3,500");

    let search = TextSearch::new(TextSearch::half_the_cores());
    // Warm-up: files read once are in the operating system's cache.
    search.all(&root, "needle_word").expect("the warm-up");

    // Best of several runs: a busy machine can slow any one, and the limit is on
    // what the search takes when the machine lets it.
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        let list = search.all(&root, "needle_word").expect("a search");
        best = best.min(start.elapsed());
        assert_eq!(list.len(), matches, "the list is complete");
        assert!(
            list.windows(2)
                .all(|pair| (&pair[0].path, pair[0].line) < (&pair[1].path, pair[1].line)),
            "the list is sorted"
        );
        assert!(list.iter().all(|found| found.line == WORD_LINE as u32));
    }
    assert!(
        best < Duration::from_millis(100),
        "the best of {RUNS} runs took {best:?} on {} thread(s)",
        search.threads()
    );
}
