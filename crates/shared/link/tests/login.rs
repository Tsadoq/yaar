use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use yaar_link::login::{LoginShell, ShellError, ShellGroup, take_in};

/// A killed shell is gone within a fraction of a second; an unkilled one would run to this
/// deadline.
const DEADLINE: Duration = Duration::from_secs(10);

struct Place(PathBuf);

impl Place {
    fn new() -> Self {
        let path = PathBuf::from(format!("/tmp/yaar-link-login-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("make the test's folder");
        Self(path)
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_shell_started_after_the_group_has_ended_is_killed_at_its_start() {
    let place = Place::new();
    let program = place.0.join("login-shell");
    std::fs::write(&program, "#!/bin/sh\nexec sleep 600\n").expect("write the shell");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
        .expect("make it run");
    let base: Vec<(OsString, OsString)> = vec![
        (
            "PATH".into(),
            std::env::var_os("PATH").expect("a search path"),
        ),
        ("HOME".into(), place.0.clone().into_os_string()),
    ];
    let group = ShellGroup::default();
    group.end();

    let began = Instant::now();
    let result = take_in(
        &LoginShell {
            program,
            deadline: DEADLINE,
        },
        &base,
        &group,
    );

    assert_eq!(
        result,
        Err(ShellError::Status(None)),
        "the shell was not ended by a signal (the take ended after {:?})",
        began.elapsed()
    );
    assert!(
        began.elapsed() < DEADLINE,
        "the take waited {:?}, the shell's deadline",
        began.elapsed()
    );
}
