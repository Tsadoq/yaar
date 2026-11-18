//! `yaar-review`: the Window program behind a command line.
//!
//! ```text
//! yaar-review                   open the window
//! yaar-review --version
//! yaar-review --keeper-status   find or start the Keeper as the window would, say who
//!                               answered, and leave without opening a window
//! ```

use std::process::ExitCode;

use yaar_window::VERSION;

fn main() -> ExitCode {
    yaar_frame::trace::mark("main");
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => {
            open_window();
            ExitCode::SUCCESS
        }
        ["--version"] => {
            println!("yaar-review {VERSION}");
            ExitCode::SUCCESS
        }
        ["--keeper-status"] => keeper_status(),
        _ => {
            eprintln!("usage: yaar-review [--version | --keeper-status]");
            ExitCode::from(2)
        }
    }
}

fn open_window() {
    gpui_kit::application().run(|cx| {
        yaar_window::init(cx);
        yaar_window::open(yaar_link::socket_path(), cx).expect("open the window");
    });
}

fn keeper_status() -> ExitCode {
    let socket = yaar_link::socket_path();
    match yaar_frame::keeper::attach(&socket, VERSION) {
        Ok(attached) => {
            let keeper = attached.keeper;
            // SAFETY: `getsid` only reads; an unknown pid gives -1.
            let session = unsafe { libc::getsid(keeper.pid as libc::pid_t) };
            println!(
                "keeper version={} protocol={} pid={} session={} started={} socket={}",
                keeper.app_version,
                keeper.protocol,
                keeper.pid,
                session,
                attached.started,
                socket.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("yaar-review: {e}");
            ExitCode::FAILURE
        }
    }
}
