//! The login shell's environment, taken once at start on a thread of its own: a Keeper started
//! from the Finder has a short search path, which the user's git hooks would run with. Until
//! the take ends, reads of git use the Keeper's own environment and the user's own git waits.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use yaar_link::login::{self, ShellGroup};
use yaar_recorder::GitEnvironment;

use crate::Options;

pub(crate) fn waiting_for(options: &Options) -> GitEnvironment {
    match &options.login_shell {
        Some(shell) => GitEnvironment::waiting(shell.deadline),
        None => GitEnvironment::own(),
    }
}

pub(crate) struct Take {
    group: ShellGroup,
    /// Set when the Keeper leaves: a shell killed for it is no fault to say.
    leaving: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

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
                let ends = EndsTheTake(environment);
                let environment = &ends.0;
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

/// Ends the take with none when dropped, if nothing ended it: a thread that panics cannot leave the
/// user's git waiting for the whole deadline.
struct EndsTheTake(GitEnvironment);

impl Drop for EndsTheTake {
    fn drop(&mut self) {
        self.0.ended(None);
    }
}

impl Take {
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
