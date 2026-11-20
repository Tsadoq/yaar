//! What the tests of this crate share: the recorded events of pi given to the Adapter as the
//! Keeper hands them over, and a folder of a test's own.

// Each test program uses the helpers it needs, and none uses all of them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_adapter_pi::Pi;
use yaar_agent::{Adapter, Asked, CliSession, Event, Report, Reported};
use yaar_system_tests::recorded::PiRecord;

/// Gives the report of `record` to the Adapter as the Keeper would hand it, with the
/// terminal and process the reporting program would have named, and the payload the
/// extension would have written.
pub fn feed(adapter: &mut Pi, record: &PiRecord) -> Vec<Reported> {
    adapter
        .report(&Report {
            terminal: record.terminal(),
            pid: record.pid,
            payload: &record.payload_bytes(),
        })
        .expect("a recorded event is a report")
}

/// Gives the report of `record` to the Adapter as the Keeper does: the report first, then
/// the question of what it waits for, with the Adapter's own `folder`. Returns the events
/// and the answer to the question.
pub fn feed_asked(
    adapter: &mut Pi,
    folder: &Path,
    record: &PiRecord,
) -> (Vec<Reported>, Option<Asked>) {
    let events = feed(adapter, record);
    let asked = adapter
        .asked(
            folder,
            &Report {
                terminal: record.terminal(),
                pid: record.pid,
                payload: &record.payload_bytes(),
            },
        )
        .expect("a recorded event can be asked about");
    (events, asked)
}

/// The event as the Adapter should tell it for `record`: in its session, terminal and
/// process, with the folder and the session's file the record names.
pub fn told(record: &PiRecord, event: Event) -> Reported {
    Reported {
        session: CliSession(record.session().to_owned()),
        terminal: record.terminal().map(str::to_owned),
        pid: record.pid,
        folder: record.payload["cwd"].as_str().map(PathBuf::from),
        cli_record: record.session_file().map(PathBuf::from),
        event,
    }
}

/// A file of `project`, as the Adapter names it.
pub fn in_project(project: &Path, relative: &str) -> PathBuf {
    project.join(relative)
}

/// A folder of the test's own, removed when the test ends.
pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// Makes the folder.
    pub fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-adapter-pi-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).expect("make the scratch folder");
        Scratch { root }
    }

    /// The folder itself, with every link in its path resolved.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// A project folder inside it, made, for the recorded events to be moved into.
    pub fn project(&self) -> PathBuf {
        let project = self.root.join("project");
        std::fs::create_dir_all(&project).expect("make the project");
        project
    }

    /// The Adapter's own folder of the app inside it, not made, as it is not before the
    /// app first writes there.
    pub fn adapter_folder(&self) -> PathBuf {
        self.root.join("agents").join("pi")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
