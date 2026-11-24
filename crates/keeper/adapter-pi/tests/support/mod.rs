//! Helpers shared by the test programs: recorded pi events fed to the Adapter, and a scratch
//! folder.

// Each test program uses the helpers it needs, and none uses all of them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_adapter_pi::Pi;
use yaar_agent::{Adapter, Asked, CliSession, Event, Report, Reported};
use yaar_system_tests::recorded::PiRecord;

pub fn feed(adapter: &mut Pi, record: &PiRecord) -> Vec<Reported> {
    adapter
        .report(&Report {
            terminal: record.terminal(),
            pid: record.pid,
            payload: &record.payload_bytes(),
        })
        .expect("a recorded event is a report")
}

/// Reports first, then the question of what the Adapter waits for, as the Keeper does.
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

pub struct Scratch {
    root: PathBuf,
}

impl Scratch {
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

    pub fn project(&self) -> PathBuf {
        let project = self.root.join("project");
        std::fs::create_dir_all(&project).expect("make the project");
        project
    }

    /// The Adapter's own folder inside it, not made: it does not exist before the app first writes
    /// there.
    pub fn adapter_folder(&self) -> PathBuf {
        self.root.join("agents").join("pi")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
