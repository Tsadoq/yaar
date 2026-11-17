//! Marks of a start, for timing it from outside the app.
//!
//! With `YAAR_TRACE_START` set, each mark is one line on standard error:
//! `yaar-trace <name> epoch_ms=<milliseconds since 1970>`. Without it nothing is printed.

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
