// Each test program uses the helpers it needs, and none uses all of them.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use yaar_adapter_claude::ClaudeCode;
use yaar_agent::{Adapter, Asked, CliSession, Event, Report, Reported};
use yaar_system_tests::recorded::{self, HookRecord};

pub fn feed(adapter: &mut ClaudeCode, record: &HookRecord) -> Vec<Reported> {
    adapter
        .report(&Report {
            terminal: record.terminal(),
            pid: record.parent,
            payload: &record.input_bytes(),
        })
        .expect("a recorded hook is a report")
}

pub fn feed_asked(adapter: &mut ClaudeCode, folder: &Path, record: &HookRecord) -> Option<Asked> {
    feed(adapter, record);
    adapter
        .asked(
            folder,
            &Report {
                terminal: record.terminal(),
                pid: record.parent,
                payload: &record.input_bytes(),
            },
        )
        .expect("a recorded hook can be asked about")
}

pub fn replay(name: &str) -> (ClaudeCode, Vec<HookRecord>, Vec<Vec<Reported>>) {
    let records = recorded::hook_events(name);
    let mut adapter = ClaudeCode::new();
    let by_line = records
        .iter()
        .map(|record| feed(&mut adapter, record))
        .collect();
    (adapter, records, by_line)
}

pub fn told(record: &HookRecord, event: Event) -> Reported {
    Reported {
        session: CliSession(record.session().to_owned()),
        terminal: record.terminal().map(str::to_owned),
        pid: record.parent,
        folder: Some(PathBuf::from(recorded::PROJECT)),
        cli_record: None,
        event,
    }
}

pub fn in_project(relative: &str) -> PathBuf {
    Path::new(recorded::PROJECT).join(relative)
}

pub fn command_of(record: &HookRecord) -> String {
    record.input["tool_input"]["command"]
        .as_str()
        .expect("the input of a shell command holds its text")
        .to_owned()
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
        let root = temp.join(format!("yaar-adapter-claude-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).expect("make the scratch folder");
        Scratch { root }
    }

    /// The folder itself, with every link in its path resolved.
    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
