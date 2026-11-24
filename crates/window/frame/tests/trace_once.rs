//! Component test: `trace::mark_once` prints its mark once, however often it is asked for.
//!
//! A mark is a line on standard error, and only with `YAAR_TRACE_START` set, which no test may
//! change in its own process (the environment is shared by every thread). So the test starts a
//! child of its own binary, which runs only this test in "child" mode, with the variable in
//! that child's environment alone, and reads the child's standard error.

use std::process::{Command, Stdio};

use yaar_frame::trace::mark_once;

/// Set in the environment of the child, so that the test in it asks for the marks and the
/// test in the parent starts the child.
const CHILD: &str = "YAAR_TRACE_ONCE_CHILD";

/// What the test is called, for the child to run it alone.
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
    // The child ended, so its standard error is whole. It ran the test (the mark of another
    // name is there), and the name asked for twice is there once.
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
