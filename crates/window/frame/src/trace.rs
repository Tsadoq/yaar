//! Marks of a start, for timing it from outside the app. With `YAAR_TRACE_START` set, each mark is
//! one line on standard error: `yaar-trace <name> epoch_ms=<ms since 1970>`.

use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Prints the mark once per name: called from code that runs on every frame.
pub fn mark_once(name: &'static str) {
    if first_of(name) {
        mark(name);
    }
}

pub fn first_of(name: &'static str) -> bool {
    static ASKED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    // A panic elsewhere while the list was held leaves it consistent: a name is pushed or it is
    // not.
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

    // The names differ from test to test because the record is the process's and the tests of one
    // process share it.

    #[test]
    fn a_new_name_is_first_and_then_it_is_not() {
        assert!(first_of("first-then-not"));
        assert!(!first_of("first-then-not"));
        // The same text at another address is the same name: the literal above may be merged with
        // any other of the crate, and a copy is not.
        let copy: &'static str = Box::leak(String::from("first-then-not").into_boxed_str());
        assert!(!first_of(copy));
    }

    #[test]
    fn another_name_is_still_first() {
        assert!(first_of("one-name"));
        assert!(first_of("another-name"));
        assert!(!first_of("one-name"));
        assert!(!first_of("another-name"));
    }

    // Printing once is read in `tests/trace_once.rs`, from a child process with `YAAR_TRACE_START`
    // set.
    #[test]
    fn a_mark_taken_once_is_taken_for_good() {
        mark_once("taken-once");
        assert!(!first_of("taken-once"));
        mark_once("taken-once");
        assert!(!first_of("taken-once"));
    }
}
