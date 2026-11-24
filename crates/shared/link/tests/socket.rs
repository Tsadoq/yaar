use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_link::{SOCKET_PATH_MAX, check_private_folder, connect, socket_address};

struct Place(PathBuf);

impl Place {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!("/tmp/yaar-link-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).expect("make the test's folder");
        Self(path)
    }

    fn socket_of(&self, length: usize) -> PathBuf {
        let folder = self.0.as_os_str().len() + "/".len();
        let path = self.0.join("s".repeat(length - folder));
        assert_eq!(path.as_os_str().len(), length);
        path
    }

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

struct TooLong {
    wanted: PathBuf,
    short: PathBuf,
}

impl TooLong {
    fn new(place: &Place) -> Self {
        let wanted = place.socket_of(SOCKET_PATH_MAX + 1);
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
        // The mask of this process may have removed bits at creation.
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

fn is_administrator() -> bool {
    // SAFETY: `geteuid` only reads and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

const SOMEBODY_ELSE: u32 = 1;

#[test]
fn a_folder_of_another_user_is_refused() {
    if !is_administrator() {
        return;
    }
    let place = Place::new();
    let folder = place.private("theirs");
    std::os::unix::fs::chown(&folder, Some(SOMEBODY_ELSE), None).expect("give the folder away");
    assert_eq!(
        kind_of(check_private_folder(&folder)),
        ErrorKind::PermissionDenied
    );
}

fn listener_of(user: u32, socket: &Path) -> Child {
    // SAFETY: an address is plain numbers and bytes, and all zeros is a valid one.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let path = socket.as_os_str().as_bytes();
    assert!(path.len() < address.sun_path.len());
    for (to, from) in address.sun_path.iter_mut().zip(path) {
        *to = *from as libc::c_char;
    }
    let length = size_of::<libc::sockaddr_un>() as libc::socklen_t;

    // The listening socket belongs to the user who called `listen`, so the child binds and listens
    // itself after the uid change; `cat` then only waits on stdin.
    let mut stays = Command::new("cat");
    stays.uid(user).stdin(Stdio::piped()).stdout(Stdio::null());
    // SAFETY: the three calls may be made between `fork` and `exec`, and the closure
    // allocates nothing. The socket stays open in the program that is started.
    unsafe {
        stays.pre_exec(move || {
            let socket = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
            if socket < 0
                || libc::bind(socket, (&raw const address).cast(), length) != 0
                || libc::listen(socket, 8) != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    stays.spawn().expect("start a process that listens")
}

#[test]
fn a_process_of_another_user_at_the_other_end_is_refused() {
    if !is_administrator() {
        return;
    }
    let place = Place::new();
    fs::set_permissions(&place.0, fs::Permissions::from_mode(0o755)).expect("open the way");
    let folder = place.private("theirs");
    std::os::unix::fs::chown(&folder, Some(SOMEBODY_ELSE), None).expect("give the folder away");
    let socket = folder.join("keeper.sock");
    let mut theirs = listener_of(SOMEBODY_ELSE, &socket);

    assert_eq!(socket_address(&socket), socket);
    let refused = connect(&socket);
    theirs.kill().expect("end the process");
    theirs.wait().expect("collect it");
    assert_eq!(kind_of(refused), ErrorKind::PermissionDenied);
}

#[test]
fn the_longest_path_the_system_binds_is_kept_and_one_byte_more_leads_to_the_short_place() {
    let place = Place::new();

    let fits = place.socket_of(SOCKET_PATH_MAX);
    assert_eq!(socket_address(&fits), fits);
    let listener = UnixListener::bind(&fits).expect("the system binds a path of this length");
    let stream = connect(&fits).expect("reach the listener");
    assert_eq!(
        stream.peer_addr().expect("where it leads").as_pathname(),
        Some(fits.as_path())
    );
    drop(listener);

    let over = place.socket_of(SOCKET_PATH_MAX + 1);
    UnixListener::bind(&over).expect_err("the system binds no path of this length");
    let short = socket_address(&over);
    assert_ne!(short, over);
    assert!(short.as_os_str().len() <= SOCKET_PATH_MAX);
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
    assert_eq!(
        kind_of(connect(&place.0.join("keeper.sock"))),
        ErrorKind::NotFound
    );
    let stale = place.0.join("stale.sock");
    drop(UnixListener::bind(&stale).expect("bind"));
    assert_eq!(kind_of(connect(&stale)), ErrorKind::ConnectionRefused);

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
    fs::set_permissions(&place.0, fs::Permissions::from_mode(0o755)).expect("open the folder");
    let stream = connect(&socket).expect("connect");
    let (accepted, _) = listener.accept().expect("accept");
    assert_eq!(
        stream.peer_addr().expect("where it leads").as_pathname(),
        Some(socket.as_path())
    );
    drop(accepted);
}
