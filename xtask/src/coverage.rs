//! The test plan, checked: every requirement has a test, and what the plan calls built is
//! where the plan says.
//!
//! The plan is three files in `test-plan/`: the requirements, the tests that prove them,
//! and the slices the app is built in.
//!
//! - Requirements. An id is `FR-` or `NFR-` and three digits, and is used once. A
//!   requirement is for `R1`, for `Later`, or `Withdrawn`.
//! - Tests. Every key is known and every value comes from its list. A test proves
//!   requirements that exist and are not withdrawn, or names its slice. A unit test goes
//!   through a call and nothing else does. A system test goes through the programs. What
//!   needs a person or real hardware runs at release, from a checklist.
//! - Coverage. Every requirement that is not withdrawn has a test. Every requirement of R1
//!   has one that runs in CI, unless the plan says why it is proven by hand only.
//! - Built. A test called built names a test function or a workflow step that is there.
//! - Slices. Numbered from 1 with no gap. Each requirement of R1 is closed by one slice.
//!   Once a slice is closed, each of its tests that runs in CI is built.
//!
//! A test is built when its `built` list holds something, and planned until then.

use std::collections::{BTreeMap, BTreeSet};

use toml::{Table, Value};

/// Where the requirements are, from the root of the repository.
pub const REQUIREMENTS: &str = "test-plan/requirements.toml";
/// Where the tests are, from the root of the repository.
pub const TESTS: &str = "test-plan/tests.toml";
/// Where the slices are, from the root of the repository.
pub const SLICES: &str = "test-plan/slices.toml";

const RELEASES: [&str; 3] = ["R1", "Later", "Withdrawn"];
const LEVELS: [&str; 3] = ["unit", "component", "system"];
const PARTS: [&str; 23] = [
    "link",
    "diff",
    "agent",
    "keeper",
    "dock",
    "switchboard",
    "adapter-claude",
    "adapter-pi",
    "review-engine",
    "store",
    "recorder",
    "workspaces",
    "window",
    "frame",
    "review-pane",
    "terminal-pane",
    "navigator",
    "text-search",
    "companion-claude",
    "companion-pi",
    "build",
    "package",
    "app",
];
const DRIVERS: [&str; 14] = [
    "call",
    "test-platform",
    "git",
    "database",
    "pty",
    "recorded-events",
    "language-servers",
    "client",
    "agent",
    "workflow",
    "real-clis",
    "lit-screen",
    "person",
    "linux-hardware",
];
/// The drivers that start the built programs.
const SYSTEM_DRIVERS: [&str; 7] = [
    "client",
    "agent",
    "workflow",
    "real-clis",
    "lit-screen",
    "person",
    "linux-hardware",
];
/// The drivers that no CI machine has.
const RELEASE_DRIVERS: [&str; 4] = ["real-clis", "lit-screen", "person", "linux-hardware"];
const RUNS: [&str; 2] = ["ci", "release"];
const CHECKLISTS: [&str; 4] = ["conventions", "lit-screen", "real-clis", "linux-hardware"];
const TEST_KEYS: [&str; 12] = [
    "id",
    "what",
    "level",
    "part",
    "through",
    "driver",
    "runs",
    "proves",
    "checklist",
    "slice",
    "built",
    "note",
];

/// How wide a line of the list may be.
const LIST_WIDTH: usize = 100;

/// What the check found in a plan.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// One sentence per breach of a rule.
    pub breaches: Vec<String>,
    /// The requirements that are not withdrawn.
    pub requirements: usize,
    /// The tests, built or not.
    pub tests: usize,
    /// The tests that name where they are built.
    pub built: usize,
    /// The tests that are not built yet.
    pub planned: usize,
    /// The requirements the plan says are proven by hand only.
    pub by_hand_only: usize,
    /// The highest slice number.
    pub slices: usize,
    /// The highest slice whose tests must all be built.
    pub closed: usize,
    /// Each requirement in file order and, indented under it, the tests that prove it.
    pub list: Vec<String>,
}

impl Report {
    /// The one line that says what a plan without a breach holds.
    pub fn summary(&self) -> String {
        format!(
            "{} requirements, each with a test or more; {} tests, {} built, {} planned; \
             {} requirements proven by hand only; {} of {} slices closed",
            self.requirements,
            self.tests,
            self.built,
            self.planned,
            self.by_hand_only,
            self.closed,
            self.slices
        )
    }
}

struct Requirement<'a> {
    id: &'a str,
    /// Missing when the file names a release that does not exist.
    release: Option<&'a str>,
}

struct Test<'a> {
    /// Its id, or its place in the file when it has none.
    name: String,
    what: &'a str,
    level: Option<&'a str>,
    runs: Option<&'a str>,
    proves: Vec<&'a str>,
    slice: Option<i64>,
    built: bool,
}

impl Test<'_> {
    fn line(&self, widest_name: usize) -> String {
        let start = format!(
            "  {:widest_name$}  {:9}  {:7}  {:7}  ",
            self.name,
            self.level.unwrap_or("?"),
            self.runs.unwrap_or("?"),
            if self.built { "built" } else { "planned" }
        );
        let room = LIST_WIDTH.saturating_sub(start.chars().count());
        format!("{start}{}", cut(self.what, room))
    }
}

/// Checks the three files of the plan. `file` gives the content of a file by its path from
/// the root of the repository, or nothing when there is no such file.
///
/// A breach of a rule is a sentence in the report. A file that is not TOML is an error.
pub fn check(
    requirements: &str,
    tests: &str,
    slices: &str,
    file: &dyn Fn(&str) -> Option<String>,
) -> Result<Report, String> {
    let requirements_doc = parse(REQUIREMENTS, requirements)?;
    let tests_doc = parse(TESTS, tests)?;
    let slices_doc = parse(SLICES, slices)?;

    let mut breaches = Vec::new();
    let requirements = read_requirements(&requirements_doc, &mut breaches);
    let release_of: BTreeMap<&str, Option<&str>> =
        requirements.iter().map(|r| (r.id, r.release)).collect();
    let tests = read_tests(&tests_doc, &release_of, file, &mut breaches);
    let by_hand = read_by_hand_only(&tests_doc, &release_of, &mut breaches);
    check_coverage(&requirements, &tests, &by_hand, &mut breaches);
    let (slices, closed) = check_slices(
        &slices_doc,
        &requirements,
        &release_of,
        &tests,
        &mut breaches,
    );

    let widest_name = tests.iter().map(|t| t.name.chars().count()).max();
    let mut list = Vec::new();
    for requirement in &requirements {
        let id = requirement.id;
        list.push(format!("{id}  {}", requirement.release.unwrap_or("?")));
        for test in tests.iter().filter(|t| t.proves.contains(&id)) {
            list.push(test.line(widest_name.unwrap_or_default()));
        }
    }

    let built = tests.iter().filter(|t| t.built).count();
    Ok(Report {
        breaches,
        requirements: requirements
            .iter()
            .filter(|r| r.release != Some("Withdrawn"))
            .count(),
        tests: tests.len(),
        built,
        planned: tests.len() - built,
        by_hand_only: by_hand.len(),
        slices: usize::try_from(slices).unwrap_or_default(),
        closed: usize::try_from(closed).unwrap_or_default(),
        list,
    })
}

fn parse(path: &str, text: &str) -> Result<Table, String> {
    text.parse::<Table>()
        .map_err(|e| format!("{path} is not TOML: {e}"))
}

/// The tables under `[[key]]`, in file order.
fn tables<'a>(doc: &'a Table, key: &str, path: &str, breaches: &mut Vec<String>) -> Vec<&'a Table> {
    let Some(value) = doc.get(key) else {
        return Vec::new();
    };
    let found = value.as_array().and_then(|items| {
        items
            .iter()
            .map(Value::as_table)
            .collect::<Option<Vec<_>>>()
    });
    found.unwrap_or_else(|| {
        breaches.push(format!("`{key}` in {path} is not a list of tables"));
        Vec::new()
    })
}

/// The text under `key`. A missing or empty one is a breach.
fn text<'a>(table: &'a Table, key: &str, who: &str, breaches: &mut Vec<String>) -> Option<&'a str> {
    match table.get(key).map(Value::as_str) {
        Some(Some(text)) if !text.trim().is_empty() => return Some(text),
        Some(Some(_)) => breaches.push(format!("{who} has an empty `{key}`")),
        Some(None) => breaches.push(format!("{who}: `{key}` is not text")),
        None => breaches.push(format!("{who} has no `{key}`")),
    }
    None
}

/// The text under `key`, when it is one of `allowed`.
fn one_of<'a>(
    table: &'a Table,
    key: &str,
    allowed: &[&str],
    who: &str,
    breaches: &mut Vec<String>,
) -> Option<&'a str> {
    let value = text(table, key, who, breaches)?;
    if allowed.contains(&value) {
        return Some(value);
    }
    breaches.push(format!(
        "{who}: `{key}` is `{value}`, which is not one of {}",
        allowed.join(", ")
    ));
    None
}

/// The texts of the list under `key`.
fn texts<'a>(table: &'a Table, key: &str, who: &str, breaches: &mut Vec<String>) -> Vec<&'a str> {
    let Some(value) = table.get(key) else {
        breaches.push(format!("{who} has no `{key}`"));
        return Vec::new();
    };
    let found = value
        .as_array()
        .and_then(|items| items.iter().map(Value::as_str).collect::<Option<Vec<_>>>());
    found.unwrap_or_else(|| {
        breaches.push(format!("{who}: `{key}` is not a list of texts"));
        Vec::new()
    })
}

fn is_requirement_id(id: &str) -> bool {
    id.strip_prefix("NFR-")
        .or_else(|| id.strip_prefix("FR-"))
        .is_some_and(|digits| digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit()))
}

fn read_requirements<'a>(doc: &'a Table, breaches: &mut Vec<String>) -> Vec<Requirement<'a>> {
    let mut found: Vec<Requirement> = Vec::new();
    let mut seen = BTreeSet::new();
    let entries = tables(doc, "requirement", REQUIREMENTS, breaches);
    for (at, table) in entries.into_iter().enumerate() {
        let place = format!("requirement {}", at + 1);
        let Some(id) = text(table, "id", &place, breaches) else {
            continue;
        };
        if !is_requirement_id(id) {
            breaches.push(format!(
                "the requirement id `{id}` is not FR- or NFR- and three digits"
            ));
        }
        let release = one_of(table, "release", &RELEASES, id, breaches);
        text(table, "text", id, breaches);
        if seen.insert(id) {
            found.push(Requirement { id, release });
        } else {
            breaches.push(format!("{id} is the id of two requirements"));
        }
    }
    if found.is_empty() {
        breaches.push(format!("{REQUIREMENTS} holds no requirement"));
    }
    found
}

fn read_tests<'a>(
    doc: &'a Table,
    release_of: &BTreeMap<&str, Option<&str>>,
    file: &dyn Fn(&str) -> Option<String>,
    breaches: &mut Vec<String>,
) -> Vec<Test<'a>> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let entries = tables(doc, "test", TESTS, breaches);
    for (at, table) in entries.into_iter().enumerate() {
        let place = format!("test {}", at + 1);
        let id = text(table, "id", &place, breaches);
        let who = id.unwrap_or(&place);
        if id.is_some_and(|id| !seen.insert(id)) {
            breaches.push(format!("{who} is the id of two tests"));
        }
        // The order of the keys in a table is not kept on every build.
        let unknown: BTreeSet<&str> = table
            .keys()
            .map(String::as_str)
            .filter(|key| !TEST_KEYS.contains(key))
            .collect();
        for key in unknown {
            breaches.push(format!("{who} has the unknown key `{key}`"));
        }

        let what = text(table, "what", who, breaches).unwrap_or_default();
        text(table, "through", who, breaches);
        let level = one_of(table, "level", &LEVELS, who, breaches);
        one_of(table, "part", &PARTS, who, breaches);
        let driver = one_of(table, "driver", &DRIVERS, who, breaches);
        let runs = one_of(table, "runs", &RUNS, who, breaches);
        let has_checklist = table.contains_key("checklist");
        if has_checklist {
            one_of(table, "checklist", &CHECKLISTS, who, breaches);
        }
        let slice = match table.get("slice").map(Value::as_integer) {
            Some(Some(n)) if n >= 0 => Some(n),
            Some(_) => {
                breaches.push(format!("{who}: `slice` is not a whole number from 0 up"));
                None
            }
            None => None,
        };

        let proves = texts(table, "proves", who, breaches);
        for id in &proves {
            match release_of.get(id) {
                None => breaches.push(format!("{who} proves {id}, which is no requirement")),
                Some(Some("Withdrawn")) => {
                    breaches.push(format!("{who} proves {id}, which is withdrawn"));
                }
                Some(_) => {}
            }
        }
        if proves.is_empty() && !table.contains_key("slice") {
            breaches.push(format!(
                "{who} proves no requirement and so must name its slice"
            ));
        }

        if let (Some(level), Some(driver)) = (level, driver) {
            if (level == "unit") != (driver == "call") {
                breaches.push(format!(
                    "{who} is a {level} test with the driver {driver}: a unit test goes \
                     through a call, and no other test does"
                ));
            } else if level == "system" && !SYSTEM_DRIVERS.contains(&driver) {
                breaches.push(format!(
                    "{who} is a system test with the driver {driver}, which starts no program"
                ));
            }
        }
        if let (Some(driver), Some("ci")) = (driver, runs)
            && RELEASE_DRIVERS.contains(&driver)
        {
            breaches.push(format!(
                "{who} runs in ci with the driver {driver}, which only a release has"
            ));
        }
        match runs {
            Some("release") if !has_checklist && driver != Some("workflow") => {
                breaches.push(format!("{who} runs at release and names no checklist"));
            }
            Some("ci") if has_checklist => {
                breaches.push(format!("{who} runs in ci and names a checklist"));
            }
            _ => {}
        }

        let built = if table.contains_key("built") {
            texts(table, "built", who, breaches)
        } else {
            Vec::new()
        };
        for entry in &built {
            check_built(who, entry, file, breaches);
        }

        found.push(Test {
            name: who.to_owned(),
            what,
            level,
            runs,
            proves,
            slice,
            built: !built.is_empty(),
        });
    }
    found
}

/// One entry of a `built` list: `path::name`, where the path is a Rust file that holds the
/// function `name`, or a workflow that holds the step or job `name`.
fn check_built(
    who: &str,
    entry: &str,
    file: &dyn Fn(&str) -> Option<String>,
    breaches: &mut Vec<String>,
) {
    let split = entry
        .rsplit_once("::")
        .filter(|(path, name)| !path.is_empty() && !name.is_empty());
    let Some((path, name)) = split else {
        breaches.push(format!("{who}: `{entry}` is not `path::name`"));
        return;
    };
    let is_rust = path.ends_with(".rs");
    if !is_rust && !path.ends_with(".yml") {
        breaches.push(format!("{who}: {path} is neither a .rs nor a .yml file"));
        return;
    }
    let Some(content) = file(path) else {
        breaches.push(format!("{who}: {path} is not a file"));
        return;
    };
    if is_rust {
        if !content.contains(&format!("fn {name}(")) {
            breaches.push(format!("{who}: {path} has no function {name}"));
        }
    } else {
        let wanted = format!("name: {name}");
        let there = content
            .lines()
            .map(str::trim)
            .any(|line| line.strip_prefix("- ").unwrap_or(line) == wanted);
        if !there {
            breaches.push(format!("{who}: {path} has nothing named `{name}`"));
        }
    }
}

/// The requirements that have a `[[by_hand_only]]` entry.
fn read_by_hand_only<'a>(
    doc: &'a Table,
    release_of: &BTreeMap<&str, Option<&str>>,
    breaches: &mut Vec<String>,
) -> BTreeSet<&'a str> {
    let mut found = BTreeSet::new();
    let entries = tables(doc, "by_hand_only", TESTS, breaches);
    for (at, table) in entries.into_iter().enumerate() {
        let place = format!("by_hand_only entry {}", at + 1);
        let Some(id) = text(table, "id", &place, breaches) else {
            continue;
        };
        text(
            table,
            "why",
            &format!("the by_hand_only entry of {id}"),
            breaches,
        );
        match release_of.get(id) {
            None => breaches.push(format!(
                "a by_hand_only entry names {id}, which is no requirement"
            )),
            Some(Some(release)) if *release != "R1" => breaches.push(format!(
                "a by_hand_only entry names {id}, which is {release} and not R1"
            )),
            Some(_) => {}
        }
        if !found.insert(id) {
            breaches.push(format!("{id} has two by_hand_only entries"));
        }
    }
    found
}

fn check_coverage(
    requirements: &[Requirement],
    tests: &[Test],
    by_hand: &BTreeSet<&str>,
    breaches: &mut Vec<String>,
) {
    for requirement in requirements {
        let id = requirement.id;
        let naming: Vec<&Test> = tests.iter().filter(|t| t.proves.contains(&id)).collect();
        let in_ci = naming.iter().find(|t| t.runs == Some("ci"));
        let at_release = naming.iter().any(|t| t.runs == Some("release"));
        match requirement.release {
            None | Some("Withdrawn") => {}
            Some("R1") if by_hand.contains(id) => {
                if let Some(test) = in_ci {
                    breaches.push(format!(
                        "{id} is by hand only, yet {} runs in ci: the entry is stale",
                        test.name
                    ));
                } else if !at_release {
                    breaches.push(format!("{id} is by hand only and has no release test"));
                }
            }
            Some(_) if naming.is_empty() => breaches.push(format!("{id} has no test")),
            Some("R1") if in_ci.is_none() => breaches.push(format!(
                "{id} has no test that runs in ci and no by_hand_only entry"
            )),
            Some(_) => {}
        }
    }
}

/// Returns the highest slice number and the highest closed slice.
fn check_slices(
    doc: &Table,
    requirements: &[Requirement],
    release_of: &BTreeMap<&str, Option<&str>>,
    tests: &[Test],
    breaches: &mut Vec<String>,
) -> (i64, i64) {
    // How often each number is used, and the slices that close each requirement of R1.
    let mut times: BTreeMap<i64, usize> = BTreeMap::new();
    let mut closing: BTreeMap<&str, BTreeSet<i64>> = BTreeMap::new();
    let entries = tables(doc, "slice", SLICES, breaches);
    for (at, table) in entries.into_iter().enumerate() {
        let n = match table.get("n").and_then(Value::as_integer) {
            Some(n) if n >= 1 => n,
            _ => {
                breaches.push(format!(
                    "slice entry {} has no whole number `n` from 1 up",
                    at + 1
                ));
                continue;
            }
        };
        *times.entry(n).or_default() += 1;
        let who = format!("slice {n}");
        for id in texts(table, "closes", &who, breaches) {
            match release_of.get(id) {
                None => breaches.push(format!("{who} closes {id}, which is no requirement")),
                Some(Some("R1")) => {
                    closing.entry(id).or_default().insert(n);
                }
                Some(Some(release)) => {
                    breaches.push(format!("{who} closes {id}, which is {release} and not R1"))
                }
                Some(None) => {}
            }
        }
    }

    let mut before = 0;
    for (&n, &count) in &times {
        if count > 1 {
            breaches.push(format!("{count} slices have the number {n}"));
        }
        if n != before + 1 {
            breaches.push(format!(
                "slice {n} comes after slice {before}: the numbers have a gap"
            ));
        }
        before = n;
    }
    let highest = before;

    for requirement in requirements {
        if requirement.release != Some("R1") {
            continue;
        }
        let id = requirement.id;
        match closing.get(id) {
            None => breaches.push(format!("{id} is closed by no slice")),
            Some(slices) if slices.len() > 1 => {
                let slices: Vec<String> = slices.iter().map(i64::to_string).collect();
                breaches.push(format!("{id} is closed by slices {}", slices.join(" and ")));
            }
            Some(_) => {}
        }
    }

    let closed = match doc.get("closed").and_then(Value::as_integer) {
        Some(closed) if (0..=highest).contains(&closed) => closed,
        Some(closed) => {
            breaches.push(format!(
                "`closed` is {closed}, which is not between 0 and the highest slice, {highest}"
            ));
            return (highest, 0);
        }
        None => {
            breaches.push(format!("{SLICES} has no whole number `closed`"));
            return (highest, 0);
        }
    };

    // Release tests wait for a release, whatever is closed.
    for test in tests.iter().filter(|t| t.runs == Some("ci") && !t.built) {
        let slice = test.slice.or_else(|| {
            test.proves
                .iter()
                .filter_map(|id| closing.get(id)?.first().copied())
                .min()
        });
        if let Some(slice) = slice.filter(|slice| *slice <= closed) {
            breaches.push(format!(
                "slice {slice} is closed but {} is not built",
                test.name
            ));
        }
    }
    (highest, closed)
}

/// `text`, cut to `width` characters. A cut text ends in `…`.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small plan that keeps every rule: two requirements of R1, one of them proven by
    /// hand only, one for later and one withdrawn.
    const REQUIREMENTS_SOUND: &str = r#"
version = "v1"

[[requirement]]
id = "FR-001"
release = "R1"
section = "Diff"
text = "The app shall show the changes of a branch."

[[requirement]]
id = "NFR-001"
release = "R1"
section = "Speed"
text = "The app shall show its window within a second of a cold start."

[[requirement]]
id = "FR-002"
release = "Later"
section = "Diff"
text = "The app shall show the changes of a pull request."

[[requirement]]
id = "FR-003"
release = "Withdrawn"
section = "Diff"
text = "The app shall show the changes of a patch file."
"#;

    const TESTS_SOUND: &str = r#"
[[test]]
id = "DIFF-01"
what = "A changed line shows as changed."
level = "unit"
part = "diff"
through = "public functions"
driver = "call"
runs = "ci"
proves = ["FR-001"]
built = ["crates/shared/diff/src/lib.rs::a_changed_line_shows_as_changed"]

[[test]]
id = "START-01"
what = "A cold start is timed on a lit screen after a restart."
level = "system"
part = "app"
through = "the packaged app"
driver = "lit-screen"
runs = "release"
checklist = "lit-screen"
proves = ["NFR-001"]

[[test]]
id = "PULL-01"
what = "The changes of a pull request are read from a repository made for the test."
level = "component"
part = "workspaces"
through = "public functions"
driver = "git"
runs = "ci"
proves = ["FR-002"]

[[test]]
id = "BUILD-01"
what = "The rules between the crates hold."
level = "system"
part = "build"
through = "the workflow"
driver = "workflow"
runs = "ci"
proves = []
slice = 0
built = [".github/workflows/ci.yml::Layout rules"]
note = "Proves no requirement: it keeps the build honest."

[[by_hand_only]]
id = "NFR-001"
why = "A cold start is timed on a lit screen after a restart."
"#;

    const SLICES_SOUND: &str = r#"
closed = 0

[[slice]]
n = 1
name = "Read a diff"
builds = "The diff, from git to the review pane."
closes = ["FR-001"]

[[slice]]
n = 2
name = "Start fast"
builds = "The window before anything else."
closes = ["NFR-001"]
"#;

    /// The two files the sound plan says its tests are built in.
    fn disk(path: &str) -> Option<String> {
        match path {
            "crates/shared/diff/src/lib.rs" => {
                Some("#[test]\nfn a_changed_line_shows_as_changed() {}\n".to_owned())
            }
            ".github/workflows/ci.yml" => Some(
                "jobs:\n  checks:\n    name: checks\n    steps:\n      - name: Layout rules\n        run: cargo xtask layout\n"
                    .to_owned(),
            ),
            _ => None,
        }
    }

    fn report_of(requirements: &str, tests: &str, slices: &str) -> Report {
        check(requirements, tests, slices, &disk).expect("the plan is TOML")
    }

    /// The breaches of the sound plan with one piece of its tests replaced.
    fn breaches_with_tests(from: &str, to: &str) -> Vec<String> {
        assert!(TESTS_SOUND.contains(from), "{from}");
        let tests = TESTS_SOUND.replace(from, to);
        report_of(REQUIREMENTS_SOUND, &tests, SLICES_SOUND).breaches
    }

    /// The same, for its slices.
    fn breaches_with_slices(from: &str, to: &str) -> Vec<String> {
        assert!(SLICES_SOUND.contains(from), "{from}");
        let slices = SLICES_SOUND.replace(from, to);
        report_of(REQUIREMENTS_SOUND, TESTS_SOUND, &slices).breaches
    }

    /// The same, for its requirements.
    fn breaches_with_requirements(from: &str, to: &str) -> Vec<String> {
        assert!(REQUIREMENTS_SOUND.contains(from), "{from}");
        let requirements = REQUIREMENTS_SOUND.replace(from, to);
        report_of(&requirements, TESTS_SOUND, SLICES_SOUND).breaches
    }

    #[test]
    fn a_sound_plan_has_no_breach() {
        let report = report_of(REQUIREMENTS_SOUND, TESTS_SOUND, SLICES_SOUND);
        assert_eq!(report.breaches, Vec::<String>::new());
        assert_eq!(
            report.summary(),
            "3 requirements, each with a test or more; 4 tests, 2 built, 2 planned; \
             1 requirements proven by hand only; 0 of 2 slices closed"
        );
    }

    #[test]
    fn a_requirement_with_no_test_is_a_breach() {
        let found = breaches_with_tests(r#"proves = ["FR-002"]"#, r#"proves = ["FR-001"]"#);
        assert_eq!(found, ["FR-002 has no test"]);
    }

    #[test]
    fn a_withdrawn_requirement_needs_no_test() {
        let found = report_of(REQUIREMENTS_SOUND, TESTS_SOUND, SLICES_SOUND).breaches;
        assert!(!found.iter().any(|b| b.contains("FR-003")), "{found:?}");
    }

    #[test]
    fn an_r1_requirement_with_only_a_release_test_needs_a_by_hand_only_entry() {
        let (tests, _) = TESTS_SOUND
            .split_once("[[by_hand_only]]")
            .expect("the entry");
        let found = report_of(REQUIREMENTS_SOUND, tests, SLICES_SOUND).breaches;
        assert_eq!(
            found,
            ["NFR-001 has no test that runs in ci and no by_hand_only entry"]
        );
    }

    #[test]
    fn a_later_requirement_may_have_only_a_release_test() {
        let found = breaches_with_tests(
            "driver = \"git\"\nruns = \"ci\"",
            "driver = \"git\"\nruns = \"release\"\nchecklist = \"conventions\"",
        );
        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_by_hand_only_entry_is_stale_once_a_ci_test_proves_its_requirement() {
        let found = breaches_with_tests(
            r#"proves = ["FR-001"]"#,
            r#"proves = ["FR-001", "NFR-001"]"#,
        );
        assert_eq!(
            found,
            ["NFR-001 is by hand only, yet DIFF-01 runs in ci: the entry is stale"]
        );
    }

    #[test]
    fn a_by_hand_only_entry_needs_a_release_test() {
        let found = breaches_with_tests(r#"proves = ["NFR-001"]"#, r#"proves = ["FR-002"]"#);
        assert_eq!(found, ["NFR-001 is by hand only and has no release test"]);
    }

    #[test]
    fn a_by_hand_only_entry_names_a_requirement_of_r1_and_says_why() {
        let later = breaches_with_tests(
            "[[by_hand_only]]",
            "[[by_hand_only]]\nid = \"FR-002\"\nwhy = \"x\"\n\n[[by_hand_only]]",
        );
        assert_eq!(
            later,
            ["a by_hand_only entry names FR-002, which is Later and not R1"]
        );

        let unknown = breaches_with_tests(
            "[[by_hand_only]]",
            "[[by_hand_only]]\nid = \"FR-999\"\nwhy = \"x\"\n\n[[by_hand_only]]",
        );
        assert_eq!(
            unknown,
            ["a by_hand_only entry names FR-999, which is no requirement"]
        );

        let silent = breaches_with_tests(
            "why = \"A cold start is timed on a lit screen after a restart.\"",
            "why = \"\"",
        );
        assert_eq!(
            silent,
            ["the by_hand_only entry of NFR-001 has an empty `why`"]
        );
    }

    #[test]
    fn a_test_that_names_an_unknown_requirement_is_a_breach() {
        let found =
            breaches_with_tests(r#"proves = ["FR-001"]"#, r#"proves = ["FR-001", "FR-999"]"#);
        assert_eq!(found, ["DIFF-01 proves FR-999, which is no requirement"]);
    }

    #[test]
    fn a_withdrawn_requirement_named_by_a_test_is_a_breach() {
        let found =
            breaches_with_tests(r#"proves = ["FR-001"]"#, r#"proves = ["FR-001", "FR-003"]"#);
        assert_eq!(found, ["DIFF-01 proves FR-003, which is withdrawn"]);
    }

    #[test]
    fn a_test_that_proves_nothing_must_name_its_slice() {
        let found = breaches_with_tests("slice = 0\n", "");
        assert_eq!(
            found,
            ["BUILD-01 proves no requirement and so must name its slice"]
        );
    }

    #[test]
    fn a_unit_test_with_a_driver_other_than_call_is_a_breach() {
        let found = breaches_with_tests(r#"driver = "call""#, r#"driver = "git""#);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("DIFF-01 is a unit test with the driver git"),
            "{found:?}"
        );
    }

    #[test]
    fn a_test_above_unit_that_goes_through_a_call_is_a_breach() {
        let found = breaches_with_tests(r#"driver = "git""#, r#"driver = "call""#);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("PULL-01 is a component test with the driver call"),
            "{found:?}"
        );
    }

    #[test]
    fn a_system_test_with_a_driver_that_starts_no_program_is_a_breach() {
        let found = breaches_with_tests(r#"level = "component""#, r#"level = "system""#);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("PULL-01 is a system test with the driver git"),
            "{found:?}"
        );
    }

    #[test]
    fn a_driver_that_only_a_release_has_cannot_run_in_ci() {
        let found = breaches_with_tests(
            "runs = \"release\"\nchecklist = \"lit-screen\"",
            "runs = \"ci\"",
        );
        assert!(
            found
                .iter()
                .any(|b| b.starts_with("START-01 runs in ci with the driver lit-screen")),
            "{found:?}"
        );
    }

    #[test]
    fn a_release_test_with_no_checklist_is_a_breach() {
        let found = breaches_with_tests("checklist = \"lit-screen\"\n", "");
        assert_eq!(found, ["START-01 runs at release and names no checklist"]);
    }

    #[test]
    fn a_release_test_of_a_workflow_needs_no_checklist() {
        let found = breaches_with_tests(
            "driver = \"workflow\"\nruns = \"ci\"",
            "driver = \"workflow\"\nruns = \"release\"",
        );
        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_ci_test_with_a_checklist_is_a_breach() {
        let found = breaches_with_tests(
            "driver = \"call\"\n",
            "driver = \"call\"\nchecklist = \"conventions\"\n",
        );
        assert_eq!(found, ["DIFF-01 runs in ci and names a checklist"]);
    }

    #[test]
    fn a_built_test_whose_function_is_not_in_the_file_is_a_breach() {
        let found = breaches_with_tests(
            "lib.rs::a_changed_line_shows_as_changed",
            "lib.rs::a_changed_line_shows",
        );
        assert_eq!(
            found,
            ["DIFF-01: crates/shared/diff/src/lib.rs has no function a_changed_line_shows"]
        );
    }

    #[test]
    fn a_built_workflow_step_that_does_not_exist_is_a_breach() {
        let found = breaches_with_tests("ci.yml::Layout rules", "ci.yml::Layout");
        assert_eq!(
            found,
            ["BUILD-01: .github/workflows/ci.yml has nothing named `Layout`"]
        );
    }

    #[test]
    fn a_built_workflow_job_is_found_by_its_name() {
        let found = breaches_with_tests("ci.yml::Layout rules", "ci.yml::checks");
        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_built_entry_names_a_rust_file_or_a_workflow_that_is_there() {
        let missing = breaches_with_tests("diff/src/lib.rs::", "diff/src/gone.rs::");
        assert_eq!(
            missing,
            ["DIFF-01: crates/shared/diff/src/gone.rs is not a file"]
        );

        let other =
            breaches_with_tests(".github/workflows/ci.yml::Layout rules", "README.md::Check");
        assert_eq!(
            other,
            ["BUILD-01: README.md is neither a .rs nor a .yml file"]
        );

        let no_name = breaches_with_tests("ci.yml::Layout rules", "ci.yml");
        assert_eq!(
            no_name,
            ["BUILD-01: `.github/workflows/ci.yml` is not `path::name`"]
        );
    }

    #[test]
    fn a_requirement_closed_by_two_slices_and_one_closed_by_none_are_breaches() {
        let found = breaches_with_slices(r#"closes = ["NFR-001"]"#, r#"closes = ["FR-001"]"#);
        assert_eq!(
            found,
            [
                "FR-001 is closed by slices 1 and 2",
                "NFR-001 is closed by no slice"
            ]
        );
    }

    #[test]
    fn a_slice_closes_only_requirements_of_r1() {
        let later = breaches_with_slices(r#"["NFR-001"]"#, r#"["NFR-001", "FR-002"]"#);
        assert_eq!(later, ["slice 2 closes FR-002, which is Later and not R1"]);

        let withdrawn = breaches_with_slices(r#"["NFR-001"]"#, r#"["NFR-001", "FR-003"]"#);
        assert_eq!(
            withdrawn,
            ["slice 2 closes FR-003, which is Withdrawn and not R1"]
        );

        let unknown = breaches_with_slices(r#"["NFR-001"]"#, r#"["NFR-001", "FR-999"]"#);
        assert_eq!(unknown, ["slice 2 closes FR-999, which is no requirement"]);
    }

    #[test]
    fn slice_numbers_with_a_gap_or_a_repeat_are_breaches() {
        let gap = breaches_with_slices("n = 2", "n = 3");
        assert_eq!(gap, ["slice 3 comes after slice 1: the numbers have a gap"]);

        let repeat = breaches_with_slices("n = 2", "n = 1");
        assert_eq!(repeat, ["2 slices have the number 1"]);

        let from_zero = breaches_with_slices("n = 1", "n = 0");
        assert!(
            from_zero.contains(&"slice entry 1 has no whole number `n` from 1 up".to_owned()),
            "{from_zero:?}"
        );
    }

    #[test]
    fn a_closed_slice_with_a_planned_test_is_a_breach() {
        let built = breaches_with_slices("closed = 0", "closed = 1");
        assert_eq!(built, Vec::<String>::new());

        let slices = SLICES_SOUND.replace("closed = 0", "closed = 1");
        let tests = TESTS_SOUND.replace(
            "built = [\"crates/shared/diff/src/lib.rs::a_changed_line_shows_as_changed\"]\n",
            "",
        );
        let found = report_of(REQUIREMENTS_SOUND, &tests, &slices).breaches;
        assert_eq!(found, ["slice 1 is closed but DIFF-01 is not built"]);

        let open = report_of(REQUIREMENTS_SOUND, &tests, SLICES_SOUND).breaches;
        assert_eq!(open, Vec::<String>::new());
    }

    #[test]
    fn a_test_that_names_its_slice_is_held_to_that_slice() {
        let found =
            breaches_with_tests("built = [\".github/workflows/ci.yml::Layout rules\"]\n", "");
        assert_eq!(found, ["slice 0 is closed but BUILD-01 is not built"]);
    }

    #[test]
    fn a_release_test_is_never_held_to_a_closed_slice() {
        let found = breaches_with_slices("closed = 0", "closed = 2");
        assert_eq!(found, Vec::<String>::new());
    }

    #[test]
    fn a_ci_test_that_proves_only_later_requirements_has_no_slice() {
        let report = report_of(
            REQUIREMENTS_SOUND,
            TESTS_SOUND,
            &SLICES_SOUND.replace("closed = 0", "closed = 2"),
        );
        // PULL-01 is planned, runs in ci and proves only FR-002.
        assert_eq!(report.breaches, Vec::<String>::new());
        assert_eq!(report.planned, 2);
    }

    #[test]
    fn closed_beyond_the_highest_slice_is_a_breach() {
        let found = breaches_with_slices("closed = 0", "closed = 3");
        assert_eq!(
            found,
            ["`closed` is 3, which is not between 0 and the highest slice, 2"]
        );
    }

    #[test]
    fn an_unknown_key_in_a_test_is_a_breach() {
        let found = breaches_with_tests(
            "driver = \"call\"\n",
            "driver = \"call\"\nchecklsit = \"conventions\"\n",
        );
        assert_eq!(found, ["DIFF-01 has the unknown key `checklsit`"]);
    }

    #[test]
    fn a_value_from_outside_its_list_is_a_breach() {
        for (from, to) in [
            (r#"level = "unit""#, r#"level = "integration""#),
            (r#"part = "diff""#, r#"part = "differ""#),
            (r#"driver = "git""#, r#"driver = "svn""#),
            (
                "driver = \"git\"\nruns = \"ci\"",
                "driver = \"git\"\nruns = \"nightly\"",
            ),
            (
                r#"checklist = "lit-screen""#,
                r#"checklist = "dark-screen""#,
            ),
        ] {
            let found = breaches_with_tests(from, to);
            assert_eq!(found.len(), 1, "{to}: {found:?}");
            assert!(found[0].contains("which is not one of"), "{found:?}");
        }
    }

    #[test]
    fn a_test_says_what_it_checks_and_what_it_goes_through() {
        let found = breaches_with_tests(r#"what = "A changed line shows as changed.""#, "");
        assert_eq!(found, ["DIFF-01 has no `what`"]);

        let found = breaches_with_tests(r#"through = "the workflow""#, r#"through = """#);
        assert_eq!(found, ["BUILD-01 has an empty `through`"]);
    }

    #[test]
    fn two_tests_with_one_id_and_a_test_with_none_are_breaches() {
        let twice = breaches_with_tests(r#"id = "PULL-01""#, r#"id = "DIFF-01""#);
        assert_eq!(twice, ["DIFF-01 is the id of two tests"]);

        let none = breaches_with_tests("id = \"PULL-01\"\n", "");
        assert_eq!(none, ["test 3 has no `id`"]);
    }

    #[test]
    fn a_requirement_with_a_bad_id_release_or_text_is_a_breach() {
        let id = breaches_with_requirements(r#"id = "FR-003""#, r#"id = "FR-3""#);
        assert_eq!(
            id,
            ["the requirement id `FR-3` is not FR- or NFR- and three digits"]
        );

        let release = breaches_with_requirements(r#"release = "Withdrawn""#, r#"release = "R2""#);
        assert_eq!(release.len(), 1, "{release:?}");
        assert!(
            release[0].starts_with("FR-003: `release` is `R2`"),
            "{release:?}"
        );

        let text = breaches_with_requirements(
            r#"text = "The app shall show the changes of a patch file.""#,
            r#"text = " ""#,
        );
        assert_eq!(text, ["FR-003 has an empty `text`"]);
    }

    #[test]
    fn two_requirements_with_one_id_are_a_breach() {
        let found = breaches_with_requirements(r#"id = "FR-003""#, r#"id = "FR-002""#);
        assert_eq!(found, ["FR-002 is the id of two requirements"]);
    }

    #[test]
    fn a_file_that_is_not_toml_is_an_error_and_not_a_breach() {
        let error =
            check(REQUIREMENTS_SOUND, "[[test]\n", SLICES_SOUND, &disk).expect_err("half a table");
        assert!(
            error.starts_with("test-plan/tests.toml is not TOML"),
            "{error}"
        );
    }

    #[test]
    fn the_list_has_each_requirement_in_file_order_with_its_tests_under_it() {
        let report = report_of(REQUIREMENTS_SOUND, TESTS_SOUND, SLICES_SOUND);
        assert_eq!(
            report.list,
            [
                "FR-001  R1",
                "  DIFF-01   unit       ci       built    A changed line shows as changed.",
                "NFR-001  R1",
                "  START-01  system     release  planned  A cold start is timed on a lit screen after a restart.",
                "FR-002  Later",
                "  PULL-01   component  ci       planned  The changes of a pull request are read from a repository m…",
                "FR-003  Withdrawn",
            ]
        );
        assert!(report.list.iter().all(|l| l.chars().count() <= LIST_WIDTH));
    }
}
