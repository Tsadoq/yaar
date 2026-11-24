//! System tests of a socket path longer than the system can bind.

use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use yaar_link::{Hello, Message, SOCKET_PATH_MAX, handshake, socket_address, write_message};
use yaar_system_tests::{Client, World, attach, is_alive, program, wait_until};

/// A socket path of exact length, with the short place both programs bind for it.
/// Drop removes the short folder and any Keeper the Window program started.
struct Wanted {
    path: PathBuf,
    bound: PathBuf,
    keeper: Option<u32>,
}

impl Wanted {
    fn of_length(world: &World, length: usize) -> Wanted {
        let home = world.home();
        let folder = home.as_os_str().len() + "/".len();
        let path = home.join("s".repeat(length - folder));
        assert_eq!(path.as_os_str().len(), length);
        let bound = socket_address(&path);
        Wanted {
            path,
            bound,
            keeper: None,
        }
    }

    fn short_folder(&self) -> &Path {
        assert_ne!(self.bound, self.path, "this path is bound where it is");
        self.bound.parent().expect("the short path has a folder")
    }

    fn keeper_status(&self, world: &World) -> Output {
        world
            .command(&program("yaar-review"))
            .arg("--keeper-status")
            .env("YAAR_SOCKET", &self.path)
            .env("YAAR_KEEPER_LINGER_MS", "20000")
            .output()
            .expect("run the Window program")
    }

    fn stop_keeper(&mut self) {
        let pid = self.keeper.expect("the Window program named a Keeper");
        let (mut window, who) = attach(&self.bound);
        assert_eq!(who.pid, pid);
        write_message(&mut window, &Message::Shutdown).expect("ask for the stop");
        assert!(
            wait_until(Duration::from_secs(5), || !is_alive(pid)),
            "the Keeper did not stop"
        );
        // Its pid may belong to another process from now on.
        self.keeper = None;
    }
}

impl Drop for Wanted {
    fn drop(&mut self) {
        match self.keeper {
            Some(pid) if is_alive(pid) => {
                // SAFETY: a signal to a process this test caused to start.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
            Some(_) => {}
            // Either the test stopped its Keeper, or the Window program started one and failed
            // before naming it: whatever answers where that Keeper would listen is asked to stop.
            None => {
                if let Ok(mut stream) = UnixStream::connect(&self.bound)
                    && stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .is_ok()
                    && handshake(&mut stream, &Hello::for_this_process("scripted-window")).is_ok()
                {
                    let _ = write_message(&mut stream, &Message::Shutdown);
                }
            }
        }
        if self.bound != self.path {
            let _ = std::fs::remove_dir_all(self.short_folder());
        }
    }
}

fn field(said: &str, name: &str) -> String {
    said.split_whitespace()
        .find_map(|word| word.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} in: {said}"))
        .to_owned()
}

fn said(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_window_program_and_a_scripted_client_meet_the_keeper_of_a_socket_path_too_long_to_bind() {
    let world = World::new();

    let mut fits = Wanted::of_length(&world, SOCKET_PATH_MAX);
    assert_eq!(fits.bound, fits.path);
    let status = said(&fits.keeper_status(&world));
    let pid: u32 = field(&status, "pid").parse().expect("a process id");
    fits.keeper = Some(pid);
    assert_eq!(field(&status, "started"), "true");
    let bound = std::fs::symlink_metadata(&fits.path).expect("the path itself");
    assert!(bound.file_type().is_socket());
    fits.stop_keeper();

    let mut long = Wanted::of_length(&world, SOCKET_PATH_MAX + 1);
    UnixListener::bind(&long.path).expect_err("the system binds no path of this length");
    assert_ne!(long.bound, long.path);
    assert!(long.bound.as_os_str().len() <= SOCKET_PATH_MAX);

    let first = said(&long.keeper_status(&world));
    let pid: u32 = field(&first, "pid").parse().expect("a process id");
    long.keeper = Some(pid);
    assert_eq!(field(&first, "started"), "true");
    assert!(is_alive(pid), "the Keeper ended with the Window program");

    assert!(!long.path.exists());
    let bound = std::fs::symlink_metadata(&long.bound).expect("the short path");
    assert!(bound.file_type().is_socket());

    let second = said(&long.keeper_status(&world));
    assert_eq!(field(&second, "started"), "false");
    assert_eq!(field(&second, "pid"), pid.to_string());

    let client = Client::attach(&long.path);
    assert_eq!(client.keeper().pid, pid);
    drop(client);

    long.stop_keeper();
}

#[test]
fn the_window_program_starts_no_keeper_at_a_short_place_that_is_not_the_users_own() {
    let world = World::new();
    let long = Wanted::of_length(&world, SOCKET_PATH_MAX + 1);
    // Somebody was first at the name anyone can compute, and left the folder open to others.
    std::fs::DirBuilder::new()
        .mode(0o755)
        .create(long.short_folder())
        .expect("make the folder");
    std::fs::set_permissions(
        long.short_folder(),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("open it");

    let refused = long.keeper_status(&world);
    assert!(!refused.status.success(), "the Window program went on");
    let reason = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        reason.contains("does not belong to this user")
            && reason.contains(&long.bound.display().to_string()),
        "{reason}"
    );
    let left: Vec<_> = std::fs::read_dir(long.short_folder())
        .expect("read the folder")
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
