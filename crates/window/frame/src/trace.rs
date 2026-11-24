//! Marks of a start, for timing it from outside the app.
//!
//! With `YAAR_TRACE_START` set, each mark is one line on standard error:
//! `yaar-trace <name> epoch_ms=<milliseconds since 1970>`. Without it nothing is printed.

use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// Prints the mark `name` with the current time, if start tracing is on.
pub fn mark(name: &str) {
    if std::env::var_os("YAAR_TRACE_START").is_none() {
        return;
    }
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    eprintln!("yaar-trace {name} epoch_ms={ms}");
}

/// Prints the mark `name` as [`mark`] does, the first time this process asks for `name`
/// only: a mark such as the first diff drawn is called from code that runs on every frame.
pub fn mark_once(name: &'static str) {
    if first_of(name) {
        mark(name);
    }
}

/// Whether this is the first time this process asks for `name`: true once, false after.
/// [`mark_once`] prints only on true, so a test that drew a diff can read here whether the
/// mark of the first diff was taken.
pub fn first_of(name: &'static str) -> bool {
    static ASKED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    // A panic elsewhere while the list was held leaves it as consistent as it was: a
    // name is pushed or it is not.
    let mut asked = ASKED.lock().unwrap_or_else(PoisonError::into_inner);
    if asked.contains(&name) {
        return false;
    }
    asked.push(name);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // The names differ from test to test because the record is the process's, and the
    // tests of one process share it.

    #[test]
    fn a_new_name_is_first_and_then_it_is_not() {
        assert!(first_of("first-then-not"));
        assert!(!first_of("first-then-not"));
        assert!(!first_of("first-then-not"));
    }

    #[test]
    fn another_name_is_still_first() {
        assert!(first_of("one-name"));
        assert!(first_of("another-name"));
        assert!(!first_of("one-name"));
        assert!(!first_of("another-name"));
    }

    // That the mark prints only once is not read here: it prints to standard error and only
    // with `YAAR_TRACE_START` set, which a test neither sets nor reads. What is read is that
    // `mark_once` takes the name.
    #[test]
    fn a_mark_taken_once_is_taken_for_good() {
        mark_once("taken-once");
        assert!(!first_of("taken-once"));
        // A second ask neither fails nor starts the name over.
        mark_once("taken-once");
        assert!(!first_of("taken-once"));
    }
}
