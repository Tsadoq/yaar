//! The plugin folder the Adapter writes, and the command Claude Code runs from it.

mod support;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use yaar_adapter_claude::{ClaudeCode, HOOK_EVENTS, HOOK_PROGRAM, PLUGIN, PLUGIN_DIRS};
use yaar_agent::{Adapter, ShellEntry};

use support::Scratch;

/// Every file under `folder`, as a path relative to it.
fn files_under(folder: &Path) -> BTreeSet<PathBuf> {
    fn walk(folder: &Path, root: &Path, found: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(folder).expect("read the folder") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                walk(&path, root, found);
            } else {
                found.insert(path.strip_prefix(root).expect("under the root").to_owned());
            }
        }
    }
    let mut found = BTreeSet::new();
    walk(folder, folder, &mut found);
    found
}

fn json_at(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).expect("the file is there");
    serde_json::from_str(&text).expect("the file is JSON")
}

/// Installs into a folder of a scratch, with the companions in `companions`.
fn install(scratch: &Scratch, companions: &Path) -> (PathBuf, Vec<ShellEntry>) {
    let folder = scratch.path().join("agents").join("claude");
    let entries = ClaudeCode::new()
        .install(&folder, companions)
        .expect("the plugin is written");
    (folder, entries)
}

#[test]
fn install_writes_the_two_files_of_the_plugin_and_nothing_else() {
    let scratch = Scratch::new();
    let (folder, _) = install(&scratch, Path::new("/opt/yaar/programs"));

    assert_eq!(
        files_under(&folder),
        BTreeSet::from([
            PathBuf::from("plugin/.claude-plugin/plugin.json"),
            PathBuf::from("plugin/hooks/hooks.json"),
        ])
    );
}

#[test]
fn the_manifest_names_the_plugin_and_the_version_of_the_crate() {
    let scratch = Scratch::new();
    let (folder, _) = install(&scratch, Path::new("/opt/yaar/programs"));

    let manifest = json_at(&folder.join("plugin/.claude-plugin/plugin.json"));

    let keys: BTreeSet<_> = manifest.as_object().expect("an object").keys().collect();
    assert_eq!(
        keys,
        BTreeSet::from([
            &"name".to_owned(),
            &"version".to_owned(),
            &"description".to_owned()
        ])
    );
    assert_eq!(manifest["name"], PLUGIN);
    assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    // One plain sentence: a stray backslash or a line break is what a broken line
    // continuation in the source leaves behind, and Claude Code shows the text as is.
    assert!(
        manifest["description"]
            .as_str()
            .is_some_and(|text| !text.is_empty()
                && !text.contains(['\\', '\n'])
                && !text.contains("  ")
                && text.ends_with('.'))
    );
}

#[test]
fn every_hook_runs_the_quoted_path_of_the_hook_program_for_every_tool() {
    let scratch = Scratch::new();
    let (folder, _) = install(&scratch, Path::new("/opt/yaar/programs"));

    let hooks = json_at(&folder.join("plugin/hooks/hooks.json"));

    let group = json!([{"hooks": [{
        "type": "command",
        "command": format!("'/opt/yaar/programs/{HOOK_PROGRAM}'"),
    }]}]);
    let expected: serde_json::Map<String, Value> = HOOK_EVENTS
        .iter()
        .map(|event| ((*event).to_owned(), group.clone()))
        .collect();
    assert_eq!(hooks, json!({ "hooks": expected }));
}

#[test]
fn the_shells_of_the_app_get_the_plugin_folder_under_the_variable_claude_code_reads() {
    let scratch = Scratch::new();
    let (folder, entries) = install(&scratch, Path::new("/opt/yaar/programs"));

    assert_eq!(
        entries,
        [ShellEntry::PathList {
            name: OsString::from(PLUGIN_DIRS),
            path: folder.join("plugin"),
        }]
    );
}

#[test]
fn installing_again_writes_the_same_files_over_whatever_was_there() {
    let scratch = Scratch::new();
    let (folder, _) = install(&scratch, Path::new("/opt/yaar/programs"));
    let manifest = folder.join("plugin/.claude-plugin/plugin.json");
    let hooks = folder.join("plugin/hooks/hooks.json");
    let before = (
        std::fs::read(&manifest).expect("read"),
        std::fs::read(&hooks).expect("read"),
    );
    std::fs::write(&manifest, "stale").expect("write");
    std::fs::write(&hooks, "{}").expect("write");

    let (_, _) = install(&scratch, Path::new("/opt/yaar/programs"));

    assert_eq!(
        (
            std::fs::read(&manifest).expect("read"),
            std::fs::read(&hooks).expect("read")
        ),
        before
    );
    assert_eq!(files_under(&folder).len(), 2, "no temporary file is left");
}

#[test]
fn a_folder_of_programs_given_as_a_relative_path_is_written_as_an_absolute_one() {
    let scratch = Scratch::new();
    let (folder, _) = install(&scratch, Path::new("programs"));

    let hooks = json_at(&folder.join("plugin/hooks/hooks.json"));

    let command = hooks["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .expect("a command");
    assert!(command.starts_with("'/"), "{command}");
    assert!(
        command.ends_with(&format!("/programs/{HOOK_PROGRAM}'")),
        "{command}"
    );
}

#[test]
fn a_folder_that_cannot_be_made_is_an_error() {
    let scratch = Scratch::new();
    let blocked = scratch.path().join("file");
    std::fs::write(&blocked, "not a folder").expect("write");

    let answer = ClaudeCode::new().install(&blocked.join("claude"), Path::new("/opt/yaar"));

    assert!(answer.is_err());
}

#[test]
fn the_command_runs_a_hook_program_whose_folder_holds_a_space_and_a_quote_in_its_own_place() {
    let scratch = Scratch::new();
    let companions = scratch.path().join("Application Support").join("it's here");
    std::fs::create_dir_all(&companions).expect("make the folder");
    let witness = scratch.path().join("parent");
    // A program of the test's own: it says whose child it is.
    let program = companions.join(HOOK_PROGRAM);
    std::fs::write(
        &program,
        format!("#!/bin/sh\necho $PPID > '{}'\n", witness.display()),
    )
    .expect("write the program");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let (folder, _) = install(&scratch, &companions);
    let hooks = json_at(&folder.join("plugin/hooks/hooks.json"));
    let command = hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .expect("a command");

    let status = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .status()
        .expect("run the command");

    assert!(status.success());
    let parent = std::fs::read_to_string(&witness).expect("the program ran");
    assert_eq!(
        parent.trim(),
        std::process::id().to_string(),
        "`sh` ran the lone command in its own place, so the program's parent is the test"
    );
}
