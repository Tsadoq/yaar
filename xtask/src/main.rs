//! Build chores of the repository (`B-BUILD`). Started as `cargo xtask <chore>`.
//!
//! ```text
//! cargo xtask layout           check the rules between the crates of the workspace
//! cargo xtask coverage         check that every requirement has a test in the plan
//! cargo xtask coverage list    the same, then each requirement with its tests
//! ```

mod coverage;
mod layout;

use std::fs;
use std::path::{Component, Path};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["layout"] => check_layout(),
        ["coverage"] => check_coverage(false),
        ["coverage", "list"] => check_coverage(true),
        _ => {
            eprintln!("usage: cargo xtask layout | coverage [list]");
            ExitCode::from(2)
        }
    }
}

fn cargo(args: &[&str]) -> Result<String, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("cargo printed no UTF-8: {e}"))
}

fn check_layout() -> ExitCode {
    let run = || -> Result<Vec<String>, String> {
        let metadata = cargo(&["metadata", "--format-version", "1", "--locked"])?;
        let graph = layout::Graph::from_metadata(&metadata)?;
        let mut breaches = layout::check(&graph);
        // What the two programs are built from when built for a user: no test support.
        let features = cargo(&[
            "tree",
            "--locked",
            "-p",
            "yaar-window",
            "-p",
            "yaar-keeper",
            "-e",
            "normal,build",
            "--prefix",
            "none",
            "-f",
            "{p}|{f}",
        ])?;
        breaches.extend(layout::check_shipped_features(&features));
        Ok(breaches)
    };
    match run() {
        Ok(breaches) if breaches.is_empty() => {
            println!("layout: the five rules hold");
            ExitCode::SUCCESS
        }
        Ok(breaches) => {
            for breach in &breaches {
                eprintln!("layout: {breach}");
            }
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("layout: {e}");
            ExitCode::from(2)
        }
    }
}

fn check_coverage(list: bool) -> ExitCode {
    let run = || -> Result<coverage::Report, String> {
        // The plan is found from where this crate lives, so the chore runs from any folder.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("the xtask crate is in no folder")?;
        let read = |path: &str| {
            fs::read_to_string(root.join(path)).map_err(|e| format!("cannot read {path}: {e}"))
        };
        // Only files inside the repository: no `..`, no path from the root of the disk.
        let file = |path: &str| {
            let inside = Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_)));
            if inside {
                fs::read_to_string(root.join(path)).ok()
            } else {
                None
            }
        };
        coverage::check(
            &read(coverage::REQUIREMENTS)?,
            &read(coverage::TESTS)?,
            &read(coverage::SLICES)?,
            &file,
        )
    };
    match run() {
        Ok(report) => {
            if report.breaches.is_empty() {
                println!("coverage: {}", report.summary());
            }
            for breach in &report.breaches {
                eprintln!("coverage: {breach}");
            }
            if list {
                for line in &report.list {
                    println!("{line}");
                }
            }
            if report.breaches.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("coverage: {e}");
            ExitCode::from(2)
        }
    }
}
