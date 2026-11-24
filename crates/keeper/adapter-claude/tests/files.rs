mod support;

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use yaar_adapter_claude::{ClaudeCode, REVIEWS};
use yaar_agent::Adapter;

use support::Scratch;

const THIRTY_DAYS: Duration = Duration::from_secs(30 * 24 * 60 * 60);

struct Place {
    scratch: Scratch,
    folder: PathBuf,
}

impl Place {
    fn new() -> Place {
        let scratch = Scratch::new();
        let folder = scratch.path().join("agents").join("claude");
        fs::create_dir_all(folder.join(REVIEWS)).expect("make the folder of reviews");
        Place { scratch, folder }
    }

    fn session(&self, id: &str) -> PathBuf {
        self.folder.join(REVIEWS).join(id)
    }

    fn write(&self, id: &str, name: &str, bytes: &[u8]) -> PathBuf {
        let folder = self.session(id);
        fs::create_dir_all(&folder).expect("make the session's folder");
        let file = folder.join(name);
        fs::create_dir_all(file.parent().expect("a parent")).expect("make the file's folder");
        fs::write(&file, bytes).expect("write a review");
        file
    }

    fn outside(&self) -> (PathBuf, PathBuf) {
        let file = self.scratch.path().join("outside.txt");
        fs::write(&file, noise(20_000, 1)).expect("write the outside file");
        let folder = self.scratch.path().join("outside-folder");
        fs::create_dir_all(&folder).expect("make the outside folder");
        fs::write(folder.join("inside.txt"), noise(20_000, 2))
            .expect("write the file of the outside folder");
        (file, folder)
    }

    fn remove(&self, running: &[&str], written_before: Option<SystemTime>) -> io::Result<u64> {
        let running: BTreeSet<&str> = running.iter().copied().collect();
        ClaudeCode::new().remove_files_of_ended(
            &self.folder,
            &|id| running.contains(id),
            written_before,
        )
    }
}

/// Incompressible bytes, so that a file uses more than no blocks; `seed` makes two differ.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

fn blocks_of(path: &Path) -> u64 {
    fs::symlink_metadata(path)
        .expect("a file to count")
        .blocks()
        * 512
}

fn set_modified(path: &Path, time: SystemTime) {
    File::options()
        .write(true)
        .open(path)
        .expect("open the file")
        .set_modified(time)
        .expect("set the modification time");
}

#[test]
fn the_files_of_a_session_that_ended_go_and_those_of_one_that_runs_stay() {
    let place = Place::new();
    let ended = place.write("s1", "r-1.md", &noise(20_000, 3));
    let running_text = noise(30_000, 4);
    let running = place.write("s2", "r-1.md", &running_text);
    let expected = blocks_of(&ended);
    assert!(expected > 0, "a file of 20 000 bytes uses blocks");

    let removed = place.remove(&["s2"], None).expect("the clean-up runs");

    assert!(!place.session("s1").exists(), "the folder of s1 is gone");
    assert_eq!(fs::read(&running).expect("s2's file"), running_text);
    assert_eq!(removed, expected, "the bytes it removed, as blocks");
}

#[test]
fn every_file_of_an_ended_session_is_counted_the_hidden_temporary_ones_too() {
    let place = Place::new();
    let first = place.write("s1", "R-1.md", &[b'x'; 9_000]);
    let second = place.write("s1", "R-2.md", &[b'y'; 100]);
    let temporary = place.write("s1", ".R-3.md.4242.tmp", &[b'z'; 5_000]);
    let expected = blocks_of(&first) + blocks_of(&second) + blocks_of(&temporary);

    let removed = place.remove(&[], None).expect("the clean-up runs");

    assert!(!place.session("s1").exists());
    assert_eq!(removed, expected);
}

#[test]
fn a_link_inside_a_session_is_removed_as_a_link_and_never_followed() {
    let place = Place::new();
    let (file, folder) = place.outside();
    let review = place.write("s1", "r-1.md", &noise(9_000, 5));
    let to_file = place.session("s1").join("to-file");
    let to_folder = place.session("s1").join("to-folder");
    symlink(&file, &to_file).expect("link to a file");
    symlink(&folder, &to_folder).expect("link to a folder");
    let expected = blocks_of(&review) + blocks_of(&to_file) + blocks_of(&to_folder);
    assert!(expected > 0, "the review uses blocks");
    let outside = blocks_of(&file) + blocks_of(&folder.join("inside.txt"));
    assert!(outside > 0, "the files outside use blocks");

    let removed = place.remove(&[], None).expect("the clean-up runs");

    assert!(!place.session("s1").exists());
    assert_eq!(fs::read(&file).expect("the file"), noise(20_000, 1));
    assert_eq!(
        fs::read(folder.join("inside.txt")).expect("the folder's file"),
        noise(20_000, 2)
    );
    assert_eq!(removed, expected, "what the links name is not counted");
}

#[test]
fn a_session_that_is_itself_a_link_goes_as_a_link_and_its_target_stays() {
    let place = Place::new();
    let (_, folder) = place.outside();
    symlink(&folder, place.session("s1")).expect("link a session to a folder");
    let expected = blocks_of(&place.session("s1"));
    assert!(
        blocks_of(&folder.join("inside.txt")) > 0,
        "the folder's file uses blocks"
    );

    let removed = place.remove(&[], None).expect("the clean-up runs");

    assert!(fs::symlink_metadata(place.session("s1")).is_err());
    assert_eq!(
        fs::read(folder.join("inside.txt")).expect("the folder's file"),
        noise(20_000, 2)
    );
    assert_eq!(removed, expected, "what the link names is not counted");
}

#[test]
fn a_link_in_the_place_of_a_session_made_within_the_set_time_is_kept() {
    let place = Place::new();
    let (_, folder) = place.outside();
    symlink(&folder, place.session("s1")).expect("link a session to a folder");

    let removed = place
        .remove(&[], Some(SystemTime::now() - THIRTY_DAYS))
        .expect("the clean-up runs");

    // A link's own time cannot be set back with std, so only the kept half shows here.
    assert_eq!(removed, 0, "a link made now is within the time");
    assert!(fs::symlink_metadata(place.session("s1")).is_ok());
    assert_eq!(
        fs::read(folder.join("inside.txt")).expect("the folder's file"),
        noise(20_000, 2)
    );
}

#[test]
fn a_folder_of_reviews_that_is_a_link_is_not_followed() {
    let place = Place::new();
    let (_, folder) = place.outside();
    fs::remove_dir_all(place.folder.join(REVIEWS)).expect("remove the folder of reviews");
    fs::create_dir_all(folder.join("s1")).expect("a folder named like a session");
    fs::write(
        folder.join("s1").join("r-1.md"),
        b"not a review of the Adapter",
    )
    .expect("write the file");
    symlink(&folder, place.folder.join(REVIEWS)).expect("link the folder of reviews");

    let removed = place
        .remove(&[], None)
        .expect("nothing to do is no failure");

    assert_eq!(removed, 0);
    assert!(folder.join("s1").join("r-1.md").exists());
}

#[test]
fn a_session_written_within_the_set_time_is_kept_and_one_written_before_it_goes() {
    let place = Place::new();
    let file = place.write("s3", "r-1.md", &[b'a'; 4_000]);
    let now = SystemTime::now();
    let expected = blocks_of(&file);

    let removed = place
        .remove(&[], Some(now - THIRTY_DAYS))
        .expect("the clean-up runs");
    assert_eq!(removed, 0, "a file written now is within the time");
    assert_eq!(fs::read(&file).expect("kept"), [b'a'; 4_000]);

    set_modified(&file, now - THIRTY_DAYS - Duration::from_secs(24 * 60 * 60));
    let removed = place
        .remove(&[], Some(now - THIRTY_DAYS))
        .expect("the clean-up runs");
    assert!(!place.session("s3").exists(), "31 days old is older");
    assert_eq!(removed, expected);
}

#[test]
fn a_session_with_one_file_still_new_keeps_all_of_them() {
    let place = Place::new();
    let now = SystemTime::now();
    let old = place.write("s1", "r-1.md", b"old");
    let new = place.write("s1", "r-2.md", b"new");
    set_modified(&old, now - THIRTY_DAYS - Duration::from_secs(3600));

    let removed = place
        .remove(&[], Some(now - THIRTY_DAYS))
        .expect("the clean-up runs");

    assert_eq!(removed, 0);
    assert_eq!(fs::read(&old).expect("kept"), b"old");
    assert_eq!(fs::read(&new).expect("kept"), b"new");
}

// `read_dir` order depends on the file system: the two sessions hold the old and the new file
// swapped, so in any order one meets its old file first and a running count would show.
#[test]
fn a_session_kept_for_a_new_file_adds_nothing_to_the_bytes_removed_in_whatever_order_its_files_come()
 {
    let place = Place::new();
    let now = SystemTime::now();
    let old = now - THIRTY_DAYS - Duration::from_secs(3600);
    let first_old = place.write("s1", "a", &noise(9_000, 10));
    let first_new = place.write("s1", "b", &noise(9_000, 11));
    let second_new = place.write("s2", "a", &noise(9_000, 12));
    let second_old = place.write("s2", "b", &noise(9_000, 13));
    set_modified(&first_old, old);
    set_modified(&second_old, old);
    assert!(
        blocks_of(&first_old) > 0,
        "a file of 9 000 bytes uses blocks"
    );

    let removed = place
        .remove(&[], Some(now - THIRTY_DAYS))
        .expect("the clean-up runs");

    assert_eq!(
        removed, 0,
        "a kept session adds nothing to what was removed"
    );
    for (file, seed) in [
        (&first_old, 10),
        (&first_new, 11),
        (&second_new, 12),
        (&second_old, 13),
    ] {
        assert_eq!(fs::read(file).expect("kept"), noise(9_000, seed));
    }
}

#[test]
fn a_file_in_a_folder_of_a_session_counts_for_all_it_says() {
    let place = Place::new();
    let now = SystemTime::now();
    let old = place.write("s1", "r-1.md", &noise(5_000, 6));
    let nested = place.write("s1", "sub/r-2.md", &noise(7_000, 7));
    set_modified(&old, now - THIRTY_DAYS - Duration::from_secs(3600));

    let removed = place
        .remove(&[], Some(now - THIRTY_DAYS))
        .expect("the clean-up runs");
    assert_eq!(removed, 0, "the file in the folder is within the time");
    assert!(old.exists());
    assert!(nested.exists());

    set_modified(&nested, now - THIRTY_DAYS - Duration::from_secs(3600));
    let expected = blocks_of(&old) + blocks_of(&nested);
    let removed = place
        .remove(&[], Some(now - THIRTY_DAYS))
        .expect("the clean-up runs");
    assert!(!place.session("s1").exists());
    assert_eq!(
        removed, expected,
        "the blocks of the file in the folder too"
    );
}

#[test]
fn a_file_written_at_the_very_time_is_not_before_it() {
    let place = Place::new();
    let cut = SystemTime::now() - THIRTY_DAYS;
    let file = place.write("s1", "r-1.md", &noise(20_000, 8));
    set_modified(&file, cut);
    let expected = blocks_of(&file);

    assert_eq!(place.remove(&[], Some(cut)).expect("runs"), 0);
    assert!(file.exists());

    let later = cut + Duration::from_secs(1);
    assert_eq!(place.remove(&[], Some(later)).expect("runs"), expected);
    assert!(!place.session("s1").exists());
}

#[test]
fn a_session_that_runs_keeps_its_files_however_old() {
    let place = Place::new();
    let file = place.write("s1", "r-1.md", b"read still");
    set_modified(&file, SystemTime::now() - THIRTY_DAYS * 3);

    let removed = place
        .remove(&["s1"], Some(SystemTime::now()))
        .expect("the clean-up runs");

    assert_eq!(removed, 0);
    assert_eq!(fs::read(&file).expect("kept"), b"read still");
}

#[test]
fn what_is_no_session_folder_is_left_where_it_lies() {
    let place = Place::new();
    let stray = place.folder.join(REVIEWS).join("notes.txt");
    fs::write(&stray, b"not a session").expect("write a stray file");
    let named_like_a_session = place.folder.join(REVIEWS).join("s9");
    fs::write(&named_like_a_session, noise(9_000, 14)).expect("write a file named like a session");
    assert!(blocks_of(&named_like_a_session) > 0, "the file uses blocks");
    let odd = place.write("not a session id", "r-1.md", b"odd");
    let beside = place.folder.join("plugin").join("plugin.json");
    fs::create_dir_all(beside.parent().expect("a parent")).expect("make the plugin folder");
    fs::write(&beside, b"{}").expect("write the plugin's file");

    let removed = place.remove(&[], None).expect("the clean-up runs");

    assert_eq!(removed, 0);
    assert!(stray.exists());
    assert_eq!(
        fs::read(&named_like_a_session).expect("kept"),
        noise(9_000, 14),
        "a plain file named like a session is kept byte for byte"
    );
    assert!(odd.exists());
    assert!(beside.exists());
}

#[test]
fn a_folder_with_no_reviews_in_it_is_nothing_to_clean() {
    let place = Place::new();
    fs::remove_dir_all(place.folder.join(REVIEWS)).expect("remove the folder of reviews");
    assert_eq!(place.remove(&[], None).expect("no reviews"), 0);

    fs::remove_dir_all(&place.folder).expect("remove the Adapter's folder");
    assert_eq!(place.remove(&[], None).expect("no folder"), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn a_session_folder_whose_name_is_not_text_is_passed_over() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let place = Place::new();
    let odd = place
        .folder
        .join(REVIEWS)
        .join(OsStr::from_bytes(b"s\xff1"));
    fs::create_dir_all(&odd).expect("make a folder whose name is not text");
    fs::write(odd.join("r-1.md"), b"kept").expect("write the file");
    place.write("s2", "r-1.md", b"gone");

    place.remove(&[], None).expect("the clean-up runs");

    assert_eq!(fs::read(odd.join("r-1.md")).expect("kept"), b"kept");
    assert!(!place.session("s2").exists());
}

#[cfg(target_os = "macos")]
struct Unlocked(Vec<PathBuf>);

#[cfg(target_os = "macos")]
impl Drop for Unlocked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        for folder in &self.0 {
            let _ = fs::set_permissions(folder, fs::Permissions::from_mode(0o755));
        }
    }
}

// Needs a folder that refuses its owner: the Linux tests run as root, who is refused nothing.
#[cfg(target_os = "macos")]
#[test]
fn a_session_that_cannot_be_removed_stays_the_others_go_on_and_the_call_gives_the_failure() {
    use std::os::unix::fs::PermissionsExt;

    let place = Place::new();
    // The session `read_dir` names first cannot be removed: a call that gave up at the first
    // failure would leave the others behind.
    let ids = ["s1", "s2", "s3", "s4"];
    for id in ids {
        place.write(id, "r-1.md", &noise(9_000, 20));
    }
    let listed: Vec<String> = fs::read_dir(place.folder.join(REVIEWS))
        .expect("list the sessions")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("text")
        })
        .collect();
    let stuck = listed.first().expect("a session").clone();
    let stuck_file = place.write(&stuck, "sub/r-2.md", &noise(9_000, 21));
    let sub = stuck_file.parent().expect("a folder").to_owned();
    let _unlocked = Unlocked(vec![sub.clone()]);
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).expect("lock the folder");
    assert!(
        File::create(sub.join("probe")).is_err(),
        "this test needs a user the folder's mode refuses, not root"
    );

    let outcome = place.remove(&[], None);

    let error = outcome.expect_err("the call gives the failure at its end");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    assert!(
        stuck_file.exists(),
        "the file that could not go is still there"
    );
    for id in ids.iter().filter(|id| **id != stuck) {
        assert!(
            !place.session(id).exists(),
            "{id} went though {stuck} did not"
        );
    }
}

#[cfg(target_os = "macos")]
struct Unflagged(Vec<PathBuf>);

#[cfg(target_os = "macos")]
impl Drop for Unflagged {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = std::process::Command::new("/usr/bin/chflags")
                .arg("nouchg")
                .arg(file)
                .status();
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_failure_the_call_gives_is_the_first_of_two_sessions_that_cannot_be_removed() {
    use std::os::unix::fs::PermissionsExt;

    const EPERM: i32 = 1;
    const EACCES: i32 = 13;

    let place = Place::new();
    for id in ["s1", "s2", "s3"] {
        place.write(id, "r-1.md", &noise(9_000, 22));
    }
    let listed: Vec<String> = fs::read_dir(place.folder.join(REVIEWS))
        .expect("list the sessions")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .into_string()
                .expect("text")
        })
        .collect();
    let (first, second) = (&listed[0], &listed[1]);
    let refusing_folder = place
        .write(first, "sub/r-2.md", &noise(9_000, 23))
        .parent()
        .expect("a folder")
        .to_owned();
    let immutable_file = place.write(second, "r-3.md", &noise(9_000, 24));
    let _unlocked = Unlocked(vec![refusing_folder.clone()]);
    let _unflagged = Unflagged(vec![immutable_file.clone()]);
    fs::set_permissions(&refusing_folder, fs::Permissions::from_mode(0o555))
        .expect("lock the folder");
    assert!(
        File::create(refusing_folder.join("probe")).is_err(),
        "this test needs a user the folder's mode refuses, not root"
    );
    let flagged = std::process::Command::new("/usr/bin/chflags")
        .arg("uchg")
        .arg(&immutable_file)
        .status()
        .expect("run chflags");
    assert!(flagged.success(), "mark the file immutable");

    let outcome = place.remove(&[], None);

    let error = outcome.expect_err("the call gives a failure at its end");
    assert_eq!(
        error.raw_os_error(),
        Some(EACCES),
        "the folder of {first} refused first, the file of {second} (EPERM = {EPERM}) after: {error}"
    );
}
