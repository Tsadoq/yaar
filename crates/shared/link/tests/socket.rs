//! Component test: reaching a socket through the Link's own rule, on real folders and
//! real sockets.

use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_link::{SOCKET_PATH_MAX, check_private_folder, connect, socket_address};

/// A folder of this test's own, removed when the test ends.
struct Place(PathBuf);

impl Place {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!("/tmp/yaar-link-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).expect("make the test's folder");
        Self(path)
    }

    /// A folder inside, made for the user alone.
    fn private(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&folder)
            .expect("make a private folder");
        folder
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A socket path too long to bind, that no other test and no other run asks for, with the
/// short place it leads to. The folder of the short place is removed when the test ends.
struct TooLong {
    wanted: PathBuf,
    short: PathBuf,
}

impl TooLong {
    fn new(place: &Place) -> Self {
        let wanted = place
            .0
            .join("d".repeat(SOCKET_PATH_MAX))
            .join("keeper.sock");
        let short = socket_address(&wanted);
        assert_ne!(short, wanted, "a path this long cannot be bound");
        Self { wanted, short }
    }

    fn folder(&self) -> &Path {
        self.short.parent().expect("the short path has a folder")
    }

    fn make_folder(&self, mode: u32) {
        fs::DirBuilder::new()
            .mode(mode)
            .create(self.folder())
            .expect("make the folder of the short path");
        // Whatever the mask of this process took away or left.
        fs::set_permissions(self.folder(), fs::Permissions::from_mode(mode)).expect("set its mode");
    }
}

impl Drop for TooLong {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.folder());
    }
}

fn kind_of<T>(result: io::Result<T>) -> ErrorKind {
    match result {
        Ok(_) => panic!("this had to be refused"),
        Err(e) => e.kind(),
    }
}

#[test]
fn a_folder_made_for_the_user_alone_passes() {
    let place = Place::new();
    check_private_folder(&place.private("mine")).expect("a private folder");
}

#[test]
fn a_folder_that_others_can_enter_is_refused() {
    let place = Place::new();
    let folder = place.private("open");
    for mode in [0o755, 0o750, 0o701, 0o770, 0o707] {
        fs::set_permissions(&folder, fs::Permissions::from_mode(mode)).expect("open it");
        assert_eq!(
            kind_of(check_private_folder(&folder)),
            ErrorKind::PermissionDenied,
            "mode {mode:o}"
        );
    }
}

#[test]
fn a_link_to_a_private_folder_is_refused() {
    let place = Place::new();
    let folder = place.private("mine");
    let link = place.0.join("link");
    symlink(&folder, &link).expect("make the link");
    assert_eq!(
        kind_of(check_private_folder(&link)),
        ErrorKind::PermissionDenied
    );
}

#[test]
fn a_file_is_refused_and_a_folder_that_is_not_there_is_the_systems_own_error() {
    let place = Place::new();
    let file = place.0.join("file");
    fs::write(&file, "").expect("write a file");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).expect("close it");
    assert_eq!(
        kind_of(check_private_folder(&file)),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        kind_of(check_private_folder(&place.0.join("nothing"))),
        ErrorKind::NotFound
    );
}

#[test]
fn a_folder_of_another_user_is_refused() {
    // Only the administrator can give a folder away, so this is shown where the tests
    // run as one, which is the container of the Linux checks. Elsewhere there is no
    // folder of another user that is closed to everybody else and can be counted on.
    // SAFETY: `geteuid` only reads and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let place = Place::new();
    let folder = place.private("theirs");
    std::os::unix::fs::chown(&folder, Some(1), None).expect("give the folder away");
    assert_eq!(
        kind_of(check_private_folder(&folder)),
        ErrorKind::PermissionDenied
    );
}

#[test]
fn a_long_path_is_reached_at_its_short_place_while_the_folder_is_the_users_own() {
    let place = Place::new();
    let long = TooLong::new(&place);
    long.make_folder(0o700);
    let listener = UnixListener::bind(&long.short).expect("bind the short path");

    let stream = connect(&long.wanted).expect("reach the listener by the long path");
    let (accepted, _) = listener.accept().expect("accept");
    assert_eq!(
        stream.peer_addr().expect("where it leads").as_pathname(),
        Some(long.short.as_path())
    );
    drop(accepted);
}

#[test]
fn a_short_place_whose_folder_is_open_to_others_is_not_connected_to() {
    let place = Place::new();
    let long = TooLong::new(&place);
    long.make_folder(0o755);
    let listener = UnixListener::bind(&long.short).expect("bind the short path");
    listener.set_nonblocking(true).expect("do not wait");

    assert_eq!(kind_of(connect(&long.wanted)), ErrorKind::PermissionDenied);
    // Refused before anything was connected: nobody waits at the listener.
    assert_eq!(kind_of(listener.accept()), ErrorKind::WouldBlock);
}

#[test]
fn a_short_place_behind_a_link_is_not_connected_to() {
    let place = Place::new();
    let long = TooLong::new(&place);
    // Somebody else was first: the name anyone can compute leads to a folder of theirs.
    let elsewhere = place.private("elsewhere");
    symlink(&elsewhere, long.folder()).expect("make the link");
    let listener = UnixListener::bind(elsewhere.join("keeper.sock")).expect("bind");
    listener.set_nonblocking(true).expect("do not wait");

    assert_eq!(kind_of(connect(&long.wanted)), ErrorKind::PermissionDenied);
    assert_eq!(kind_of(listener.accept()), ErrorKind::WouldBlock);
}

#[test]
fn where_nothing_listens_the_error_is_the_systems_own() {
    let place = Place::new();
    // A path that can be bound, with no socket file.
    assert_eq!(
        kind_of(connect(&place.0.join("keeper.sock"))),
        ErrorKind::NotFound
    );
    // A socket file nobody listens at any more.
    let stale = place.0.join("stale.sock");
    drop(UnixListener::bind(&stale).expect("bind"));
    assert_eq!(kind_of(connect(&stale)), ErrorKind::ConnectionRefused);

    // A long path whose short place was never made, and one whose folder is there and
    // private with nothing in it.
    let long = TooLong::new(&place);
    assert_eq!(kind_of(connect(&long.wanted)), ErrorKind::NotFound);
    long.make_folder(0o700);
    assert_eq!(kind_of(connect(&long.wanted)), ErrorKind::NotFound);
}

#[test]
fn a_path_that_can_be_bound_is_reached_as_it_is() {
    let place = Place::new();
    let socket = place.0.join("keeper.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    // The folder is open to others here, as the app's own folder may be: the check of
    // the folder is for the short place only. The process at the other end is this one.
    fs::set_permissions(&place.0, fs::Permissions::from_mode(0o755)).expect("open the folder");
    let stream = connect(&socket).expect("connect");
    let (accepted, _) = listener.accept().expect("accept");
    assert_eq!(
        stream.peer_addr().expect("where it leads").as_pathname(),
        Some(socket.as_path())
    );
    drop(accepted);
}
