//! `yaar-keeper [--socket <path>] | --version`: serves until asked to stop or until idle.
//! `YAAR_KEEPER_LINGER_MS` sets how long it stays with no Window attached.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use yaar_keeper::{Exit, Options, run};
use yaar_link::login::{LoginShell, SHELL_DEADLINE};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let mut options = Options::new(VERSION);
    options.login_shell = Some(LoginShell {
        program: yaar_link::user_shell(),
        deadline: SHELL_DEADLINE,
    });
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--version") => {
                println!("yaar-keeper {VERSION}");
                return ExitCode::SUCCESS;
            }
            Some("--socket") => match args.next() {
                Some(path) => options.socket = PathBuf::from(path),
                None => return usage("--socket needs a path"),
            },
            _ => return usage(&format!("unknown argument: {}", arg.to_string_lossy())),
        }
    }
    if let Some(ms) = std::env::var("YAAR_KEEPER_LINGER_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        options.linger = Duration::from_millis(ms);
    }

    match run(options) {
        Ok(Exit::Shutdown | Exit::Idle | Exit::AlreadyRunning) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("yaar-keeper: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage(problem: &str) -> ExitCode {
    eprintln!("yaar-keeper: {problem}\nusage: yaar-keeper [--socket <path>] | --version");
    ExitCode::from(2)
}
