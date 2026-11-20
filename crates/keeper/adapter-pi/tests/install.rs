//! What installing pi's Adapter writes into the app's own folder, the one entry it gives
//! every shell, and what it leaves alone: pi's settings, which only the user's yes writes.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_adapter_pi::{
    AGENT_FOLDER, EXTENSION, EXTENSION_FILE, EXTENSION_SOURCE, Pi, REPORT_PROGRAM, REPORT_VARIABLE,
};
use yaar_agent::{Adapter, ShellEntry};

/// A folder of the test's own, removed when the test ends.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // `/tmp` is a link on macOS: the folder is named by its resolved path.
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!(
            "yaar-adapter-pi-install-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("make the scratch folder");
        Scratch { root }
    }

    /// The user's home of this test, made empty.
    fn home(&self) -> PathBuf {
        let home = self.root.join("home");
        std::fs::create_dir_all(&home).expect("make the home");
        home
    }

    /// The Adapter's own folder of the app, as the Keeper would hand it, with a space and
    /// an apostrophe in its name: the path is not made for a shell to read.
    fn folder(&self) -> PathBuf {
        self.root.join("the app's data").join("agents").join("pi")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Every file and folder under `root`, as a path relative to it, with a trailing `/` on a
/// folder.
fn tree_under(root: &Path) -> BTreeSet<String> {
    fn walk(folder: &Path, root: &Path, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(folder).expect("read the folder") {
            let path = entry.expect("an entry").path();
            let relative = path.strip_prefix(root).expect("under the root");
            if path.is_dir() {
                found.insert(format!("{}/", relative.display()));
                walk(&path, root, found);
            } else {
                found.insert(relative.display().to_string());
            }
        }
    }
    let mut found = BTreeSet::new();
    walk(root, root, &mut found);
    found
}

/// Fails, without printing the two texts, unless the file is the extension's source as it is.
fn assert_is_the_source(file: &Path) {
    let written = std::fs::read(file).expect("the extension is there");
    assert!(
        written == EXTENSION_SOURCE.as_bytes(),
        "{} holds {} bytes, not the source's {}",
        file.display(),
        written.len(),
        EXTENSION_SOURCE.len()
    );
}

/// What installing into `folder` gives, for the Adapter that knows `home`.
fn install(home: &Path, folder: &Path, companions: &Path) -> Vec<ShellEntry> {
    Pi::in_home(home)
        .install(folder, companions)
        .expect("the extension is written")
}

#[test]
fn install_writes_the_extension_as_it_is_and_nothing_else_into_a_folder_whose_name_is_not_plain() {
    let scratch = Scratch::new();
    let folder = scratch.folder();

    install(&scratch.home(), &folder, Path::new("/opt/yaar/programs"));

    assert_eq!(
        tree_under(&folder),
        BTreeSet::from([
            format!("{EXTENSION}/"),
            format!("{EXTENSION}/{EXTENSION_FILE}")
        ]),
        "the extension and no temporary file, no settings, no answer"
    );
    assert!(!EXTENSION_SOURCE.is_empty(), "the source is not empty");
    assert_is_the_source(&folder.join(EXTENSION).join(EXTENSION_FILE));
}

#[test]
fn install_gives_one_variable_naming_the_reporting_program_by_its_absolute_path() {
    let scratch = Scratch::new();

    let entries = install(
        &scratch.home(),
        &scratch.folder(),
        Path::new("/opt/yaar/programs"),
    );

    assert_eq!(
        entries,
        [ShellEntry::Variable {
            name: OsString::from(REPORT_VARIABLE),
            value: OsString::from(format!("/opt/yaar/programs/{REPORT_PROGRAM}")),
        }]
    );
}

#[test]
fn a_relative_folder_of_programs_is_made_absolute() {
    let scratch = Scratch::new();

    let entries = install(&scratch.home(), &scratch.folder(), Path::new("programs"));

    let [ShellEntry::Variable { value, .. }] = entries.as_slice() else {
        panic!("one variable, got {entries:?}");
    };
    let expected =
        std::path::absolute(Path::new("programs").join(REPORT_PROGRAM)).expect("the absolute path");
    assert_eq!(Path::new(value), expected);
    assert!(Path::new(value).is_absolute(), "{value:?}");
}

#[test]
fn installing_again_gives_the_same_file_at_the_same_path_and_the_same_entry() {
    let scratch = Scratch::new();
    let folder = scratch.folder();
    let companions = Path::new("/opt/yaar/programs");
    let file = folder.join(EXTENSION).join(EXTENSION_FILE);

    let first = install(&scratch.home(), &folder, companions);
    let first_tree = tree_under(&folder);
    let second = install(&scratch.home(), &folder, companions);

    assert_eq!(first, second);
    assert_eq!(first_tree, tree_under(&folder), "nothing new appears");
    assert_is_the_source(&file);
}

#[test]
fn installing_writes_the_new_text_over_a_file_of_an_earlier_version() {
    let scratch = Scratch::new();
    let folder = scratch.folder();
    let file = folder.join(EXTENSION).join(EXTENSION_FILE);
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("make the folder");
    std::fs::write(&file, "the extension of an earlier version of the app").expect("write");

    install(&scratch.home(), &folder, Path::new("/opt/yaar/programs"));

    assert_is_the_source(&file);
    assert_eq!(
        tree_under(&folder),
        BTreeSet::from([
            format!("{EXTENSION}/"),
            format!("{EXTENSION}/{EXTENSION_FILE}")
        ]),
        "the temporary file is gone"
    );
}

#[test]
fn installing_leaves_the_users_home_as_it_was_with_no_pi_and_with_one() {
    // Without a folder of pi's, and with one that holds a settings file of the user's.
    for settings in [None, Some("{\"theme\": \"dark\"}\n")] {
        let scratch = Scratch::new();
        let home = scratch.home();
        if let Some(text) = settings {
            let agent = home.join(AGENT_FOLDER);
            std::fs::create_dir_all(&agent).expect("make pi's folder");
            std::fs::write(agent.join("settings.json"), text).expect("write pi's settings");
        }
        let before = tree_under(&home);

        // The Adapter knows this home, so that a write into it would be seen here.
        install(&home, &scratch.folder(), Path::new("/opt/yaar/programs"));

        assert_eq!(tree_under(&home), before, "{settings:?}");
        if let Some(text) = settings {
            let kept = std::fs::read_to_string(home.join(AGENT_FOLDER).join("settings.json"))
                .expect("pi's settings");
            assert_eq!(kept, text, "pi's settings are not touched");
        }
    }
}

#[test]
fn a_program_path_that_is_not_text_is_refused_and_nothing_is_written() {
    let scratch = Scratch::new();
    let folder = scratch.folder();
    let companions = PathBuf::from(OsString::from_vec(b"/opt/yaar/\xff\xfe".to_vec()));

    let refused = Pi::in_home(&scratch.home()).install(&folder, &companions);

    assert_eq!(
        refused.expect_err("a path that is no text").kind(),
        ErrorKind::InvalidInput
    );
    assert!(!folder.exists(), "nothing was written");
}
