//! Stand-in agent CLI, started as `claude`, `pi` or `otheragent`; see `yaar_system_tests::standin`.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args();
    let called = args.next().unwrap_or_default();
    let name = Path::new(&called)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rest: Vec<String> = args.collect();
    let status = yaar_system_tests::standin::run(&name, &rest);
    ExitCode::from(u8::try_from(status).unwrap_or(1))
}
