//! System tests of the place where the two programs meet when the socket path they are
//! given is longer than the system can bind.

use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use yaar_link::{Hello, Message, SOCKET_PATH_MAX, handshake, socket_address, write_message};
use yaar_system_tests::{Client, World, attach, is_alive, program, wait_until};

/// A socket path of a world, of a length chosen to the byte, with the place where the
/// two programs meet when they are given it.
///
/// The path lies in the world, so no other test and no other run computes the same short
/// place. When the test ends, whatever it caused is removed: the folder of the short
/// place, and a Keeper that the Window program started and the test did not come to stop.
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

    /// The folder of the short place. Only for a path that leads to one.
    fn short_folder(&self) -> &Path {
        assert_ne!(self.bound, self.path, "this path is bound where it is");
        self.bound.parent().expect("the short path has a folder")
    }

    /// Runs the Window program as the world's user, told to find or start the Keeper at
    /// this path and to say who answered.
    fn keeper_status(&self, world: &World) -> Output {
        world
            .command(&program("yaar-review"))
            .arg("--keeper-status")
            .env("YAAR_SOCKET", &self.path)
            .env("YAAR_KEEPER_LINGER_MS", "20000")
            .output()
            .expect("run the Window program")
    }

    /// Goes straight to where the Keeper listens, makes sure that the one there is the
    /// one the Window program named, and stops it.
    fn stop_keeper(&mut self) {
        let pid = self.keeper.expect("the Window program named a Keeper");
        let (mut window, who) = attach(&self.bound);
        assert_eq!(who.pid, pid);
        write_message(&mut window, &Message::Shutdown).expect("ask for the stop");
        assert!(
            wait_until(Duration::from_secs(5), || !is_alive(pid)),
            "the Keeper did not stop"
        );
        // It is gone, and its number may be another process's from now on.
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
            // The test stopped its Keeper, and then nothing answers. Or the Window
            // program started one and failed before it said which: whatever answers
            // where that Keeper would listen is asked to stop, so that none stays
            // behind for as long as it lingers.
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

/// The value of one `name=value` word of what `--keeper-status` printed.
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

// KEEP-01, the half of the built programs.
#[test]
fn the_window_program_and_a_scripted_client_meet_the_keeper_of_a_socket_path_too_long_to_bind() {
    let world = World::new();

    // What the system allows, to the byte: given a path of that length, the Window
    // program starts a Keeper that listens at the path itself.
    let mut fits = Wanted::of_length(&world, SOCKET_PATH_MAX);
    assert_eq!(fits.bound, fits.path);
    let status = said(&fits.keeper_status(&world));
    let pid: u32 = field(&status, "pid").parse().expect("a process id");
    fits.keeper = Some(pid);
    assert_eq!(field(&status, "started"), "true");
    let bound = std::fs::symlink_metadata(&fits.path).expect("the path itself");
    assert!(bound.file_type().is_socket());
    fits.stop_keeper();

    // One byte more, in the same folder, and the system refuses the path.
    let mut long = Wanted::of_length(&world, SOCKET_PATH_MAX + 1);
    UnixListener::bind(&long.path).expect_err("the system binds no path of this length");
    assert_ne!(long.bound, long.path);
    assert!(long.bound.as_os_str().len() <= SOCKET_PATH_MAX);

    // The Window program finds no Keeper for the long path and starts one.
    let first = said(&long.keeper_status(&world));
    let pid: u32 = field(&first, "pid").parse().expect("a process id");
    long.keeper = Some(pid);
    assert_eq!(field(&first, "started"), "true");
    assert!(is_alive(pid), "the Keeper ended with the Window program");

    // Nothing is at the long path, which cannot be bound. The Keeper listens at the
    // short one, which both programs computed from the long one alone.
    assert!(!long.path.exists());
    let bound = std::fs::symlink_metadata(&long.bound).expect("the short path");
    assert!(bound.file_type().is_socket());

    // A second Window program finds that Keeper again by the long path, and starts none.
    let second = said(&long.keeper_status(&world));
    assert_eq!(field(&second, "started"), "false");
    assert_eq!(field(&second, "pid"), pid.to_string());

    // A scripted client reaches the same process by the same long path.
    let client = Client::attach(&long.path);
    assert_eq!(client.keeper().pid, pid);
    drop(client);

    // Whoever goes straight to the short path meets it too, and stops it.
    long.stop_keeper();
}

#[test]
fn the_window_program_starts_no_keeper_at_a_short_place_that_is_not_the_users_own() {
    let world = World::new();
    let long = Wanted::of_length(&world, SOCKET_PATH_MAX + 1);
    // Somebody was first at the name anyone can compute, and left the folder open to
    // others.
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
    // The reason is that the place is not the user's, and not that a Keeper was started
    // and did not answer: none was started.
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
