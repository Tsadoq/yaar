//! Component tests: the Dock through its calls, with real pseudo-terminals and the
//! programs of scripts the tests write.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use yaar_dock::{
    Dock, DockError, Ending, Entry, Event, Events, LEFT_OUT_VARIABLES, RING_BYTES, STOP_STEP, Size,
    Start, TERMINAL_VARIABLES, TerminalId,
};

/// How long a test waits for a fact before it says the fact did not come.
const DEADLINE: Duration = Duration::from_secs(30);

const SIZE: Size = Size {
    columns: 80,
    rows: 24,
};

/// A folder of the test's own, with the scripts the test's programs are, removed when the
/// test ends.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let temp = std::env::temp_dir()
            .canonicalize()
            .expect("the temporary folder");
        let root = temp.join(format!("yaar-dock-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).expect("make the scratch folder");
        Scratch { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    /// A program: a script of POSIX `sh`, which can be run.
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write the script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the script runnable");
        path
    }

    fn start(&self, script: PathBuf) -> Start {
        Start::program(script, Vec::new(), self.root.clone(), SIZE)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The events of a terminal, each with the time it was told.
type Told = Vec<(Instant, Event)>;

/// What one terminal told, in order, and when.
#[derive(Clone)]
struct Watch {
    told: Arc<(Mutex<Told>, Condvar)>,
}

impl Watch {
    fn new() -> (Watch, Events) {
        let told = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
        let events = {
            let told = Arc::clone(&told);
            Box::new(move |event| {
                told.0
                    .lock()
                    .expect("the events")
                    .push((Instant::now(), event));
                told.1.notify_all();
            })
        };
        (Watch { told }, events)
    }

    /// How many events have been told, without copying them.
    fn count(&self) -> usize {
        self.told.0.lock().expect("the events").len()
    }

    /// What the program wrote in the first `count` events.
    fn output_of_first(&self, count: usize) -> Vec<u8> {
        let told = self.told.0.lock().expect("the events");
        let mut bytes = Vec::new();
        for (_, event) in &told[..count] {
            if let Event::Output(output) = event {
                bytes.extend_from_slice(output);
            }
        }
        bytes
    }

    fn events(&self) -> Vec<Event> {
        let told = self.told.0.lock().expect("the events");
        told.iter().map(|(_, event)| event.clone()).collect()
    }

    /// Waits until `done` is true of the events so far, and gives them. Fails with the
    /// events seen when the deadline passes.
    fn wait_until(&self, what: &str, done: impl Fn(&[Event]) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + DEADLINE;
        let mut told = self.told.0.lock().expect("the events");
        loop {
            let events: Vec<Event> = told.iter().map(|(_, event)| event.clone()).collect();
            if done(&events) {
                return events;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "no {what} within {DEADLINE:?}; the events so far: {:?}",
                lines(&events)
            );
            told = self.told.1.wait_timeout(told, left).expect("the events").0;
        }
    }

    /// Like `wait_until`, for a program that writes a lot: `done` looks at the events where
    /// they are, and they are copied once, when it holds.
    fn wait_told(&self, what: &str, done: impl Fn(&[(Instant, Event)]) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + DEADLINE;
        let mut told = self.told.0.lock().expect("the events");
        loop {
            if done(&told) {
                return told.iter().map(|(_, event)| event.clone()).collect();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "no {what} within {DEADLINE:?}; {} events so far",
                told.len()
            );
            told = self.told.1.wait_timeout(told, left).expect("the events").0;
        }
    }

    fn wait_for_line(&self, line: &str) -> Vec<Event> {
        self.wait_until(&format!("line {line:?}"), |events| {
            lines(events).iter().any(|seen| seen == line)
        })
    }

    fn wait_for_end(&self) -> Vec<Event> {
        self.wait_until("end", |events| {
            matches!(events.last(), Some(Event::Ended(_)))
        })
    }

    /// When the event that first made `is_it` true was told.
    fn when(&self, is_it: impl Fn(&[Event]) -> bool) -> Instant {
        let told = self.told.0.lock().expect("the events");
        let mut events = Vec::new();
        for (at, event) in told.iter() {
            events.push(event.clone());
            if is_it(&events) {
                return *at;
            }
        }
        panic!("it never became true");
    }
}

/// What the program wrote, as lines. A terminal ends a line with a carriage return and a
/// line feed, or, in raw mode, with the line feed alone; the unfinished last line, if
/// any, is a line too.
fn lines(events: &[Event]) -> Vec<String> {
    let mut bytes = Vec::new();
    for event in events {
        if let Event::Output(output) = event {
            bytes.extend_from_slice(output);
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_owned())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn open(dock: &Dock, start: Start) -> (TerminalId, Watch) {
    let id = TerminalId::random().expect("an id");
    let (watch, events) = Watch::new();
    dock.open(id.clone(), start, events)
        .expect("open a terminal");
    (id, watch)
}

fn type_line(dock: &Dock, terminal: &TerminalId, line: &str) {
    dock.write(terminal, format!("{line}\n").as_bytes())
        .expect("write to the terminal");
}

/// Whether the process is gone: not there, or only waiting to be collected by whoever
/// adopted it.
fn is_gone(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return true;
    }
    let state = if cfg!(target_os = "linux") {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(')')
                    .map(|(_, rest)| rest.trim_start().chars().take(1).collect::<String>())
            })
            .unwrap_or_else(|| "Z".to_owned())
    } else {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .expect("run ps");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    state.is_empty() || state.starts_with('Z')
}

/// Whether the process has been collected by its parent: no process of that number is
/// there at all, not even one that has ended and waits to be collected.
fn is_collected(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    let answer = unsafe { libc::kill(pid, 0) };
    answer != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Kills the process when the test ends, whatever way it ends.
struct Reap(libc::pid_t);

impl Drop for Reap {
    fn drop(&mut self) {
        // SAFETY: `kill` only sends a signal.
        unsafe { libc::kill(self.0, libc::SIGKILL) };
        let deadline = Instant::now() + DEADLINE;
        while !is_gone(self.0) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// The number after `prefix` in the line of the output that starts with it.
fn number_after(events: &[Event], prefix: &str) -> libc::pid_t {
    lines(events)
        .iter()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no line {prefix:?} in {:?}", lines(events)))
        .trim()
        .parse()
        .expect("a number")
}

/// The number a script wrote to `path` with `echo $! > path`, once the file holds a whole
/// line of it. A fact of the file system, not of the terminal: where a program shares its
/// terminal with another, no line of the terminal can be told to be whole or to begin where
/// the program began it.
fn pid_in_file(path: &Path) -> libc::pid_t {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.strip_suffix('\n')?.parse().ok())
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "no number in {path:?} within {DEADLINE:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn three_terminals_run_at_once_each_with_its_own_program_and_their_output_stays_apart() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    let names = ["alpha", "beta", "gamma"];

    // Each program writes its own name and a number 2,000 times, then answers each line
    // it is typed with its own name. No echo, so that the output is the program's alone.
    let mut terminals = Vec::new();
    for name in names {
        let script = scratch.script(
            name,
            &format!(
                "stty -echo\n\
                 i=1\n\
                 while [ $i -le 2000 ]; do echo \"{name} $i\"; i=$((i + 1)); done\n\
                 while read line; do echo \"{name} got $line\"; done\n"
            ),
        );
        let (id, watch) = open(&dock, scratch.start(script));
        terminals.push((name, id, watch));
    }
    // All three are open before any is written to.
    assert_eq!(dock.running(), 3);

    for (name, _, watch) in &terminals {
        // Each terminal's first line names its own script.
        watch.wait_for_line(&format!("{name} 1"));
        assert_eq!(lines(&watch.events())[0], format!("{name} 1"));
    }
    for (name, id, _) in &terminals {
        type_line(&dock, id, &format!("ping-{name}"));
    }
    for (name, _, watch) in &terminals {
        watch.wait_for_line(&format!("{name} got ping-{name}"));
    }

    for (name, _, watch) in &terminals {
        let expected: Vec<String> = (1..=2000)
            .map(|n| format!("{name} {n}"))
            .chain([format!("{name} got ping-{name}")])
            .collect();
        // Every line is its own, in order, and the answer is to its own line: nothing of
        // another terminal came in, and nothing else.
        assert_eq!(lines(&watch.events()), expected, "the output of {name}");
    }
    assert_eq!(dock.running(), 3);

    // Dropping the Dock ends all three, and tells each how.
    drop(dock);
    for (name, _, watch) in &terminals {
        assert!(
            matches!(watch.events().last(), Some(Event::Ended(_))),
            "{name} was not told its end"
        );
    }
}

#[test]
fn a_program_the_app_knows_nothing_of_gets_keys_and_gives_output_as_any_program() {
    let scratch = Scratch::new();
    // Raw mode, so that Ctrl-C and the arrow key reach the program as bytes; each byte it
    // reads is answered with its value in hexadecimal.
    let script = scratch.script(
        "otheragent",
        "stty raw -echo\n\
         printf 'ready\\n'\n\
         while :; do\n\
         \x20 byte=$(dd bs=1 count=1 2>/dev/null | od -An -tx1)\n\
         \x20 set -- $byte\n\
         \x20 byte=$1\n\
         \x20 [ -z \"$byte\" ] && exit 1\n\
         \x20 if [ \"$byte\" = 71 ]; then printf 'bye\\n'; exit 0; fi\n\
         \x20 printf '%s\\n' \"$byte\"\n\
         done\n",
    );
    let dock = Dock::new();
    let (id, watch) = open(&dock, scratch.start(script));

    // It runs in a terminal: its output starts with what it says, and the Dock counts it.
    let events = watch.wait_for_line("ready");
    assert_eq!(lines(&events)[0], "ready");
    assert_eq!(dock.running(), 1);

    // Keys go in, as the Window sends them: a letter, Ctrl-C, an arrow, Return.
    for keys in [&b"a"[..], &[0x03], &[0x1b, b'[', b'A'], &[0x0d]] {
        dock.write(&id, keys).expect("write keys");
    }
    watch.wait_for_line("0d");
    // The output comes back, and raw mode passed Ctrl-C on as a byte.
    assert_eq!(
        lines(&watch.events()),
        ["ready", "61", "03", "1b", "5b", "41", "0d"]
    );

    // Its own way out: the last output is what it says, and the end follows.
    dock.write(&id, b"q").expect("write q");
    let events = watch.wait_for_end();
    assert_eq!(events.last(), Some(&Event::Ended(Ending::Exited(0))));
    assert_eq!(lines(&events).last().map(String::as_str), Some("bye"));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Ended(_)))
            .count(),
        1
    );
    assert_eq!(dock.running(), 0);
}

#[test]
fn dropping_the_dock_ends_a_program_and_a_job_it_put_in_a_process_group_of_its_own() {
    let scratch = Scratch::new();
    // Job control puts the background job in a group of its own, as an interactive shell
    // does, and a shell passes no hang-up on to it.
    let script = scratch.script(
        "jobs",
        "set -m\nsleep 300 &\necho \"job $!\"\necho \"me $$\"\nwait\n",
    );
    let dock = Dock::new();
    let (_, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_until("both ids", |events| {
        let seen = lines(events);
        seen.iter().any(|line| line.starts_with("job "))
            && seen.iter().any(|line| line.starts_with("me "))
    });
    let (job, me) = (number_after(&events, "job "), number_after(&events, "me "));
    // SAFETY: both only read.
    let (group_of_job, group_of_me, session_of_job) =
        unsafe { (libc::getpgid(job), libc::getpgid(me), libc::getsid(job)) };
    assert_ne!(
        group_of_job, group_of_me,
        "the job is in the program's group"
    );
    assert_eq!(session_of_job, me, "the job is in the program's session");
    let _reap = Reap(job);

    let dropping = Instant::now();
    drop(dock);
    // Both are gone when the drop returns, and a program that takes the hang-up is not
    // pressed with the harder signals.
    assert!(is_gone(me), "the program is still there");
    assert!(is_gone(job), "the job is still there");
    assert!(
        dropping.elapsed() < STOP_STEP,
        "took {:?}",
        dropping.elapsed()
    );
    assert!(matches!(watch.events().last(), Some(Event::Ended(_))));
}

#[test]
fn a_program_takes_the_hang_up_even_when_the_keeper_ignores_it() {
    // SAFETY: setting what a signal does is sound at any time; this is the first
    // statement of a test, which `cargo nextest` runs in a process of its own.
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
    let scratch = Scratch::new();
    let script = scratch.script("patient", "echo ready\nsleep 300\n");
    let dock = Dock::new();
    let (_, watch) = open(&dock, scratch.start(script));
    watch.wait_for_line("ready");

    // The program is hung up at once, with no need of the harder signals.
    let stopping = Instant::now();
    dock.stop_all();
    assert!(
        stopping.elapsed() < STOP_STEP,
        "took {:?}",
        stopping.elapsed()
    );
    assert_eq!(
        watch.events().last(),
        Some(&Event::Ended(Ending::Signalled(libc::SIGHUP)))
    );
}

#[test]
fn a_program_that_ignores_the_hang_up_and_the_terminate_signal_is_killed_after_two_steps() {
    let scratch = Scratch::new();
    let script = scratch.script(
        "stubborn",
        "trap '' HUP TERM\necho ready\nwhile :; do sleep 1; done\n",
    );
    let dock = Dock::new();
    let (_, watch) = open(&dock, scratch.start(script));
    watch.wait_for_line("ready");

    let stopping = Instant::now();
    dock.stop_all();
    let took = stopping.elapsed();
    assert!(
        took >= STOP_STEP * 2 - Duration::from_millis(100),
        "took {took:?}"
    );
    assert!(
        took < STOP_STEP * 2 + Duration::from_secs(10),
        "took {took:?}"
    );
    assert_eq!(dock.running(), 0);
    // stop_all returns after the end was told.
    assert_eq!(
        watch.events().last(),
        Some(&Event::Ended(Ending::Signalled(libc::SIGKILL)))
    );
}

#[test]
fn a_terminal_gets_the_keepers_environment_less_other_terminals_with_its_own_on_top() {
    // SAFETY: changing the environment is sound only while no other thread reads it, as
    // a thread that starts a process does. This is the first statement of a test, and
    // `cargo nextest` gives each test a process of its own: no other thread exists yet.
    unsafe {
        for name in LEFT_OUT_VARIABLES {
            std::env::set_var(name, "from-another-terminal");
        }
        std::env::set_var("TERM", "dumb");
        std::env::set_var("YAAR_DOCK_TEST_PASSED_ON", "yes");
    }
    let scratch = Scratch::new();
    let script = scratch.script("environment", "env\n");
    let dock = Dock::new();
    let mut start = scratch.start(script);
    start.env = vec![
        (OsString::from("TERM_PROGRAM"), OsString::from("mine")),
        (OsString::from("FROM_THE_CALLER"), OsString::from("1")),
        // Of two with one name, the later wins.
        (OsString::from("TWICE"), OsString::from("first")),
        (OsString::from("TWICE"), OsString::from("second")),
    ];
    let (_, watch) = open(&dock, start);
    let events = watch.wait_for_end();

    let environment: HashMap<String, String> = lines(&events)
        .iter()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    for name in LEFT_OUT_VARIABLES {
        assert!(!environment.contains_key(name), "{name} is set");
    }
    for (name, value) in TERMINAL_VARIABLES {
        if name != "TERM_PROGRAM" {
            assert_eq!(
                environment.get(name).map(String::as_str),
                Some(value),
                "{name}"
            );
        }
    }
    // A pair of the caller wins over the terminal's own.
    assert_eq!(
        environment.get("TERM_PROGRAM").map(String::as_str),
        Some("mine")
    );
    assert_eq!(
        environment.get("FROM_THE_CALLER").map(String::as_str),
        Some("1")
    );
    assert_eq!(environment.get("TWICE").map(String::as_str), Some("second"));
    // What the Keeper had stays, and its TERM did not.
    assert_eq!(
        environment
            .get("YAAR_DOCK_TEST_PASSED_ON")
            .map(String::as_str),
        Some("yes")
    );
    assert_eq!(
        environment.get("TERM_PROGRAM_VERSION").map(String::as_str),
        Some(env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn an_id_the_dock_holds_is_refused_and_nothing_is_started() {
    let scratch = Scratch::new();
    let marker = scratch.path().join("started");
    let first = scratch.script("first", "echo ready\nread line\n");
    let second = scratch.script("second", &format!("touch '{}'\n", marker.display()));
    let dock = Dock::new();
    let id = TerminalId::random().expect("an id");
    let (watch, events) = Watch::new();
    dock.open(id.clone(), scratch.start(first), events)
        .expect("open the first");
    watch.wait_for_line("ready");

    let (other, other_events) = Watch::new();
    let refused = dock.open(id.clone(), scratch.start(second.clone()), other_events);
    assert!(
        matches!(&refused, Err(DockError::TakenId(taken)) if *taken == id),
        "{refused:?}"
    );
    assert_eq!(dock.running(), 1);

    // Also once the program has ended: an id names one terminal only.
    type_line(&dock, &id, "go");
    watch.wait_for_end();
    let (_, more_events) = Watch::new();
    let refused = dock.open(id.clone(), scratch.start(second), more_events);
    assert!(matches!(refused, Err(DockError::TakenId(_))), "{refused:?}");

    assert!(!marker.exists(), "the refused program was started");
    assert!(other.events().is_empty());
}

#[test]
fn a_program_that_cannot_be_started_is_a_start_error_and_leaves_nothing_behind() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    let script = scratch.script("fine", "echo ready\n");

    let id = TerminalId::random().expect("an id");
    let missing = Start::program(
        scratch.path().join("no-such-program"),
        Vec::new(),
        scratch.path().to_owned(),
        SIZE,
    );
    let (_, events) = Watch::new();
    let refused = dock.open(id.clone(), missing, events);
    assert!(
        matches!(refused, Err(DockError::Start { .. })),
        "{refused:?}"
    );

    let (_, events) = Watch::new();
    let elsewhere = Start::program(
        script.clone(),
        Vec::new(),
        scratch.path().join("no-such-folder"),
        SIZE,
    );
    let refused = dock.open(id.clone(), elsewhere, events);
    assert!(
        matches!(refused, Err(DockError::Start { .. })),
        "{refused:?}"
    );
    assert_eq!(dock.running(), 0);

    // The id of a start that failed was not used up.
    let (watch, events) = Watch::new();
    dock.open(id, scratch.start(script), events)
        .expect("open with the same id");
    watch.wait_for_line("ready");
}

#[test]
fn what_the_dock_is_asked_of_a_terminal_it_does_not_hold_is_refused() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    let unknown = TerminalId::random().expect("an id");
    assert!(matches!(
        dock.write(&unknown, b"x"),
        Err(DockError::UnknownTerminal(id)) if id == unknown
    ));
    assert!(matches!(
        dock.resize(&unknown, SIZE),
        Err(DockError::UnknownTerminal(_))
    ));

    // A terminal whose program ended is not held either.
    let script = scratch.script("short", "echo done\n");
    let (id, watch) = open(&dock, scratch.start(script));
    watch.wait_for_end();
    assert!(matches!(
        dock.write(&id, b"x"),
        Err(DockError::UnknownTerminal(_))
    ));
    assert!(matches!(
        dock.resize(&id, SIZE),
        Err(DockError::UnknownTerminal(_))
    ));
}

#[test]
fn the_program_starts_at_the_size_given_and_a_resize_reaches_it() {
    let scratch = Scratch::new();
    let script = scratch.script(
        "size",
        "stty -echo\nstty size\nwhile read line; do stty size; done\n",
    );
    let dock = Dock::new();
    let mut start = scratch.start(script);
    start.size = Size {
        columns: 100,
        rows: 30,
    };
    let (id, watch) = open(&dock, start);
    watch.wait_for_line("30 100");

    dock.resize(
        &id,
        Size {
            columns: 120,
            rows: 40,
        },
    )
    .expect("resize");
    type_line(&dock, &id, "again");
    watch.wait_for_line("40 120");

    // A size under 1 is taken as 1.
    dock.resize(
        &id,
        Size {
            columns: 0,
            rows: 0,
        },
    )
    .expect("resize");
    type_line(&dock, &id, "again");
    watch.wait_for_line("1 1");
}

#[test]
fn a_program_that_reads_nothing_does_not_hold_up_the_caller() {
    let scratch = Scratch::new();
    let script = scratch.script("deaf", "echo ready\nsleep 300\n");
    let dock = Dock::new();
    let (id, watch) = open(&dock, scratch.start(script));
    watch.wait_for_line("ready");

    // 20 MiB is more than the queue holds and far more than the terminal does.
    let megabyte = vec![b'x'; 1024 * 1024];
    let writing = Instant::now();
    for _ in 0..20 {
        dock.write(&id, &megabyte).expect("write");
    }
    assert!(
        writing.elapsed() < Duration::from_secs(5),
        "the calls waited {:?}",
        writing.elapsed()
    );

    // And it can still be stopped.
    let stopping = Instant::now();
    drop(dock);
    assert!(
        stopping.elapsed() < STOP_STEP,
        "took {:?}",
        stopping.elapsed()
    );
    assert!(matches!(watch.events().last(), Some(Event::Ended(_))));
}

#[test]
fn terminals_fed_more_than_their_programs_read_are_still_stopped_by_the_hang_up_alone() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    // Programs that read nothing, with the terminal's echo on: what is typed to them is
    // echoed back, so a Dock that keeps typing into a terminal whose programs are dying
    // keeps the terminal busy and the programs from leaving.
    let mut terminals = Vec::new();
    for n in 0..4 {
        let script = scratch.script(&format!("deaf{n}"), "echo ready\nsleep 300\n");
        let (id, watch) = open(&dock, scratch.start(script));
        watch.wait_for_line("ready");
        terminals.push((id, watch));
    }
    let megabyte = vec![b'x'; 1024 * 1024];
    for (id, _) in &terminals {
        for _ in 0..16 {
            dock.write(id, &megabyte).expect("write");
        }
    }

    let stopping = Instant::now();
    dock.stop_all();
    assert!(
        stopping.elapsed() < STOP_STEP,
        "took {:?}",
        stopping.elapsed()
    );
    // Every one took the hang-up: none needed the terminate or the kill signal.
    for (_, watch) in &terminals {
        assert_eq!(
            watch.events().last(),
            Some(&Event::Ended(Ending::Signalled(libc::SIGHUP)))
        );
    }
}

#[test]
fn what_is_typed_in_many_writes_and_one_big_one_reaches_the_program_whole_and_in_order() {
    let scratch = Scratch::new();
    let stored = scratch.path().join("stored");
    const SIZE_OF_TYPING: usize = 300_000;
    let script = scratch.script(
        "store",
        &format!(
            "stty raw -echo\necho ready\nhead -c {SIZE_OF_TYPING} > '{}'\necho stored\n",
            stored.display()
        ),
    );
    let dock = Dock::new();
    let (id, watch) = open(&dock, scratch.start(script));
    watch.wait_for_line("ready");

    // More than the terminal holds, so that the writes wait for the program.
    let typed: Vec<u8> = (0..SIZE_OF_TYPING).map(|n| (n % 251) as u8).collect();
    dock.write(&id, &typed[..10]).expect("write");
    dock.write(&id, &typed[10..20]).expect("write");
    dock.write(&id, &typed[20..]).expect("write");
    watch.wait_for_line("stored");

    assert_eq!(std::fs::read(&stored).expect("the stored bytes"), typed);
}

// The reader goes on to the end of the terminal when the program has been collected, and
// does not stop at the hang-up the master reports. On macOS no test can fail on that: a
// program that exits waits, in closing its terminal, until the master has read what it
// wrote, and what is still unread when it gives up is thrown away, so nothing is left to
// read once it is collected. This test shows only that a burst of output is read whole and
// told before the end; the next one shows the guard on a system that keeps the output.
#[test]
fn output_written_just_before_the_program_exits_is_not_lost() {
    let scratch = Scratch::new();
    let script = scratch.script(
        "burst",
        "i=1\nwhile [ $i -le 3000 ]; do echo \"line $i\"; i=$((i + 1)); done\n",
    );
    let dock = Dock::new();
    let (_, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_for_end();

    let expected: Vec<String> = (1..=3000).map(|n| format!("line {n}")).collect();
    assert_eq!(lines(&events), expected);
    assert_eq!(events.last(), Some(&Event::Ended(Ending::Exited(0))));
}

// Where a terminal keeps what its program wrote after the program has gone (Linux), the
// last words of a program are still on the master when the program is collected. The
// consumer of the events is held in the first event until the program has been collected,
// so the reader cannot have read the rest before. It must still tell the rest, and before
// the end. A reader that stopped when told the program was collected, or at the hang-up
// the master reports, would lose it. On macOS the terminal throws the rest away, so only
// the end is shown there.
#[test]
fn what_a_program_wrote_after_the_reader_was_held_up_is_told_when_the_program_is_collected() {
    let scratch = Scratch::new();
    let marker = scratch.path().join("go");
    let script = scratch.script(
        "last-words",
        &format!(
            "echo \"pid $$\"\nwhile [ ! -e '{}' ]; do sleep 0.05; done\necho last\n",
            marker.display()
        ),
    );
    let (watch, mut told) = Watch::new();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let events: Events = {
        let release = Arc::clone(&release);
        let mut held = false;
        Box::new(move |event| {
            let is_output = matches!(event, Event::Output(_));
            told(event);
            if is_output && !std::mem::replace(&mut held, true) {
                let (open, opened) = &*release;
                let mut open = open.lock().expect("the gate");
                let deadline = Instant::now() + DEADLINE;
                while !*open {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return;
                    }
                    open = opened.wait_timeout(open, left).expect("the gate").0;
                }
            }
        })
    };
    let dock = Dock::new();
    let id = TerminalId::random().expect("an id");
    dock.open(id, scratch.start(script), events)
        .expect("open a terminal");

    let seen = watch.wait_until("the program's id", |events| {
        lines(events).iter().any(|line| line.starts_with("pid "))
    });
    let pid = number_after(&seen, "pid ");
    // The reader is held in that event. The program goes on, ends, and is collected.
    std::fs::write(&marker, "").expect("let the program go on");
    // macOS does not let a program end while its terminal's reader is held up, so there
    // the reader is let go at once. On Linux the reader is let go once the Dock has
    // collected the program, not when it only waits to be collected: the Dock tells the
    // reader that the program was collected right after it collected it.
    if cfg!(target_os = "linux") {
        let deadline = Instant::now() + DEADLINE;
        while !is_collected(pid) {
            assert!(Instant::now() < deadline, "the program did not end");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    {
        let (open, opened) = &*release;
        *open.lock().expect("the gate") = true;
        opened.notify_all();
    }

    let events = watch.wait_for_end();
    assert_eq!(events.last(), Some(&Event::Ended(Ending::Exited(0))));
    if cfg!(target_os = "linux") {
        assert_eq!(
            lines(&events),
            vec![format!("pid {pid}"), "last".to_owned()]
        );
    }
}

#[test]
fn a_process_the_program_left_behind_does_not_keep_the_end_from_being_told() {
    let scratch = Scratch::new();
    // The shell exits at once; its background job ignores the hang-up the kernel sends
    // to the terminal's foreground group when the shell is gone, and keeps the terminal
    // open.
    let script = scratch.script(
        "leaves",
        "trap '' HUP\nsleep 300 &\necho \"left $!\"\necho done\n",
    );
    let dock = Dock::new();
    let (_, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_for_line("done");
    let left = Reap(number_after(&events, "left "));

    let events = watch.wait_for_end();
    assert_eq!(events.last(), Some(&Event::Ended(Ending::Exited(0))));
    assert_eq!(dock.running(), 0);
    // On Linux the terminal stays open for the job, and the end is told a second after
    // the program was collected, not at once. macOS takes the terminal from every
    // process when its session leader exits, so the end is not held up there.
    if cfg!(target_os = "linux") {
        let done = watch.when(|events| lines(events).iter().any(|line| line == "done"));
        let ended = watch.when(|events| matches!(events.last(), Some(Event::Ended(_))));
        assert!(
            ended - done >= Duration::from_millis(900),
            "the end came {:?} after the last output",
            ended - done
        );
    }
    assert!(!is_gone(left.0), "the job did not outlive the program");
    drop(left);
}

#[test]
fn a_process_left_behind_by_an_ended_program_is_still_stopped_with_the_dock() {
    let scratch = Scratch::new();
    // As in the test above: the job ignores the hang-up and outlives the program.
    let script = scratch.script(
        "leaves",
        "trap '' HUP\nsleep 300 &\necho \"left $!\"\necho done\n",
    );
    let dock = Dock::new();
    let (_, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_for_line("done");
    let left = Reap(number_after(&events, "left "));
    watch.wait_for_end();
    assert!(!is_gone(left.0), "the job did not outlive the program");

    // The terminal has ended, and what its program left behind is stopped all the same:
    // it ignores the hang-up and takes the terminate signal.
    dock.stop_all();
    assert!(is_gone(left.0), "the job is still there");
}

#[test]
fn no_terminal_is_opened_while_the_others_are_being_stopped() {
    let scratch = Scratch::new();
    // Says so when it is hung up, and then neither leaves nor takes the terminate signal:
    // the stop takes two steps and is under way for as long as that.
    let stubborn = scratch.script(
        "stubborn",
        "trap 'echo hup' HUP\ntrap '' TERM\necho ready\nwhile :; do sleep 1; done\n",
    );
    let marker = scratch.path().join("started");
    let late = scratch.script("late", &format!("touch '{}'\nsleep 3\n", marker.display()));
    let dock = Arc::new(Dock::new());
    let (_, watch) = open(&dock, scratch.start(stubborn));
    watch.wait_for_line("ready");

    let stopping = {
        let dock = Arc::clone(&dock);
        std::thread::spawn(move || dock.stop_all())
    };
    // The hang-up has gone out: the stop is under way.
    watch.wait_for_line("hup");
    let (_, events) = Watch::new();
    let refused = dock.open(
        TerminalId::random().expect("an id"),
        scratch.start(late),
        events,
    );
    assert!(
        matches!(refused, Err(DockError::Start { .. })),
        "{refused:?}"
    );
    stopping.join().expect("the stop ends");
    assert_eq!(dock.running(), 0);
    assert!(!marker.exists(), "the refused program was started");
}

#[test]
fn the_users_shell_is_started_as_a_login_shell_in_the_folder_it_is_given() {
    let scratch = Scratch::new();
    let shell = scratch.script("shell", "echo \"args $*\"\npwd -P\n");
    // SAFETY: changing the environment is sound only while no other thread reads it, as
    // a thread that starts a process does. No thread of this test exists yet, and
    // `cargo nextest` gives each test a process of its own. The shell is this test's own:
    // nothing of the user's environment is read.
    unsafe { std::env::set_var("SHELL", &shell) };
    let dock = Dock::new();
    let (_, watch) = open(&dock, Start::shell(scratch.path().to_owned(), SIZE));
    let events = watch.wait_for_end();

    assert_eq!(
        lines(&events),
        ["args -l".to_owned(), scratch.path().display().to_string()]
    );
}

#[test]
fn a_process_left_behind_that_keeps_writing_does_not_keep_the_end_from_being_told() {
    let scratch = Scratch::new();
    // The shell exits at once; its background job ignores the hang-up and writes to the
    // terminal for as long as it can, so that the terminal never runs dry. macOS takes the
    // terminal from the job when the shell exits, so the end is not held up there and this
    // test cannot fail on it; where the job keeps the terminal, the Dock must read only a
    // bounded amount once it lets the terminal go.
    //
    // The shell and the job write to one terminal, and a terminal that is full holds a
    // write up half way, so that a line of one can come in the middle of a line of the
    // other. The job's number therefore goes to a file, not to the terminal.
    let pid_file = scratch.path().join("job");
    let script = scratch.script(
        "chatters",
        &format!(
            "trap '' HUP\n(while :; do echo chatter; done) &\necho $! > '{}'\n",
            pid_file.display()
        ),
    );
    let dock = Dock::new();
    let (_, watch) = open(&dock, scratch.start(script));
    // Until the guard exists, the Dock's own drop stops the job if the test fails.
    let left = Reap(pid_in_file(&pid_file));

    let events = watch.wait_for_end();
    assert_eq!(events.last(), Some(&Event::Ended(Ending::Exited(0))));
    assert_eq!(dock.running(), 0);
    drop(left);
}

#[test]
fn a_program_holds_none_of_the_descriptors_of_the_dock() {
    let scratch = Scratch::new();
    // The descriptors above 2 that a shell running this script holds, one per line. The
    // shell keeps one or two of its own, which a plain start shows.
    let script = scratch.script(
        "descriptors",
        "n=3\n\
         while [ $n -le 40 ]; do\n\
         \x20 if ( true >&$n ) 2>/dev/null; then echo \"open $n\"; fi\n\
         \x20 n=$((n + 1))\n\
         done\n\
         echo checked\n",
    );
    let plain = Command::new(&script)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("run the script");
    let alone: Vec<String> = String::from_utf8_lossy(&plain.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        alone.last().map(String::as_str),
        Some("checked"),
        "{alone:?}"
    );

    // Another terminal is open, so that its master is in the Keeper to be inherited.
    let dock = Dock::new();
    let waiting = scratch.script("waiting", "echo ready\nread line\n");
    let (_, other) = open(&dock, scratch.start(waiting));
    other.wait_for_line("ready");
    let (_, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_for_line("checked");
    assert_eq!(lines(&events), alone);
}

/// The variables of an editor's connection or of another editor's terminal that a Keeper
/// started from an editor would hold, and that no shell of the app may get.
const EDITOR_VARIABLES: [&str; 6] = [
    "CLAUDE_CODE_SSE_PORT",
    "ENABLE_IDE_INTEGRATION",
    "VSCODE_INJECTION",
    "VSCODE_IPC_HOOK_CLI",
    "VSCODE_GIT_IPC_HANDLE",
    "ZED_TERM",
];

/// Whether the system takes the variable out of the environment of a program it runs
/// itself: macOS drops those of the dynamic loader before `env` starts. Elsewhere nothing
/// is dropped, so a variable of that kind that the app added would show.
fn dropped_by_the_system(name: &str) -> bool {
    cfg!(target_os = "macos") && name.starts_with("DYLD_")
}

#[test]
fn the_environment_of_a_shell_of_the_app_differs_from_the_users_only_by_what_the_app_adds() {
    // SAFETY: changing the environment is sound only while no other thread reads it, as
    // a thread that starts a process does. This is the first statement of a test, and
    // `cargo nextest` gives each test a process of its own: no other thread exists yet.
    unsafe {
        // The user's own: an editor's connection, other terminals, and a plugin folder of
        // their own that the app's has to come after.
        for name in LEFT_OUT_VARIABLES.into_iter().chain(EDITOR_VARIABLES) {
            std::env::set_var(name, "editor-connection-4242");
        }
        std::env::set_var("CLAUDE_CODE_PLUGIN_DIRS", "/user/own");
        std::env::set_var("PATH", "/usr/bin:/bin");
        // The terminal's own variables, each different from what the app sets.
        std::env::set_var("TERM", "dumb");
        std::env::set_var("COLORTERM", "16");
        std::env::set_var("TERM_PROGRAM", "Apple_Terminal");
        std::env::set_var("TERM_PROGRAM_VERSION", "0.0");
        std::env::set_var("YAAR_DOCK_TEST_PASSED_ON", "yes");
    }
    let scratch = Scratch::new();
    let plugin = scratch.path().join("plugin");
    let reporter = scratch.path().join("pieces/yaar-pi-report");
    // What the user's own environment is, less what a system program drops by itself:
    // macOS removes the variables of the dynamic loader before it runs `env`.
    let users: BTreeMap<String, String> = std::env::vars()
        .filter(|(name, _)| !dropped_by_the_system(name))
        .collect();

    // `env` itself is the program, so that no shell adds `SHLVL`, `PWD` or `_`.
    let dock = Dock::new();
    let start = scratch.start(PathBuf::from("/usr/bin/env")).with(&[
        Entry::Variable {
            name: "YAAR_SOCKET".into(),
            value: "/run/yaar/keeper.sock".into(),
        },
        Entry::Variable {
            name: "YAAR_TERMINAL".into(),
            value: "00112233445566778899aabbccddeeff".into(),
        },
        Entry::PathList {
            name: "CLAUDE_CODE_PLUGIN_DIRS".into(),
            path: plugin.clone(),
        },
        Entry::Variable {
            name: "YAAR_PI_REPORT".into(),
            value: reporter.clone().into_os_string(),
        },
    ]);
    let (_, watch) = open(&dock, start);
    let events = watch.wait_for_end();

    let theirs: BTreeMap<String, String> = lines(&events)
        .iter()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .filter(|(name, _)| !dropped_by_the_system(name))
        .collect();

    // Where their environment differs from the user's, name by name: what the user had
    // and what the program got. One entry for each clause of the sentence, and nothing
    // else may be there.
    let connection = Some("editor-connection-4242".to_owned());
    let mut differences = BTreeMap::new();
    for (name, value) in [
        (
            "CLAUDE_CODE_PLUGIN_DIRS",
            format!("/user/own:{}", plugin.display()),
        ),
        ("YAAR_PI_REPORT", reporter.display().to_string()),
        ("YAAR_SOCKET", "/run/yaar/keeper.sock".to_owned()),
        (
            "YAAR_TERMINAL",
            "00112233445566778899aabbccddeeff".to_owned(),
        ),
        ("TERM", "xterm-256color".to_owned()),
        ("COLORTERM", "truecolor".to_owned()),
        ("TERM_PROGRAM", "yaar".to_owned()),
        ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION").to_owned()),
    ] {
        differences.insert(name.to_owned(), (users.get(name).cloned(), Some(value)));
    }
    // The variables of other terminals and multiplexers, and of an editor's connection,
    // are left out.
    for name in LEFT_OUT_VARIABLES.into_iter().chain(EDITOR_VARIABLES) {
        differences.insert(name.to_owned(), (connection.clone(), None));
    }
    let mut found = BTreeMap::new();
    for name in users.keys().chain(theirs.keys()) {
        let (before, after) = (users.get(name), theirs.get(name));
        if before != after {
            found.insert(name.clone(), (before.cloned(), after.cloned()));
        }
    }
    assert_eq!(found, differences);

    // Nothing in it names an editor connection, by name or by value.
    for (name, value) in &theirs {
        assert!(!EDITOR_VARIABLES.contains(&name.as_str()), "{name} is set");
        assert_ne!(
            Some(value),
            connection.as_ref(),
            "{name} holds the connection"
        );
    }
}

/// Everything the program wrote, as the bytes the events held.
fn output(events: &[Event]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for event in events {
        if let Event::Output(output) = event {
            bytes.extend_from_slice(output);
        }
    }
    bytes
}

/// How many bytes the program wrote, as the events told so far hold them.
fn output_len(told: &[(Instant, Event)]) -> usize {
    told.iter()
        .map(|(_, event)| match event {
            Event::Output(bytes) => bytes.len(),
            _ => 0,
        })
        .sum()
}

/// Whether what the program wrote ends with `suffix`, looking at the last events only.
fn output_ends_with(told: &[(Instant, Event)], suffix: &[u8]) -> bool {
    let mut tail: Vec<u8> = Vec::new();
    for (_, event) in told.iter().rev() {
        if let Event::Output(bytes) = event {
            tail.splice(0..0, bytes.iter().copied());
            if tail.len() >= suffix.len() {
                break;
            }
        }
    }
    tail.ends_with(suffix)
}

/// What `attach` handed over, and how many events the terminal had told when it did.
struct Attached {
    ring: Vec<u8>,
    cut: bool,
    told: usize,
}

/// Attaches to the terminal and notes, inside the call, how many events `watch` holds: no
/// event of the terminal may be told while `attach` runs, so the count is the boundary
/// between what the ring holds and what is told after it.
fn attach(dock: &Dock, terminal: &TerminalId, watch: &Watch) -> Result<Attached, DockError> {
    let mut attached = None;
    dock.attach(terminal, |ring, cut| {
        attached = Some(Attached {
            ring: ring.to_vec(),
            cut,
            told: watch.count(),
        });
    })?;
    Ok(attached.expect("attach hands the ring over"))
}

#[test]
fn a_terminal_that_wrote_less_than_the_ring_holds_is_attached_with_all_of_it_and_not_cut() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    let script = scratch.script("short", "stty -echo\necho hello\necho world\nread line\n");
    let (id, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_for_line("world");

    let attached = attach(&dock, &id, &watch).expect("attach");
    assert!(!attached.cut);
    assert_eq!(attached.ring, output(&events));
    assert_eq!(
        String::from_utf8(attached.ring).expect("text"),
        "hello\r\nworld\r\n"
    );
}

#[test]
fn a_terminal_that_wrote_more_than_the_ring_holds_is_attached_with_the_newest_bytes_and_cut() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    // About 2 MiB of numbered lines, then a line that says it is over.
    let script = scratch.script("long", "stty -echo\nseq 1 300000\necho over\nread line\n");
    let (id, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_told("the last line", |told| output_ends_with(told, b"over\r\n"));
    let written = output(&events);
    assert!(written.len() > 2 * RING_BYTES, "{} bytes", written.len());

    let attached = attach(&dock, &id, &watch).expect("attach");
    assert!(attached.cut);
    assert_eq!(attached.ring.len(), RING_BYTES);
    assert_eq!(attached.ring, written[written.len() - RING_BYTES..]);
}

#[test]
fn the_ring_is_cut_by_the_byte_after_it_is_full_and_not_before() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    // Exactly `RING_BYTES` bytes, then one more when a line is typed. No newline in
    // either, so the terminal changes none of them.
    let script = scratch.script(
        "boundary",
        "stty -echo\n\
         head -c 1048576 /dev/zero | tr '\\000' a\n\
         read line\n\
         printf b\n\
         read line\n",
    );
    let (id, watch) = open(&dock, scratch.start(script));
    watch.wait_told("a full ring", |told| output_len(told) >= RING_BYTES);

    let full = attach(&dock, &id, &watch).expect("attach");
    assert_eq!(full.ring.len(), RING_BYTES);
    assert!(!full.cut, "nothing has been dropped yet");

    type_line(&dock, &id, "go");
    watch.wait_told("one byte more", |told| output_len(told) > RING_BYTES);
    let cut = attach(&dock, &id, &watch).expect("attach");
    assert!(cut.cut);
    assert_eq!(cut.ring.len(), RING_BYTES);
    assert_eq!(cut.ring.last(), Some(&b'b'));
    assert!(cut.ring[..RING_BYTES - 1].iter().all(|byte| *byte == b'a'));
}

#[test]
fn output_written_while_attaching_is_told_after_the_ring_and_none_is_lost_or_told_twice() {
    const LAST: u32 = 20_000;
    let scratch = Scratch::new();
    let dock = Dock::new();
    // Many small writes, so that reads and attaches meet at every kind of moment; they
    // begin when a line is typed, which is when the attaching begins.
    let script = scratch.script(
        "numbers",
        &format!(
            "stty -echo\n\
             echo ready\n\
             read line\n\
             i=1\n\
             while [ $i -le {LAST} ]; do echo $i; i=$((i + 1)); done\n\
             read line\n"
        ),
    );
    let (id, watch) = open(&dock, scratch.start(script));
    // The program has turned the echo off: the line typed is not part of its output.
    watch.wait_for_line("ready");
    type_line(&dock, &id, "go");

    // Attach again and again while the program runs; each time the ring must be exactly
    // what was told before it, in a terminal that wrote less than the ring holds.
    let mut attaches = 0;
    let mut last = None;
    let deadline = Instant::now() + DEADLINE;
    while let Ok(attached) = attach(&dock, &id, &watch) {
        assert!(
            Instant::now() < deadline,
            "the program did not write up to {LAST} within {DEADLINE:?}; the last ring held {} bytes",
            attached.ring.len()
        );
        assert!(!attached.cut);
        assert_eq!(
            attached.ring,
            watch.output_of_first(attached.told),
            "attach number {attaches}"
        );
        attaches += 1;
        let done = attached.ring.ends_with(format!("{LAST}\r\n").as_bytes());
        last = Some(attached);
        if done {
            break;
        }
    }
    assert!(attaches > 1, "the program ended before a second attach");

    // The last ring, then what was told after it, is every number once and in order.
    let last = last.expect("an attach");
    let events = watch.wait_for_line(&LAST.to_string());
    let mut whole = last.ring;
    whole.extend(output(&events[last.told..]));
    let text = String::from_utf8(whole).expect("text");
    let numbers: Vec<u32> = text
        .strip_prefix("ready\r\n")
        .expect("what the program said first")
        .lines()
        .map(|line| line.trim().parse().expect("a number"))
        .collect();
    assert_eq!(numbers, (1..=LAST).collect::<Vec<_>>());
}

#[test]
fn attaching_to_a_terminal_that_is_not_held_is_refused_and_hands_nothing_over() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    let refused = |dock: &Dock, id: &TerminalId| {
        let mut called = false;
        let answer = dock.attach(id, |_, _| called = true);
        assert!(
            matches!(answer, Err(DockError::UnknownTerminal(_))),
            "{answer:?}"
        );
        assert!(!called);
        assert!(matches!(
            dock.redraw(id),
            Err(DockError::UnknownTerminal(_))
        ));
    };
    refused(&dock, &TerminalId::random().expect("an id"));

    let (id, watch) = open(&dock, scratch.start(scratch.script("ends", "echo bye\n")));
    watch.wait_for_end();
    refused(&dock, &id);
}

#[test]
fn a_mark_that_comes_in_two_writes_is_told_once_after_the_output_that_completed_it() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    // The first half, then nothing until a line is typed, then the second half.
    let script = scratch.script(
        "split",
        "stty -echo\n\
         printf 'before \\033]9;4'\n\
         read line\n\
         printf ';0;\\007'\n\
         read line\n\
         echo done\n\
         read line\n",
    );
    let mut start = scratch.start(script);
    start.marks = vec![b"\x1b]9;4;0;\x07".to_vec(), b"no such mark".to_vec()];
    let (id, watch) = open(&dock, start);
    watch.wait_until("the first half", |events| {
        output(events).ends_with(b"\x1b]9;4")
    });
    // Anything told before the first half is told after it as well, in order: typing a
    // line makes the program write again, which is the barrier for what came before.
    type_line(&dock, &id, "go");
    let events = watch.wait_until("the mark", |events| events.contains(&Event::Mark(0)));
    let at = events
        .iter()
        .position(|event| *event == Event::Mark(0))
        .expect("the mark");
    // Told after the output that held its last byte, and not before the output that
    // completed it.
    let before = output(&events[..at]);
    assert!(before.ends_with(b"\x1b]9;4;0;\x07"), "{before:?}");
    // Nothing is told twice, and no other mark is told: the later events show it.
    type_line(&dock, &id, "again");
    let events = watch.wait_until("the end of the program's output", |events| {
        output(events).ends_with(b"done\r\n")
    });
    let marks: Vec<&Event> = events
        .iter()
        .filter(|event| matches!(event, Event::Mark(_)))
        .collect();
    assert_eq!(marks, [&Event::Mark(0)]);
}

#[test]
fn a_program_asked_to_redraw_gets_the_signal_of_a_changed_size_also_when_the_size_is_the_same() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    // `wait` returns at once when a trapped signal comes, which a read would not.
    let script = scratch.script(
        "redraws",
        "trap 'echo winch' WINCH\n\
         echo ready\n\
         while :; do sleep 100 & wait $!; done\n",
    );
    let (id, watch) = open(&dock, scratch.start(script));
    watch.wait_for_line("ready");
    let winches = |events: &[Event]| lines(events).iter().filter(|line| *line == "winch").count();

    dock.redraw(&id).expect("redraw");
    watch.wait_until("a winch", |events| winches(events) == 1);
    dock.resize(&id, SIZE).expect("resize to the size it has");
    dock.redraw(&id).expect("redraw again");
    let events = watch.wait_until("a second winch", |events| winches(events) >= 2);
    // Only the ones asked for: the resize to the size it had changed nothing.
    assert_eq!(winches(&events), 2, "{:?}", lines(&events));
}

#[test]
fn a_redraw_reaches_the_foreground_process_group_and_not_the_shell_or_a_job_in_the_background() {
    let scratch = Scratch::new();
    let dock = Dock::new();
    // With job control each job has a process group of its own, and the one in front of
    // the terminal is the foreground job's, not the shell's that started the program:
    // what a command typed into a shell of the app is. The shell waits for that job and
    // would say so when it gets the signal, which it runs once the job has ended; the job
    // in the background says so at once. Each job says its process number, and the
    // signals the test sends them are what it waits on, since none of them can be told
    // from the lack of a line.
    let background = scratch.script(
        "background",
        "trap 'echo behind' WINCH\n\
         trap 'echo usr1' USR1\n\
         echo bg $$\n\
         while :; do sleep 100 & wait $!; done\n",
    );
    // The shell hands the terminal to a job after it has started it: the job says its
    // number when it is in front, which is when its process group is the terminal's. It
    // then waits in a loop of short sleeps in the foreground, not in `wait` on a sleep in
    // the background: a shell that gets a signal in the moment before `wait` begins runs
    // its trap only when the sleep ends, a hundred seconds later, while after each short
    // sleep it runs any trap that is pending.
    let foreground = scratch.script(
        "foreground",
        "trap 'echo front' WINCH\n\
         trap 'exit 0' USR2\n\
         while :; do\n\
           if [ -r /proc/$$/stat ]; then\n\
             set -- $(sed 's/.*) //' /proc/$$/stat); group=$3; front=$6\n\
           else\n\
             set -- $(ps -o pgid=,tpgid= -p $$); group=$1; front=$2\n\
           fi\n\
           [ \"$group\" = \"$front\" ] && break\n\
           sleep 0.05\n\
         done\n\
         echo fg $$\n\
         while :; do sleep 0.05; done\n",
    );
    let script = scratch.script(
        "groups",
        &format!(
            "set -m\n\
             trap 'echo shell' WINCH\n\
             {background} &\n\
             {foreground}\n\
             echo end\n\
             read line\n",
            background = background.display(),
            foreground = foreground.display(),
        ),
    );
    let (id, watch) = open(&dock, scratch.start(script));
    let events = watch.wait_until("both jobs", |events| {
        let seen = lines(events);
        ["bg ", "fg "]
            .iter()
            .all(|prefix| seen.iter().any(|line| line.starts_with(prefix)))
    });
    let (behind, front) = (number_after(&events, "bg "), number_after(&events, "fg "));

    dock.redraw(&id).expect("redraw");
    watch.wait_for_line("front");

    // The job in the background answers a signal sent after the redraw. A signal it had
    // got before the first answer is run by then, or in the pass that runs the first
    // answer, and the order of those two is the shell's: a signal sent after the first
    // answer is answered after both. A shell can lose a signal that comes while it runs a
    // trap, so a signal not answered for a while is sent again.
    let answers = |events: &[Event]| lines(events).iter().filter(|line| *line == "usr1").count();
    let deadline = Instant::now() + DEADLINE;
    for _ in 0..2 {
        let before = answers(&watch.events());
        while answers(&watch.events()) == before {
            assert!(
                Instant::now() < deadline,
                "the job in the background did not answer within {DEADLINE:?}: {:?}",
                lines(&watch.events())
            );
            // SAFETY: `kill` only sends a signal, to a process of this test's own terminal.
            unsafe { libc::kill(behind, libc::SIGUSR1) };
            let resend = Instant::now() + Duration::from_millis(500);
            while answers(&watch.events()) == before && Instant::now() < resend {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    // The shell tells what it got when the job in front ends, and then says it has.
    // SAFETY: as above.
    unsafe { libc::kill(front, libc::SIGUSR2) };
    let events = watch.wait_for_line("end");

    let seen = lines(&events);
    assert_eq!(
        seen.iter().filter(|line| *line == "front").count(),
        1,
        "{seen:?}"
    );
    for line in ["behind", "shell"] {
        assert!(
            !seen.iter().any(|seen| seen == line),
            "{line:?} got the signal: {seen:?}"
        );
    }
}

#[test]
fn a_launcher_runs_one_command_through_the_users_shell_and_the_terminal_ends_with_it() {
    let scratch = Scratch::new();
    // SAFETY: changing the environment is sound only while no other thread reads it, as
    // a thread that starts a process does. This is the first statement of a test, and
    // `cargo nextest` gives each test a process of its own. A login shell reads the
    // profile in its home: that is this test's folder, not the user's.
    unsafe {
        std::env::set_var("SHELL", "/bin/sh");
        std::env::set_var("HOME", scratch.path());
        std::env::remove_var("ENV");
    }
    let dock = Dock::new();
    // A pipe and a variable are the shell's, not the Dock's.
    let command = "echo \"in $(pwd -P)\" | tr a-z A-Z; exit 7";
    let (_, watch) = open(
        &dock,
        Start::launcher(scratch.path().to_owned(), SIZE, command),
    );
    let events = watch.wait_for_end();

    assert_eq!(
        watch.events().last(),
        Some(&Event::Ended(Ending::Exited(7)))
    );
    let expected = format!("IN {}", scratch.path().display().to_string().to_uppercase());
    assert!(
        lines(&events).contains(&expected),
        "{expected:?} not in {:?}",
        lines(&events)
    );
}
