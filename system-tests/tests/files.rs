//! System tests that the app leaves the agent CLIs' settings alone and never reads or stores their
//! credentials; on Linux the Keeper also runs under `strace`.

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

/// The CLIs' settings files, relative to the home folder, as planted: what they hold means
/// nothing to the app.
const SETTINGS: [(&str, &str); 3] = [
    (".claude/settings.json", "{\"theme\": \"dark\"}\n"),
    (".claude.json", "{\"numStartups\": 3}\n"),
    (
        ".pi/agent/settings.json",
        "{\"defaultProvider\": \"none\"}\n",
    ),
];

const CREDENTIALS: [&str; 2] = [".claude/.credentials.json", ".pi/agent/auth.json"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    bytes: Vec<u8>,
    modified: SystemTime,
}

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

fn is_kept_by_a_cli(path: &Path) -> bool {
    let first = path.components().next().expect("a path");
    [".claude", ".claude.json", ".pi"]
        .iter()
        .any(|own| first.as_os_str() == *own)
}

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

fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

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

fn marker() -> String {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .expect("random bytes");
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("marker-{hex}")
}

struct Prepared {
    world: World,
    repo: yaar_system_tests::Repo,
    marker: String,
    before: BTreeMap<PathBuf, Held>,
}

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

fn plant(world: &World, relative: &str, content: &str) {
    let file = world.home().join(relative);
    fs::create_dir_all(file.parent().expect("a folder")).expect("make the folder");
    fs::write(&file, content).expect("plant the file");
}

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

/// Its children go first: the world ends a tracer as soon as the Keeper's own process is gone.
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

#[test]
fn the_settings_of_both_clis_are_unchanged_after_a_turn_and_a_marker_in_their_credentials_is_found_nowhere_the_app_stored()
 {
    settings_are_unchanged(Dialect::ClaudeCode);
}

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
    assert_eq!(
        before.keys().filter(|path| is_kept_by_a_cli(path)).count(),
        SETTINGS.len() + CREDENTIALS.len()
    );

    if dialect == Dialect::Pi {
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
    // The Keeper ends before anything is looked at: what it does as it ends is part of what it did.
    stop_the_keeper(&mut world, &mut client);

    let after = record(&home, &data);
    assert_eq!(differences(&before, &after), Vec::<String>::new());
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

#[test]
fn a_recording_of_a_home_finds_a_changed_a_touched_a_removed_and_an_added_file_of_a_cli() {
    let world = World::new();
    let home = world.home();
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
    fs::File::options()
        .write(true)
        .open(home.join(".claude.json"))
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    fs::remove_file(home.join(".pi/agent/removed.json")).unwrap();
    plant(&world, ".pi/agent/added.json", "{}\n");
    plant(&world, ".claude/settings.local.json", "{}\n");
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

    // The marker is a plain file in the worktree. git compresses its objects, so a search of the
    // files misses it there and `objects_hold` asks git.
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
    assert_ne!(marker, self::marker());
}

fn names_a_credential_file(line: &str) -> bool {
    line.contains(".credentials.json") || (line.contains("auth.json") && line.contains(".pi/agent"))
}

#[derive(Debug, PartialEq, Eq)]
struct Call {
    pid: u32,
    name: String,
    rest: String,
}

/// An unfinished call and the line that resumed it are one call.
fn calls(trace: &str) -> Vec<Call> {
    let mut unfinished: HashMap<u32, (String, String)> = HashMap::new();
    let mut found = Vec::new();
    for line in trace.lines() {
        let (pid, body) = split_pid(line);
        if let Some(resumed) = body.strip_prefix("<... ") {
            let Some((name, rest)) = resumed.split_once(" resumed>") else {
                continue;
            };
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

fn credential_lines(trace: &str) -> Vec<&str> {
    trace
        .lines()
        .filter(|line| names_a_credential_file(line))
        .collect()
}

/// Whether a successful rename put a file over pi's settings, the way the app writes them.
/// The settings must be the call's last path: the temporary file's name only starts with theirs.
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
    assert_eq!(run.get(&4003).map(String::as_str), Some("yaar-claude-hook"));
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

/// `%file` names the calls that take a path on every architecture, as arm64 has no `open`.
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

#[cfg(target_os = "linux")]
#[test]
fn neither_the_keeper_nor_a_companion_opens_a_credential_file_of_a_cli_in_a_traced_turn() {
    nothing_opens_a_credential_file(Dialect::ClaudeCode);
}

#[cfg(target_os = "linux")]
#[test]
fn neither_the_keeper_nor_a_companion_opens_a_credential_file_of_a_cli_in_a_traced_turn_as_pi() {
    nothing_opens_a_credential_file(Dialect::Pi);
}

/// No process of the trace names the extension: it is TypeScript run by pi.
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
        "the test that no search opens a credential file needs `strace` on the search path: install it (`apt-get install strace`)"
    );
    let trace_of = |world: &World| world.home().with_file_name("trace.txt");
    let Prepared {
        mut world, repo, ..
    } = world_with_files(|world| strace_wrapper(&trace_of(world)));
    if dialect == Dialect::Pi {
        world.allow_pi_extension();
    }
    let mut client = full_turn(&world, &repo, dialect);

    // The Keeper ends of its own accord, and the tracer with it, so that everything it saw is
    // written.
    stop_the_keeper(&mut world, &mut client);
    let trace = fs::read(trace_of(&world)).expect("the trace of the Keeper");
    let trace = String::from_utf8_lossy(&trace);

    let run = programs(&trace);
    for program in ["yaar-keeper", companion_program(dialect)] {
        assert!(
            run.values().any(|name| name == program),
            "the trace holds no process of {program}: {run:?}"
        );
    }
    if dialect == Dialect::Pi {
        assert!(
            replaces_the_settings_of_pi(&trace),
            "the trace holds no change of pi's settings"
        );
    }
    let named = credential_lines(&trace);
    assert!(named.is_empty(), "{named:#?}");
    drop(client);
}
