//! System tests of the place where the two programs meet when the socket path they are
//! given is longer than the system can bind.

use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use yaar_link::{Message, SOCKET_PATH_MAX, socket_address, write_message};
use yaar_system_tests::{Client, World, attach, is_alive, program, wait_until};

/// A socket path of a world that is too long to bind, with the short place it leads to.
///
/// The path lies in the world, so no other test and no other run computes the same short
/// place. When the test ends, whatever it caused there is removed: the folder, and a
/// Keeper that the Window program started and the test did not come to stop.
struct TooLong {
    wanted: PathBuf,
    short: PathBuf,
    keeper: Option<u32>,
}

impl TooLong {
    fn of(world: &World) -> TooLong {
        let wanted = world
            .home()
            .join("d".repeat(SOCKET_PATH_MAX))
            .join("keeper.sock");
        let short = socket_address(&wanted);
        assert!(wanted.as_os_str().len() > SOCKET_PATH_MAX);
        assert!(short.as_os_str().len() <= SOCKET_PATH_MAX);
        TooLong {
            wanted,
            short,
            keeper: None,
        }
    }

    fn folder(&self) -> &Path {
        self.short.parent().expect("the short path has a folder")
    }

    /// Runs the Window program as the world's user, told to find or start the Keeper at
    /// the long path and to say who answered.
    fn keeper_status(&self, world: &World) -> Output {
        world
            .command(&program("yaar-review"))
            .arg("--keeper-status")
            .env("YAAR_SOCKET", &self.wanted)
            .env("YAAR_KEEPER_LINGER_MS", "20000")
            .output()
            .expect("run the Window program")
    }
}

impl Drop for TooLong {
    fn drop(&mut self) {
        if let Some(pid) = self.keeper.filter(|pid| is_alive(*pid)) {
            // SAFETY: a signal to a process this test caused to start.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
        let _ = std::fs::remove_dir_all(self.folder());
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
    let mut long = TooLong::of(&world);

    // The Window program finds no Keeper for the long path and starts one.
    let first = said(&long.keeper_status(&world));
    let pid: u32 = field(&first, "pid").parse().expect("a process id");
    long.keeper = Some(pid);
    assert_eq!(field(&first, "started"), "true");
    assert!(is_alive(pid), "the Keeper ended with the Window program");

    // Nothing is at the long path, which cannot be bound. The Keeper listens at the
    // short one, which both programs computed from the long one alone.
    assert!(!long.wanted.exists());
    let bound = std::fs::symlink_metadata(&long.short).expect("the short path");
    assert!(bound.file_type().is_socket());

    // A second Window program finds that Keeper again by the long path, and starts none.
    let second = said(&long.keeper_status(&world));
    assert_eq!(field(&second, "started"), "false");
    assert_eq!(field(&second, "pid"), pid.to_string());

    // A scripted client reaches the same process by the same long path.
    let client = Client::attach(&long.wanted);
    assert_eq!(client.keeper().pid, pid);
    drop(client);

    // Whoever goes straight to the short path meets it too, and stops it.
    let (mut window, who) = attach(&long.short);
    assert_eq!(who.pid, pid);
    write_message(&mut window, &Message::Shutdown).expect("ask for the stop");
    assert!(
        wait_until(Duration::from_secs(5), || !is_alive(pid)),
        "the Keeper did not stop"
    );
}

#[test]
fn the_window_program_starts_no_keeper_at_a_short_place_that_is_not_the_users_own() {
    let world = World::new();
    let long = TooLong::of(&world);
    // Somebody was first at the name anyone can compute, and left the folder open to
    // others.
    std::fs::DirBuilder::new()
        .mode(0o755)
        .create(long.folder())
        .expect("make the folder");
    std::fs::set_permissions(
        long.folder(),
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
            && reason.contains(&long.short.display().to_string()),
        "{reason}"
    );
    let left: Vec<_> = std::fs::read_dir(long.folder())
        .expect("read the folder")
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
