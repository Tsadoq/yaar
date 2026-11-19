//! The plugin folder the Adapter writes into its own folder of the app.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use yaar_agent::ShellEntry;

use crate::{HOOK_EVENTS, HOOK_PROGRAM, PLUGIN, PLUGIN_DIRS, TOOL_SERVER, TOOL_SERVER_NAME};

/// Writes `folder/plugin` and names it for the shells of the app.
pub(crate) fn write(folder: &Path, companions: &Path) -> io::Result<Vec<ShellEntry>> {
    let plugin = folder.join("plugin");
    let hook = hook_program(companions)?;
    let tools = program_path(companions, TOOL_SERVER)?;

    write_json(&plugin.join(".claude-plugin"), "plugin.json", &manifest())?;
    write_json(&plugin.join("hooks"), "hooks.json", &hooks(&hook))?;
    write_json(&plugin, ".mcp.json", &tool_servers(&tools))?;

    Ok(vec![ShellEntry::PathList {
        name: OsString::from(PLUGIN_DIRS),
        path: plugin,
    }])
}

/// What `plugin.json` says.
fn manifest() -> Value {
    json!({
        "name": PLUGIN,
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Tells the app what Claude Code does, so that the app can show the \
                        changes of a turn for review.",
    })
}

/// What `hooks.json` says: one group with no matcher for each hook, running `command`.
fn hooks(command: &str) -> Value {
    let group = json!([{"hooks": [{"type": "command", "command": command}]}]);
    let hooks: serde_json::Map<String, Value> = HOOK_EVENTS
        .iter()
        .map(|event| ((*event).to_owned(), group.clone()))
        .collect();
    json!({ "hooks": hooks })
}

/// What `.mcp.json` says: the tool server, started by Claude Code as `command` itself, with
/// no shell and so no quoting.
fn tool_servers(command: &str) -> Value {
    json!({"mcpServers": {TOOL_SERVER_NAME: {"command": command, "args": []}}})
}

/// The command that runs the hook program: `exec` and its absolute path, which holds
/// spaces on a Mac, quoted for `sh` as one word and nothing more. `exec` makes every `sh`
/// run the program in its own place: a plain command is left in a `sh` of its own by dash,
/// the `sh` of Ubuntu, and that short-lived parent would be taken for the agent.
fn hook_program(companions: &Path) -> io::Result<String> {
    let text = program_path(companions, HOOK_PROGRAM)?;
    Ok(format!("exec '{}'", text.replace('\'', r"'\''")))
}

/// The absolute path of the program `name` in the folder of the app's programs, as text,
/// which the plugin's files are made of.
fn program_path(companions: &Path, name: &str) -> io::Result<String> {
    let path: PathBuf = std::path::absolute(companions.join(name))?;
    path.to_str().map(str::to_owned).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the path {} is not text, which the plugin's files are made of",
                path.display()
            ),
        )
    })
}

/// Writes `value` as `folder/name`, through a temporary file of the folder that is renamed
/// into place, so that Claude Code never reads half a file.
fn write_json(folder: &Path, name: &str, value: &Value) -> io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let mut text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    text.push('\n');

    let temporary = folder.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, folder.join(name)));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}
