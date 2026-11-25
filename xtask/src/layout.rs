//! Rules: 1. No `*-pane` crate depends on another `*-pane` crate. 2. No Keeper crate, shared crate
//! or companion has GPUI among its dependencies. 3. No Window crate depends on a Keeper crate, nor
//! the reverse. 4. Only `yaar-keeper` depends on an Adapter, and an Adapter depends on shared
//! crates only. 5. The programs the packages ship hold no GPUI test support and no `harness`
//! feature. 6. `yaar-ui` depends on no crate of the workspace. "Depends" means through normal and
//! build dependencies, direct or not.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Shared,
    Keeper,
    Window,
    Companion,
    Tooling,
}

#[derive(Debug, Default)]
pub struct Graph {
    members: BTreeMap<String, Group>,
    needs: BTreeMap<String, BTreeSet<String>>,
}

impl Graph {
    pub fn from_metadata(json: &str) -> Result<Self, String> {
        let meta: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let root = meta["workspace_root"].as_str().ok_or("no workspace_root")?;
        let member_ids: BTreeSet<&str> = meta["workspace_members"]
            .as_array()
            .ok_or("no workspace_members")?
            .iter()
            .filter_map(Value::as_str)
            .collect();

        let mut graph = Graph::default();
        let mut name_of = BTreeMap::new();
        for package in meta["packages"].as_array().ok_or("no packages")? {
            let id = package["id"].as_str().ok_or("a package has no id")?;
            let name = package["name"].as_str().ok_or("a package has no name")?;
            name_of.insert(id, name);
            if member_ids.contains(id) {
                let manifest = package["manifest_path"].as_str().unwrap_or_default();
                let relative = manifest
                    .strip_prefix(root)
                    .unwrap_or(manifest)
                    .trim_start_matches('/');
                graph.members.insert(name.to_owned(), group_of(relative));
            }
        }
        for node in meta["resolve"]["nodes"].as_array().ok_or("no resolve")? {
            let id = node["id"].as_str().ok_or("a node has no id")?;
            let from = *name_of.get(id).ok_or("a node names no package")?;
            let needs = graph.needs.entry(from.to_owned()).or_default();
            for dep in node["deps"].as_array().into_iter().flatten() {
                let outside_tests = dep["dep_kinds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|kind| kind["kind"].as_str() != Some("dev"));
                if let (true, Some(to)) = (
                    outside_tests,
                    dep["pkg"].as_str().and_then(|p| name_of.get(p)),
                ) {
                    needs.insert((*to).to_owned());
                }
            }
        }
        Ok(graph)
    }

    fn closure(&self, name: &str) -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<&str> = vec![name];
        while let Some(next) = todo.pop() {
            for dep in self.needs.get(next).into_iter().flatten() {
                if seen.insert(dep.as_str()) {
                    todo.push(dep);
                }
            }
        }
        seen
    }
}

fn group_of(manifest: &str) -> Group {
    if manifest.starts_with("crates/shared/") {
        Group::Shared
    } else if manifest.starts_with("crates/keeper/") {
        Group::Keeper
    } else if manifest.starts_with("crates/window/") {
        Group::Window
    } else if manifest.starts_with("companions/") {
        Group::Companion
    } else {
        Group::Tooling
    }
}

fn is_pane(name: &str) -> bool {
    name.ends_with("-pane")
}

fn is_adapter(name: &str) -> bool {
    name.starts_with("yaar-adapter-")
}

fn is_gpui(name: &str) -> bool {
    name.starts_with("gpui")
}

pub fn check(graph: &Graph) -> Vec<String> {
    let mut breaches = Vec::new();
    for (name, group) in &graph.members {
        let needs = graph.closure(name);

        if is_pane(name) {
            for other in needs.iter().filter(|n| is_pane(n)) {
                breaches.push(format!(
                    "rule 1: the pane {name} depends on the pane {other}"
                ));
            }
        }

        if matches!(group, Group::Shared | Group::Keeper | Group::Companion)
            && let Some(gpui) = needs.iter().find(|n| is_gpui(n))
        {
            breaches.push(format!(
                "rule 2: {name} is outside the Window and depends on {gpui}"
            ));
        }

        let other_side = match group {
            Group::Window => Some(Group::Keeper),
            Group::Keeper => Some(Group::Window),
            _ => None,
        };
        for other in &needs {
            if graph.members.get(*other).copied() == other_side && other_side.is_some() {
                breaches.push(format!(
                    "rule 3: {name} and {other} are on different sides of the Link"
                ));
            }
        }

        if name == "yaar-ui" {
            for other in needs.iter().filter(|n| graph.members.contains_key(**n)) {
                breaches.push(format!("rule 6: yaar-ui depends on {other}"));
            }
        }

        if name != "yaar-keeper" {
            for adapter in needs.iter().filter(|n| is_adapter(n)) {
                breaches.push(format!(
                    "rule 4: {name} depends on {adapter}; only yaar-keeper may"
                ));
            }
        }
        if is_adapter(name) {
            for other in &needs {
                if graph
                    .members
                    .get(*other)
                    .is_some_and(|g| *g != Group::Shared)
                {
                    breaches.push(format!(
                        "rule 4: the Adapter {name} depends on {other}, which is not a shared crate"
                    ));
                }
            }
        }
    }
    breaches
}

/// Rule 5, on the `name version (path)|feature,feature` lines of `cargo tree -e normal,build
/// --prefix none -f "{p}|{f}"`.
pub fn check_shipped_features(tree: &str) -> Vec<String> {
    let mut breaches = BTreeSet::new();
    for line in tree.lines() {
        let Some((package, features)) = line.split_once('|') else {
            continue;
        };
        let name = package.split_whitespace().next().unwrap_or_default();
        for feature in features.split(',').map(str::trim) {
            let forbidden = (is_gpui(name) && feature == "test-support")
                || (name.starts_with("yaar-") && feature == "harness");
            if forbidden {
                breaches.insert(format!(
                    "rule 5: the programs are built with {name}/{feature}"
                ));
            }
        }
    }
    breaches.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound() -> Graph {
        let mut graph = Graph::default();
        for (name, group) in [
            ("yaar-link", Group::Shared),
            ("yaar-agent", Group::Shared),
            ("yaar-keeper", Group::Keeper),
            ("yaar-dock", Group::Keeper),
            ("yaar-store", Group::Keeper),
            ("yaar-adapter-claude", Group::Keeper),
            ("yaar-window", Group::Window),
            ("yaar-frame", Group::Window),
            ("yaar-review-pane", Group::Window),
            ("yaar-terminal-pane", Group::Window),
            ("yaar-ui", Group::Window),
            ("yaar-companion-claude", Group::Companion),
        ] {
            graph.members.insert(name.to_owned(), group);
        }
        for (from, to) in [
            ("yaar-keeper", "yaar-link"),
            ("yaar-keeper", "yaar-dock"),
            ("yaar-keeper", "yaar-store"),
            ("yaar-keeper", "yaar-adapter-claude"),
            ("yaar-adapter-claude", "yaar-agent"),
            ("yaar-store", "rusqlite"),
            ("yaar-window", "yaar-frame"),
            ("yaar-window", "yaar-review-pane"),
            ("yaar-window", "yaar-terminal-pane"),
            ("yaar-review-pane", "yaar-frame"),
            ("yaar-terminal-pane", "yaar-frame"),
            ("yaar-frame", "yaar-link"),
            ("yaar-frame", "gpui-kit"),
            ("yaar-frame", "yaar-ui"),
            ("yaar-terminal-pane", "yaar-ui"),
            ("yaar-ui", "gpui-kit"),
            ("gpui-kit", "gpui-pre"),
        ] {
            plant(&mut graph, from, to);
        }
        graph
    }

    fn plant(graph: &mut Graph, from: &str, to: &str) {
        graph
            .needs
            .entry(from.to_owned())
            .or_default()
            .insert(to.to_owned());
    }

    fn breaches_of(from: &str, to: &str) -> Vec<String> {
        let mut graph = sound();
        plant(&mut graph, from, to);
        check(&graph)
    }

    #[test]
    fn a_sound_workspace_has_no_breach() {
        assert_eq!(check(&sound()), Vec::<String>::new());
    }

    #[test]
    fn a_pane_that_depends_on_another_pane_breaks_rule_1() {
        let found = breaches_of("yaar-review-pane", "yaar-terminal-pane");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("rule 1"), "{found:?}");
    }

    #[test]
    fn gpui_in_the_keeper_breaks_rule_2_also_through_another_crate() {
        let found = breaches_of("yaar-dock", "gpui-pre");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|b| b.starts_with("rule 2")), "{found:?}");
        assert!(
            breaches_of("yaar-link", "gpui-kit")
                .iter()
                .any(|b| b.starts_with("rule 2: yaar-link "))
        );
    }

    #[test]
    fn a_window_crate_that_reaches_the_store_breaks_rule_3() {
        let found = breaches_of("yaar-review-pane", "yaar-store");
        assert!(
            found
                .iter()
                .any(|b| b.starts_with("rule 3: yaar-review-pane ")),
            "{found:?}"
        );
        let reverse = breaches_of("yaar-dock", "yaar-frame");
        assert!(
            reverse.iter().any(|b| b.starts_with("rule 3: yaar-dock ")),
            "{reverse:?}"
        );
    }

    #[test]
    fn anyone_but_the_keeper_program_naming_an_adapter_breaks_rule_4() {
        let found = breaches_of("yaar-dock", "yaar-adapter-claude");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("rule 4: yaar-dock "), "{found:?}");
    }

    #[test]
    fn an_adapter_that_reaches_past_the_shared_crates_breaks_rule_4() {
        let found = breaches_of("yaar-adapter-claude", "yaar-store");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("rule 4: the Adapter "), "{found:?}");
    }

    #[test]
    fn a_window_crate_beneath_ui_is_a_breach() {
        let found = breaches_of("yaar-ui", "yaar-link");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("rule 6"), "{found:?}");
        let through = breaches_of("yaar-ui", "yaar-keeper");
        assert!(
            through
                .iter()
                .any(|b| b == "rule 6: yaar-ui depends on yaar-dock"),
            "{through:?}"
        );
    }

    #[test]
    fn test_support_in_a_shipped_program_breaks_rule_5() {
        let clean = "yaar-window v0.1.0 (/r/crates/window/window)|\n\
                     gpui-kit v0.9.1|assets,component,default\n\
                     gpui-pre v0.3.12|default,font-kit\n";
        assert_eq!(check_shipped_features(clean), Vec::<String>::new());

        let dirty = "gpui-kit v0.9.1|assets,test-support\n\
                     gpui-pre v0.3.12|test-support\n\
                     yaar-frame v0.1.0 (/r/crates/window/frame)|harness\n\
                     some-crate v1.0.0|test-support\n";
        assert_eq!(check_shipped_features(dirty).len(), 3);
    }

    #[test]
    fn what_only_tests_use_does_not_count() {
        let metadata = r#"{
            "workspace_root": "/r",
            "workspace_members": ["a", "b"],
            "packages": [
                {"id": "a", "name": "yaar-review-pane", "manifest_path": "/r/crates/window/review-pane/Cargo.toml"},
                {"id": "b", "name": "yaar-terminal-pane", "manifest_path": "/r/crates/window/terminal-pane/Cargo.toml"}
            ],
            "resolve": {"nodes": [
                {"id": "a", "deps": [{"pkg": "b", "dep_kinds": [{"kind": "dev"}]}]},
                {"id": "b", "deps": []}
            ]}
        }"#;
        let graph = Graph::from_metadata(metadata).expect("metadata");
        assert_eq!(check(&graph), Vec::<String>::new());

        let normal = metadata.replace(r#"{"kind": "dev"}"#, r#"{"kind": null}"#);
        let graph = Graph::from_metadata(&normal).expect("metadata");
        assert_eq!(check(&graph).len(), 1);
    }
}
