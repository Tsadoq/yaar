//! System tests of what the app does with the files of the user's agent CLIs: it leaves
//! their settings as they are, and it never reads or stores what they sign in with.
//!
//! The home folder is the world's own. The settings and credential files of both CLIs are
//! planted in it before the Keeper starts, so that a Keeper that touched them would find
//! them there; a marker is planted in the credential files, and is looked for in
//! everything the app stored. On Linux the Keeper also runs under `strace`, which sees
//! every file it and the programs it started open.
//!
//! Each row runs for both CLIs: its body is a function of a [`Dialect`], run as Claude Code
//! by the test that carries the row's name and as pi by the one with `_as_pi` after it. pi's
//! settings are the one file of a CLI the app may change, by the one entry the user's yes
//! asks for: a world as pi gives that yes first, and the home is looked at again after it,
//! so that the turn is shown to change nothing.
//!
//! The reading of a trace is plain code that both systems compile; the tests of it run on
//! a trace typed by hand. Only starting the Keeper under the tracer, and the row that
//! does, run on Linux alone.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use yaar_link::{ConsentAnswer, SessionState, TerminalSize};
use yaar_system_tests::standin::{Did, Script};
use yaar_system_tests::{Client, DEADLINE, Dialect, World, descendants_of, is_alive, wait_until};

const SIZE: TerminalSize = TerminalSize {
    columns: 100,
    rows: 30,
};

/// The settings and credential files of the CLIs, relative to the home folder, in the
/// order they are planted: what they hold apart from the marker is of no meaning to the
/// app, which must not read them at all.
const SETTINGS: [(&str, &str); 3] = [
    (".claude/settings.json", "{\"theme\": \"dark\"}\n"),
    (".claude.json", "{\"numStartups\": 3}\n"),
    (
        ".pi/agent/settings.json",
        "{\"defaultProvider\": \"none\"}\n",
    ),
];

/// The credential files of the CLIs, relative to the home folder.
const CREDENTIALS: [&str; 2] = [".claude/.credentials.json", ".pi/agent/auth.json"];

/// What a file held when it was recorded: its bytes and when it was last changed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    bytes: Vec<u8>,
    modified: SystemTime,
}

/// Every regular file under `folder` but those under `apart`, each by its path relative
/// to `folder`, with what it holds. A link, a socket or a folder holds nothing of its own.
fn record(folder: &Path, apart: &Path) -> BTreeMap<PathBuf, Held> {
    let mut recorded = BTreeMap::new();
    walk(folder, &mut |path| {
        if path.starts_with(apart) {
            return false;
        }
        if let Ok(meta) = fs::symlink_metadata(path)
            && meta.is_file()
        {
            let held = Held {
                bytes: fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
                modified: meta.modified().expect("when the file was changed"),
            };
            recorded.insert(
                path.strip_prefix(folder).expect("under it").to_owned(),
                held,
            );
        }
        true
    });
    recorded
}

/// Calls `visit` with every path under `folder`, folders first; `visit` says whether to go
/// into a folder. Links are not followed.
fn walk(folder: &Path, visit: &mut dyn FnMut(&Path) -> bool) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let is_folder = fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
        if visit(&path) && is_folder {
            walk(&path, visit);
        }
    }
}

/// Whether `path`, relative to the home folder, is one of the places the CLIs keep their
/// own: `.claude`, `.claude.json` and `.pi`, and what lies under them.
fn is_kept_by_a_cli(path: &Path) -> bool {
    let first = path.components().next().expect("a path");
    [".claude", ".claude.json", ".pi"]
        .iter()
        .any(|own| first.as_os_str() == *own)
}

/// What differs between two recordings of a home folder: a file that was there and is
/// gone or has other bytes or another time of change, and a file that was not there under
/// the places the CLIs keep their own (`.claude`, `.claude.json`, `.pi`).
fn differences(before: &BTreeMap<PathBuf, Held>, after: &BTreeMap<PathBuf, Held>) -> Vec<String> {
    let mut found = Vec::new();
    for (path, held) in before {
        match after.get(path) {
            None => found.push(format!("{} is gone", path.display())),
            Some(now) if now.bytes != held.bytes => {
                found.push(format!("{} holds other bytes", path.display()));
            }
            Some(now) if now.modified != held.modified => {
                found.push(format!("{} was changed", path.display()));
            }
            Some(_) => {}
        }
    }
    for path in after.keys().filter(|path| !before.contains_key(*path)) {
        if is_kept_by_a_cli(path) {
            found.push(format!("{} was added", path.display()));
        }
    }
    found
}

/// Whether `haystack` holds `needle`.
fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// The files under `folder`, but those `except` names, whose bytes hold `marker`.
fn files_holding(marker: &[u8], folder: &Path, except: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(folder, &mut |path| {
        if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
            && !except(path)
            && fs::read(path).is_ok_and(|bytes| holds(&bytes, marker))
        {
            found.push(path.to_owned());
        }
        true
    });
    found
}

/// The git stores under `folder`: the folders that hold both `objects` and `refs`.
fn git_stores(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(folder, &mut |path| {
        if path.join("objects").is_dir() && path.join("refs").is_dir() {
            found.push(path.to_owned());
        }
        true
    });
    found
}

/// Whether any object of the git store `store`, its content as git gives it, holds
/// `marker`.
fn objects_hold(world: &World, store: &Path, marker: &[u8]) -> bool {
    let printed = world
        .command(Path::new("git"))
        .arg("--git-dir")
        .arg(store)
        .args(["cat-file", "--batch-all-objects", "--batch"])
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("run git on {}: {e}", store.display()));
    assert!(
        printed.status.success(),
        "git on {}: {}",
        store.display(),
        String::from_utf8_lossy(&printed.stderr)
    );
    holds(&printed.stdout, marker)
}

/// A marker no other run has: hexadecimal text of sixteen bytes the system gave.
fn marker() -> String {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .expect("random bytes");
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("marker-{hex}")
}

/// A world made ready for a test of the files of the CLIs.
struct Prepared {
    world: World,
    /// A repository with a file `notes.txt` in it.
    repo: yaar_system_tests::Repo,
    /// The marker the credential files hold.
    marker: String,
    /// Every file of the home folder but the app's own, as it was before the Keeper
    /// started.
    before: BTreeMap<PathBuf, Held>,
}

/// A world with the stand-in on its path, the settings and credential files of both CLIs
/// planted in its home folder, and the home recorded, before its Keeper starts; the
/// Keeper runs under the wrapper that `under` gives for the world, none for no wrapper.
fn world_with_files(under: impl FnOnce(&World) -> Vec<OsString>) -> Prepared {
    let mut world = World::new();
    world.agents_on_path();
    let marker = marker();
    for (file, content) in SETTINGS {
        plant(&world, file, content);
    }
    plant(
        &world,
        CREDENTIALS[0],
        &format!("{{\"claudeAiOauth\": {{\"accessToken\": \"{marker}\"}}}}\n"),
    );
    plant(
        &world,
        CREDENTIALS[1],
        &format!("{{\"anthropic\": {{\"key\": \"{marker}\"}}}}\n"),
    );
    world.agent_script(
        "default",
        &Script::parse(
            "turn\nread notes.txt\nedit notes.txt more\nwrite new.txt hello\n\
             shell echo x > shell.txt\nend\nturn\nexit\n",
        ),
    );
    let repo = world.repository("app");
    repo.git(&["config", "user.name", "A Test"]);
    repo.git(&["config", "user.email", "test@example.invalid"]);
    repo.write("notes.txt", "alpha\n");
    repo.commit("Add the file the agent reads");
    let before = record(&world.home(), &world.data_dir());
    let wrapper = under(&world);
    if wrapper.is_empty() {
        world.start_keeper();
    } else {
        world.start_keeper_under(&wrapper);
    }
    Prepared {
        world,
        repo,
        marker,
        before,
    }
}

/// Writes `content` in the file `relative` of the world's home, making its folders.
fn plant(world: &World, relative: &str, content: &str) {
    let file = world.home().join(relative);
    fs::create_dir_all(file.parent().expect("a folder")).expect("make the folder");
    fs::write(&file, content).expect("plant the file");
}

/// A full turn of the stand-in of `dialect` in a shell of the app, as AGENT-01 has it: a
/// read, an edit, a new file and a shell write, and then the session's exit, which the app
/// has seen when this returns. Gives the client.
fn full_turn(world: &World, repo: &yaar_system_tests::Repo, dialect: Dialect) -> Client {
    let mut client = world.client();
    let workspace = client.open_folder(repo.path()).expect("open the folder").1;
    let terminal = client.open_terminal(&workspace, SIZE).expect("a terminal");
    client.type_line(&terminal, dialect.name());
    let prompt = client.wait_for_prompt(&terminal, 0);
    client.type_line(&terminal, "make the changes");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::Unreviewed)
    });
    // Let the stand-in end with its own exit.
    client.wait_for_prompt(&terminal, prompt);
    client.type_line(&terminal, "");
    client.wait_for_agents(&workspace, DEADLINE, |agents| {
        agents.shown == Some(SessionState::NotRunning)
    });
    assert!(
        wait_until(DEADLINE, || world
            .agent_log("default")
            .did
            .iter()
            .any(|did| matches!(did, Did::Exited { .. }))),
        "the stand-in did not exit"
    );
    client
}

/// Asks the Keeper to stop everything, and waits until every process it started and it
/// itself are gone, so that what it does as it ends is done. Its children go first: the
/// world ends a tracer as soon as the Keeper's own process is gone.
fn stop_the_keeper(world: &mut World, client: &mut Client) {
    let keeper = world.keeper_pid().expect("the Keeper runs");
    let started = descendants_of(keeper);
    client.stop_everything();
    assert!(
        wait_until(DEADLINE, || started.iter().all(|pid| !is_alive(*pid))),
        "still there: {started:?}"
    );
    assert!(
        wait_until(DEADLINE, || world.keeper_pid().is_none()),
        "the Keeper did not end at the stop"
    );
}

// AGENT-12.
#[test]
fn the_settings_of_both_clis_are_unchanged_after_a_turn_and_a_marker_in_their_credentials_is_found_nowhere_the_app_stored()
 {
    settings_are_unchanged(Dialect::ClaudeCode);
}

// AGENT-12, as pi.
#[test]
fn the_settings_of_both_clis_are_unchanged_after_a_turn_and_a_marker_in_their_credentials_is_found_nowhere_the_app_stored_as_pi()
 {
    settings_are_unchanged(Dialect::Pi);
}

fn settings_are_unchanged(dialect: Dialect) {
    let Prepared {
        mut world,
        repo,
        marker,
        mut before,
    } = world_with_files(|_| Vec::new());
    let home = world.home();
    let data = world.data_dir();
    // The files are there to be looked at.
    assert_eq!(
        before.keys().filter(|path| is_kept_by_a_cli(path)).count(),
        SETTINGS.len() + CREDENTIALS.len()
    );

    if dialect == Dialect::Pi {
        // pi is installed here, so the Keeper asks its question, at the first request of a
        // Window, and asking writes nothing. The user's yes then changes pi's settings and
        // nothing else of the home (what it writes there is CONSENT-01's), and it is the
        // home after it that the turn must leave as it is: no yes is given by the turn.
        let asked = world.client().consents().expect("the list of questions");
        let pi = asked.iter().find(|consent| consent.agent == "pi");
        assert_eq!(
            pi.map(|consent| consent.answer),
            Some(ConsentAnswer::NotAsked),
            "the Keeper does not list pi's question as not yet answered: {asked:?}"
        );
        assert_eq!(
            differences(&before, &record(&home, &data)),
            Vec::<String>::new()
        );
        world.allow_pi_extension();
        let allowed = record(&home, &data);
        assert_eq!(
            differences(&before, &allowed),
            [".pi/agent/settings.json holds other bytes"]
        );
        before = allowed;
    }

    let mut client = full_turn(&world, &repo, dialect);
    // The Keeper ends before anything is looked at: what it does as it ends is part of
    // what the app did.
    stop_the_keeper(&mut world, &mut client);

    // The settings files of both CLIs are unchanged, and nothing was added beside them.
    let after = record(&home, &data);
    assert_eq!(differences(&before, &after), Vec::<String>::new());
    // The worktree holds nothing of either CLI.
    let status = repo.git(&["status", "--porcelain", "--ignored"]);
    for line in status.lines() {
        let path = line.get(3..).unwrap_or_default();
        let first = Path::new(path).components().next().expect("a path");
        assert!(
            first.as_os_str() != ".claude" && first.as_os_str() != ".pi",
            "the worktree holds {path} of a CLI: {status}"
        );
    }
    assert!(!repo.path().join(".claude").exists() && !repo.path().join(".pi").exists());

    // The marker is in no file of the world but the two planted and the test's own scripts
    // and logs, and in no object of any git store of the app.
    let root = home.parent().expect("the world's folder").to_owned();
    let planted: Vec<PathBuf> = CREDENTIALS.iter().map(|file| home.join(file)).collect();
    let own = |path: &Path| {
        planted.iter().any(|file| file == path)
            || (path.parent() == Some(root.as_path())
                && path
                    .extension()
                    .is_some_and(|extension| extension == "script" || extension == "log"))
    };
    assert_eq!(
        files_holding(marker.as_bytes(), &root, &own),
        Vec::<PathBuf>::new(),
        "the marker of the credential files is in what the app stored"
    );
    // The check reads the planted files: they do hold it.
    assert_eq!(
        files_holding(marker.as_bytes(), &root, &|path| !planted
            .contains(&path.to_owned())),
        planted
    );
    let stores = git_stores(&data);
    assert!(
        !stores.is_empty(),
        "the app kept no git store under {}",
        data.display()
    );
    for store in &stores {
        assert!(
            !objects_hold(&world, store, marker.as_bytes()),
            "the marker is in an object of {}",
            store.display()
        );
    }
}

// Not rows: what AGENT-12 looks with.
#[test]
fn a_recording_of_a_home_finds_a_changed_a_touched_a_removed_and_an_added_file_of_a_cli() {
    let world = World::new();
    let home = world.home();
    // A folder of the home that is left out, as the app's own is.
    let data = home.join("apart");
    for (file, content) in SETTINGS {
        plant(&world, file, content);
    }
    plant(&world, ".claude/projects/kept.json", "{}\n");
    plant(&world, ".pi/agent/removed.json", "{}\n");
    plant(&world, "apart/db", "one");
    let before = record(&home, &data);
    assert!(!before.contains_key(Path::new("apart/db")));

    fs::write(
        home.join(".claude/settings.json"),
        "{\"theme\": \"light\"}\n",
    )
    .unwrap();
    // The same bytes, written again at another time.
    fs::File::options()
        .write(true)
        .open(home.join(".claude.json"))
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    fs::remove_file(home.join(".pi/agent/removed.json")).unwrap();
    plant(&world, ".pi/agent/added.json", "{}\n");
    plant(&world, ".claude/settings.local.json", "{}\n");
    // Elsewhere in the home, and in the app's own folder, a new file is no concern.
    plant(&world, ".sh_history", "ls\n");
    fs::write(data.join("db"), "two").unwrap();

    assert_eq!(
        differences(&before, &record(&home, &data)),
        [
            ".claude/settings.json holds other bytes",
            ".claude.json was changed",
            ".pi/agent/removed.json is gone",
            ".claude/settings.local.json was added",
            ".pi/agent/added.json was added",
        ]
    );
}

#[test]
fn the_search_for_a_marker_finds_it_in_a_file_and_in_a_git_object_and_nowhere_else() {
    let world = World::new();
    let with = world.repository("with");
    let without = world.repository("without");
    let marker = marker();
    with.write("secret.txt", &marker);
    with.commit("Keep the secret");
    let root = with.path().parent().expect("the repositories").to_owned();
    let nothing = |_: &Path| false;

    // In the worktree, as a file; in the objects, git's own compression hides it from a
    // search of the files.
    assert_eq!(
        files_holding(marker.as_bytes(), &root, &nothing),
        [with.path().join("secret.txt")]
    );
    assert_eq!(
        files_holding(marker.as_bytes(), &root, &|path| path
            .ends_with("secret.txt")),
        Vec::<PathBuf>::new()
    );
    assert_eq!(
        git_stores(&root),
        [with.path().join(".git"), without.path().join(".git")]
    );
    assert!(objects_hold(
        &world,
        &with.path().join(".git"),
        marker.as_bytes()
    ));
    assert!(!objects_hold(
        &world,
        &without.path().join(".git"),
        marker.as_bytes()
    ));
    // Two markers are two.
    assert_ne!(marker, self::marker());
}

/// Whether a file of a CLI's credentials is named by `line` of a trace: by its name
/// `.credentials.json`, or `auth.json` with `.pi/agent` in the line, in the path it was
/// given or in the path `-y` gives a descriptor it was opened beside.
fn names_a_credential_file(line: &str) -> bool {
    line.contains(".credentials.json") || (line.contains("auth.json") && line.contains(".pi/agent"))
}

/// One call of a trace, with the lines of its unfinished and resumed halves joined.
#[derive(Debug, PartialEq, Eq)]
struct Call {
    /// The process that made it; 0 when the trace names none.
    pid: u32,
    /// The system call.
    name: String,
    /// Everything of the call after its name, the result included.
    rest: String,
}

/// The calls of a trace of `strace -f -y`, in the order they finished. The lines of a
/// call that was unfinished when another process's call came, and the line that resumed
/// it, are one call. Signals and exits are no calls.
fn calls(trace: &str) -> Vec<Call> {
    let mut unfinished: HashMap<u32, (String, String)> = HashMap::new();
    let mut found = Vec::new();
    for line in trace.lines() {
        let (pid, body) = split_pid(line);
        if let Some(resumed) = body.strip_prefix("<... ") {
            let Some((name, rest)) = resumed.split_once(" resumed>") else {
                continue;
            };
            // What the call began with, when the trace has it, and what it ended with.
            let (name, began) = match unfinished.remove(&pid) {
                Some((began_name, began)) if began_name == name => (name, began),
                _ => (name, format!("{name}(")),
            };
            found.push(Call {
                pid,
                name: name.to_owned(),
                rest: format!("{began}{rest}")[name.len()..].to_owned(),
            });
        } else if let Some(began) = body.strip_suffix("<unfinished ...>") {
            if let Some((name, _)) = began.split_once('(') {
                unfinished.insert(pid, (name.to_owned(), began.to_owned()));
            }
        } else if let Some((name, _)) = body.split_once('(')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.is_empty()
        {
            found.push(Call {
                pid,
                name: name.to_owned(),
                rest: body[name.len()..].to_owned(),
            });
        }
    }
    found
}

/// The process id a line of a trace starts with, as `1234 ` or `[pid  1234] `, and the
/// rest of the line; 0 for a line with none.
fn split_pid(line: &str) -> (u32, &str) {
    let line = line.trim_start();
    if let Some(inside) = line.strip_prefix("[pid")
        && let Some((number, rest)) = inside.split_once(']')
        && let Ok(pid) = number.trim().parse()
    {
        return (pid, rest.trim_start());
    }
    if let Some((number, rest)) = line.split_once(char::is_whitespace)
        && let Ok(pid) = number.parse()
    {
        return (pid, rest.trim_start());
    }
    (0, line)
}

/// The text of the first string a call's text holds, as strace writes it: in double
/// quotes, a quote or a backslash inside it escaped with a backslash.
fn first_string(text: &str) -> Option<String> {
    let mut chars = text.split_once('"')?.1.chars();
    let mut string = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(string),
            '\\' => string.push(chars.next()?),
            c => string.push(c),
        }
    }
    None
}

/// The program each process of a trace ran last, by its file name, from the calls of
/// `execve` that did not fail.
fn programs(trace: &str) -> HashMap<u32, String> {
    let mut run = HashMap::new();
    for call in calls(trace)
        .into_iter()
        .filter(|call| call.name == "execve")
    {
        let failed = call
            .rest
            .rsplit_once(" = ")
            .is_some_and(|(_, result)| result.starts_with('-'));
        if failed {
            continue;
        }
        if let Some(path) = first_string(&call.rest)
            && let Some(name) = path.rsplit('/').next()
        {
            run.insert(call.pid, name.to_owned());
        }
    }
    run
}

/// Every line of a trace that names a credential file of a CLI.
fn credential_lines(trace: &str) -> Vec<&str> {
    trace
        .lines()
        .filter(|line| names_a_credential_file(line))
        .collect()
}

/// Whether a call of `trace` that succeeded renamed a file over pi's settings: how the app
/// puts the entry of the user's yes into them, through a temporary file beside them. The
/// settings are named by the path the call was given, `.pi/agent/settings.json`, as the
/// app gives it, and as its last word: the temporary file has a longer name.
fn replaces_the_settings_of_pi(trace: &str) -> bool {
    calls(trace).iter().any(|call| {
        let Some((arguments, result)) = call.rest.rsplit_once(" = ") else {
            return false;
        };
        call.name.starts_with("rename")
            && result.starts_with('0')
            && strings(arguments)
                .last()
                .is_some_and(|destination| destination.ends_with("/.pi/agent/settings.json"))
    })
}

/// Every string of a call's text, as strace writes them: in double quotes, a quote or a
/// backslash inside one escaped with a backslash.
fn strings(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current: Option<String> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (&mut current, c) {
            (None, '"') => current = Some(String::new()),
            (Some(_), '"') => found.extend(current.take()),
            (Some(string), '\\') => string.extend(chars.next()),
            (Some(string), c) => string.push(c),
            (None, _) => {}
        }
    }
    found
}

#[test]
fn the_programs_of_a_trace_are_read_from_the_execve_calls_that_succeeded() {
    let trace = "\
4001  execve(\"/work/target/yaar-keeper\", [\"yaar-keeper\"], 0x7ffe1 /* 12 vars */) = 0
4001  openat(AT_FDCWD, \"/etc/ld.so.cache\", O_RDONLY|O_CLOEXEC) = 3</etc/ld.so.cache>
4002  execve(\"/bin/sh\", [\"sh\", \"-c\", \"x\"], 0x7ffe1 /* 12 vars */) = 0
4003  execve(\"/usr/local/bin/yaar-claude-hook\", [\"yaar-claude-hook\"], 0x7ffe1 /* 9 vars */ <unfinished ...>
4004  execve(\"/nowhere/yaar-claude-hook\", [\"yaar-claude-hook\"], 0x7ffe1 /* 9 vars */) = -1 ENOENT (No such file or directory)
4003  <... execve resumed>) = 0
4005  execve(\"/home/user/my \\\"quoted\\\" dir/standin-agent\", [\"claude\"], 0x7ffe1 /* 9 vars */) = 0
4002  --- SIGCHLD {si_signo=SIGCHLD, si_code=CLD_EXITED, si_pid=4005} ---
";
    let run = programs(trace);
    assert_eq!(run.get(&4001).map(String::as_str), Some("yaar-keeper"));
    assert_eq!(run.get(&4002).map(String::as_str), Some("sh"));
    // Resumed after another process's call: one call, and it succeeded.
    assert_eq!(run.get(&4003).map(String::as_str), Some("yaar-claude-hook"));
    // A failed call ran nothing.
    assert_eq!(run.get(&4004), None);
    assert_eq!(run.get(&4005).map(String::as_str), Some("standin-agent"));
    assert_eq!(run.len(), 4);
}

#[test]
fn a_trace_with_the_process_in_brackets_or_with_none_is_read_as_well() {
    let trace = "\
execve(\"/work/target/yaar-keeper\", [\"yaar-keeper\"], 0x7ffe1 /* 12 vars */) = 0
[pid  5002] execve(\"/bin/sh\", [\"sh\"], 0x7ffe1 /* 12 vars */) = 0
";
    let run = programs(trace);
    assert_eq!(run.get(&0).map(String::as_str), Some("yaar-keeper"));
    assert_eq!(run.get(&5002).map(String::as_str), Some("sh"));
}

#[test]
fn an_empty_trace_holds_no_program() {
    assert!(programs("").is_empty());
    assert!(credential_lines("").is_empty());
    assert!(!replaces_the_settings_of_pi(""));
}

#[test]
fn the_settings_of_pi_are_replaced_by_a_rename_that_succeeded_onto_their_path_and_by_nothing_else()
{
    // `rename` on x86-64, `renameat` on arm64; a call that was unfinished is joined.
    let renamed = "\
9001  rename(\"/h/.pi/agent/settings.json.tmp1\", \"/h/.pi/agent/settings.json\") = 0
";
    assert!(replaces_the_settings_of_pi(renamed));
    let at = "\
9001  renameat(AT_FDCWD, \"/h/.pi/agent/settings.json.tmp1\", AT_FDCWD, \"/h/.pi/agent/settings.json\") = 0
";
    assert!(replaces_the_settings_of_pi(at));
    let split = "\
9001  renameat2(AT_FDCWD, \"/h/.pi/agent/settings.json.tmp1\", AT_FDCWD, \"/h/.pi/agent/settings.json\", 0 <unfinished ...>
9002  getpid() = 9002
9001  <... renameat2 resumed>) = 0
";
    assert!(replaces_the_settings_of_pi(split));

    // A rename that failed, one onto another file, a look at the settings and the lock
    // beside them, and a rename onto the temporary name alone, replace nothing.
    for trace in [
        "9001  rename(\"/h/.pi/agent/settings.json.tmp1\", \"/h/.pi/agent/settings.json\") = -1 EACCES (Permission denied)\n",
        "9001  rename(\"/h/.pi/agent/a.tmp\", \"/h/.pi/agent/auth.json\") = 0\n",
        "9001  rename(\"/h/other/settings.json.tmp\", \"/h/other/settings.json\") = 0\n",
        "9001  openat(AT_FDCWD, \"/h/.pi/agent/settings.json\", O_RDONLY) = 3</h/.pi/agent/settings.json>\n",
        "9001  mkdir(\"/h/.pi/agent/settings.json.lock\", 0777) = 0\n",
        "9001  rename(\"/h/x\", \"/h/.pi/agent/settings.json.tmp1\") = 0\n",
        // The settings moved away are not replaced: they are the source, not the destination.
        "9001  rename(\"/h/.pi/agent/settings.json\", \"/h/.pi/agent/settings.json.bak\") = 0\n",
        "9001  renameat2(AT_FDCWD, \"/h/.pi/agent/settings.json\", AT_FDCWD, \"/h/.pi/agent/old.json\", RENAME_NOREPLACE) = 0\n",
    ] {
        assert!(!replaces_the_settings_of_pi(trace), "{trace}");
    }
}

#[test]
fn a_call_that_was_unfinished_is_joined_with_the_line_that_resumed_it() {
    let trace = "\
6001  openat(AT_FDCWD, \"/home/u/.claude/.credentials.json\", O_RDONLY <unfinished ...>
6002  read(3</tmp/x>, \"abc\", 3) = 3
6001  <... openat resumed>) = 5</home/u/.claude/.credentials.json>
";
    let joined = calls(trace);
    assert_eq!(joined.len(), 2);
    assert_eq!(joined[0].name, "read");
    assert_eq!(joined[1].pid, 6001);
    assert_eq!(joined[1].name, "openat");
    assert_eq!(
        joined[1].rest,
        "(AT_FDCWD, \"/home/u/.claude/.credentials.json\", O_RDONLY ) = 5</home/u/.claude/.credentials.json>"
    );
}

#[test]
fn a_credential_file_is_named_by_its_path_or_by_the_path_of_the_descriptor_it_was_opened_beside() {
    let trace = "\
7001  openat(AT_FDCWD, \"/home/u/.claude/.credentials.json\", O_RDONLY) = 3</home/u/.claude/.credentials.json>
7001  openat(AT_FDCWD, \"/home/u/.pi/agent/auth.json\", O_RDONLY) = 4</home/u/.pi/agent/auth.json>
7002  openat(5</home/u/.pi/agent>, \"auth.json\", O_RDONLY) = 6</home/u/.pi/agent/auth.json>
7002  openat(5</home/u/.pi/agent>, \"auth.json\", O_RDONLY) = -1 ENOENT (No such file or directory)
7003  openat(AT_FDCWD, \"/home/u/.claude/.credentials.json\", O_RDONLY <unfinished ...>
7003  <... openat resumed>) = -1 ENOENT (No such file or directory)
";
    // Five lines name one; the line that resumed the last call names none, and its first
    // half does.
    let found = credential_lines(trace);
    assert_eq!(found.len(), 5, "{found:#?}");
    assert!(found.iter().all(|line| !line.contains("resumed")));
}

#[test]
fn a_file_that_is_no_credential_of_a_cli_is_named_by_no_line_of_a_trace() {
    let trace = "\
8001  openat(AT_FDCWD, \"/home/u/.claude/settings.json\", O_RDONLY) = 3</home/u/.claude/settings.json>
8001  openat(AT_FDCWD, \"/home/u/.claude.json\", O_RDONLY) = 4</home/u/.claude.json>
8001  openat(AT_FDCWD, \"/home/u/.pi/agent/settings.json\", O_RDONLY) = 5</home/u/.pi/agent/settings.json>
8002  openat(AT_FDCWD, \"/work/project/auth.json\", O_RDONLY) = 6</work/project/auth.json>
8002  stat(\"/work/project/credentials.json\", {st_mode=S_IFREG|0644, st_size=2, ...}) = 0
8003  execve(\"/work/target/yaar-keeper\", [\"yaar-keeper\"], 0x7ffe1 /* 12 vars */) = 0
";
    assert_eq!(credential_lines(trace), Vec::<&str>::new());
}

/// What the Keeper of AGENT-13 runs under: `strace`, following every process it starts,
/// writing to `trace` the `execve` calls, which name each process's program, and every
/// call that takes a path, with the path of each descriptor. `%file` names the calls that
/// take a path on every architecture, as arm64 has no `open`.
#[cfg(target_os = "linux")]
fn strace_wrapper(trace: &Path) -> Vec<OsString> {
    [
        "strace",
        "-f",
        "-qq",
        "-y",
        "-e",
        "trace=execve,%file",
        "-o",
    ]
    .iter()
    .map(OsString::from)
    .chain([trace.as_os_str().to_owned()])
    .collect()
}

// AGENT-13.
#[cfg(target_os = "linux")]
#[test]
fn neither_the_keeper_nor_a_companion_opens_a_credential_file_of_a_cli_in_a_traced_turn() {
    nothing_opens_a_credential_file(Dialect::ClaudeCode);
}

// AGENT-13, as pi. Written on macOS, where it does not build: the Linux run is the first.
#[cfg(target_os = "linux")]
#[test]
fn neither_the_keeper_nor_a_companion_opens_a_credential_file_of_a_cli_in_a_traced_turn_as_pi() {
    nothing_opens_a_credential_file(Dialect::Pi);
}

/// The program of the app that reports for `dialect`'s CLI: Claude Code's hook program, and
/// for pi the reporting program its extension runs. The extension itself is TypeScript, run
/// by pi, and no process of the trace names it.
#[cfg(target_os = "linux")]
fn companion_program(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::ClaudeCode => "yaar-claude-hook",
        Dialect::Pi => "yaar-pi-report",
    }
}

#[cfg(target_os = "linux")]
fn nothing_opens_a_credential_file(dialect: Dialect) {
    let found = std::process::Command::new("strace").arg("-V").output();
    assert!(
        found.is_ok_and(|output| output.status.success()),
        "AGENT-13 needs `strace` on the search path: install it (`apt-get install strace`)"
    );
    // In the world's own folder, which the world removes.
    let trace_of = |world: &World| world.home().with_file_name("trace.txt");
    let Prepared {
        mut world, repo, ..
    } = world_with_files(|world| strace_wrapper(&trace_of(world)));
    if dialect == Dialect::Pi {
        // The user's yes, which makes the Keeper open pi's settings and write its one entry
        // there, so that the trace shows the app beside the file where the credentials lie.
        world.allow_pi_extension();
    }
    let mut client = full_turn(&world, &repo, dialect);

    // The Keeper ends of its own accord, and the tracer with it, so that everything it
    // saw is written down.
    stop_the_keeper(&mut world, &mut client);
    let trace = fs::read(trace_of(&world)).expect("the trace of the Keeper");
    let trace = String::from_utf8_lossy(&trace);

    // The trace holds the Keeper and a companion, so that an empty one cannot pass.
    let run = programs(&trace);
    for program in ["yaar-keeper", companion_program(dialect)] {
        assert!(
            run.values().any(|name| name == program),
            "the trace holds no process of {program}: {run:?}"
        );
    }
    // As pi it holds the Keeper's change of pi's settings too: the app did open the folder of
    // pi's own files, and the credentials beside them were left alone.
    if dialect == Dialect::Pi {
        assert!(
            replaces_the_settings_of_pi(&trace),
            "the trace holds no change of pi's settings"
        );
    }
    // None of them, nor any other process the Keeper started, names a credential file.
    let named = credential_lines(&trace);
    assert!(named.is_empty(), "{named:#?}");
    drop(client);
}
