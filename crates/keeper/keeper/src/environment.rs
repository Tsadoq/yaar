//! The login shell's environment, taken once at the start on a thread of its own.
//!
//! A Keeper that a Window started from the Finder, the Dock or a desktop's menu has the
//! system's short search path, so the user's git would run the user's hooks without the
//! programs of their login path. [`start`] runs [`yaar_link::login::take`] with the Keeper's
//! own environment as the base and hands what it gives to the [`GitEnvironment`] every
//! `Git` of the Keeper holds, which is what [`waiting_for`] made.
//!
//! The thread starts after the socket is bound, so that no Window waits for it, and it
//! answers no Window: a read of git runs in the Keeper's own environment until the take has
//! ended, and the user's own git waits for it (`GitEnvironment`). A shell that gives no
//! environment is said in one line on the standard error,
//! `yaar-keeper: the login shell <path> gave no environment: <why>`, and the Keeper goes on
//! in its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use yaar_link::login::{self, ShellGroup};
use yaar_recorder::GitEnvironment;

use crate::Options;

/// The environment the Keeper's git runs in, before the take: waiting for it, for at most
/// the shell's deadline, when `options` name a shell, and the process's own for ever when
/// they name none.
pub(crate) fn waiting_for(options: &Options) -> GitEnvironment {
    match &options.login_shell {
        Some(shell) => GitEnvironment::waiting(shell.deadline),
        None => GitEnvironment::own(),
    }
}

/// A take under way.
pub(crate) struct Take {
    /// The shell's process group while it runs, which the end kills.
    group: ShellGroup,
    /// Set when the Keeper leaves: a shell killed for it is no fault to say.
    leaving: Arc<AtomicBool>,
    /// The thread that runs the shell; none when the options named no shell.
    thread: Option<JoinHandle<()>>,
}

/// Starts the take of `options.login_shell`, if there is one, on a thread of its own, and
/// ends `environment` with what it gave, or with none. Gives at once.
pub(crate) fn start(options: &Options, environment: GitEnvironment) -> Take {
    let group = ShellGroup::default();
    let leaving = Arc::new(AtomicBool::new(false));
    let Some(shell) = options.login_shell.clone() else {
        return Take {
            group,
            leaving,
            thread: None,
        };
    };
    let spawned = {
        let (group, leaving, environment) = (group.clone(), leaving.clone(), environment.clone());
        std::thread::Builder::new()
            .name("keeper-login-environment".to_owned())
            .spawn(move || {
                let own: Vec<_> = std::env::vars_os().collect();
                match login::take_in(&shell, &own, &group) {
                    Ok(merged) => environment.ended(Some(merged)),
                    Err(why) => {
                        if !leaving.load(Ordering::SeqCst) {
                            eprintln!(
                                "yaar-keeper: the login shell {} gave no environment: {why}",
                                shell.program.display()
                            );
                        }
                        environment.ended(None);
                    }
                }
            })
    };
    let thread = match spawned {
        Ok(thread) => Some(thread),
        Err(why) => {
            if let Some(shell) = &options.login_shell {
                eprintln!(
                    "yaar-keeper: the login shell {} gave no environment: {}",
                    shell.program.display(),
                    login::ShellError::Start(why.to_string())
                );
            }
            environment.ended(None);
            None
        }
    };
    Take {
        group,
        leaving,
        thread,
    }
}

impl Take {
    /// The Keeper leaves: a shell still running is killed, and the thread is waited for.
    /// What waits for the take is woken with none.
    pub(crate) fn end(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        self.leaving.store(true, Ordering::SeqCst);
        self.group.end();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Take {
    fn drop(&mut self) {
        self.finish();
    }
}
