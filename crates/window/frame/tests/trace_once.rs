//! Runs itself in a child process: YAAR_TRACE_START cannot be set in a test's own process (the
//! environment is shared by every thread).

use std::process::{Command, Stdio};

use yaar_frame::trace::mark_once;

const CHILD: &str = "YAAR_TRACE_ONCE_CHILD";

const TEST: &str = "a_mark_asked_for_twice_is_printed_once";

#[test]
fn a_mark_asked_for_twice_is_printed_once() {
    if std::env::var_os(CHILD).is_some() {
        mark_once("asked-for-twice");
        mark_once("asked-for-twice");
        mark_once("asked-for-once");
        return;
    }
    let child = Command::new(std::env::current_exe().expect("the test's own program"))
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .env("YAAR_TRACE_START", "1")
        .stdin(Stdio::null())
        .output()
        .expect("start the child");
    let errors = String::from_utf8_lossy(&child.stderr);
    let output = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success(),
        "the child failed: {output}\n{errors}"
    );
    let marks = |name: &str| {
        errors
            .lines()
            .filter(|line| {
                line.strip_prefix(&format!("yaar-trace {name} epoch_ms="))
                    .is_some_and(|ms| ms.parse::<u128>().is_ok())
            })
            .count()
    };
    assert_eq!(marks("asked-for-once"), 1, "{output}\n{errors}");
    assert_eq!(marks("asked-for-twice"), 1, "{output}\n{errors}");
}
