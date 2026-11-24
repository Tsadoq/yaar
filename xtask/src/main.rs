//! Build chores, run as `cargo xtask <chore>`: `layout` checks the rules between the crates;
//! `language-servers <folder>` fills the folder with the real language servers the tests need.

mod layout;
mod servers;

use std::path::Path;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["layout"] => check_layout(),
        ["language-servers", folder] => servers::fill(Path::new(folder)),
        _ => {
            eprintln!("usage: cargo xtask layout | language-servers <folder>");
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
        // Features of the programs as built for a user: no test support.
        let features = cargo(&[
            "tree",
            "--locked",
            "-p",
            "yaar-window",
            "-p",
            "yaar-keeper",
            "-p",
            "yaar-companion-claude",
            "-p",
            "yaar-adapter-pi",
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
