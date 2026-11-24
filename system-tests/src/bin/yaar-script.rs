//! `yaar-script`: a scripted Window for driving the installed app. Test support, not shipped.
//!
//! ```text
//! yaar-script worktree-with-agent --repository <folder> --branch <new branch> --agent <claude|pi>
//! yaar-script stop
//! ```
//! Both take `--socket` and `--deadline <seconds>`. Exits 0, 1 on a refusal or a passed deadline, 2
//! on a bad command line or no Keeper.

use std::any::Any;
use std::io::ErrorKind;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use yaar_link::{Head, Reply, Repository, Request, TerminalSize, Workspace, WorkspaceAgents};
use yaar_system_tests::Client;

const USAGE: &str = "usage:
  yaar-script worktree-with-agent --repository <folder> --branch <new branch> --agent <claude|pi>
              [--socket <path>] [--deadline <seconds>]
  yaar-script stop [--socket <path>] [--deadline <seconds>]";

const WORKTREE_DEADLINE: Duration = Duration::from_secs(60);

const STOP_DEADLINE: Duration = Duration::from_secs(20);

const TERMINAL: TerminalSize = TerminalSize {
    columns: 80,
    rows: 24,
};

#[derive(Debug, PartialEq, Eq)]
enum Command {
    WorktreeWithAgent {
        repository: PathBuf,
        branch: String,
        agent: String,
        socket: Option<PathBuf>,
        deadline: Duration,
    },
    Stop {
        socket: Option<PathBuf>,
        deadline: Duration,
    },
}

struct Failure {
    status: u8,
    sentence: String,
}

impl Failure {
    fn unmet(sentence: String) -> Failure {
        Failure {
            status: 1,
            sentence,
        }
    }

    fn unreachable(sentence: String) -> Failure {
        Failure {
            status: 2,
            sentence,
        }
    }
}

fn parse(args: &[String]) -> Result<Command, String> {
    let Some((name, flags)) = args.split_first() else {
        return Err("no command given".to_owned());
    };
    let wants_worktree = match name.as_str() {
        "worktree-with-agent" => true,
        "stop" => false,
        other => return Err(format!("unknown command {other}")),
    };
    let mut repository = None;
    let mut branch = None;
    let mut agent = None;
    let mut socket = None;
    let mut deadline = None;
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        let taken = match flag.as_str() {
            "--repository" if wants_worktree => &mut repository,
            "--branch" if wants_worktree => &mut branch,
            "--agent" if wants_worktree => &mut agent,
            "--socket" => &mut socket,
            "--deadline" => &mut deadline,
            other => return Err(format!("{name} does not take {other}")),
        };
        let value = flags
            .next()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        if taken.replace(value.clone()).is_some() {
            return Err(format!("{flag} is given twice"));
        }
    }
    let socket = socket.map(PathBuf::from);
    let deadline = match deadline {
        Some(text) => match text.parse::<u64>() {
            Ok(seconds) if seconds > 0 => Duration::from_secs(seconds),
            _ => {
                return Err(format!(
                    "--deadline wants a whole number of seconds, not {text}"
                ));
            }
        },
        None if wants_worktree => WORKTREE_DEADLINE,
        None => STOP_DEADLINE,
    };
    if !wants_worktree {
        return Ok(Command::Stop { socket, deadline });
    }
    let need = |value: Option<String>, flag: &str| value.ok_or_else(|| format!("{flag} is needed"));
    let agent = need(agent, "--agent")?;
    if agent != "claude" && agent != "pi" {
        return Err(format!("--agent is claude or pi, not {agent}"));
    }
    Ok(Command::WorktreeWithAgent {
        repository: PathBuf::from(need(repository, "--repository")?),
        branch: need(branch, "--branch")?,
        agent,
        socket,
        deadline,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match parse(&args) {
        Ok(command) => command,
        Err(problem) => {
            eprintln!("yaar-script: {problem}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    // The panic hook is silenced: the scripted client panics with what it saw when a wait runs out,
    // and `waiting` turns that into a sentence, so the default report would only say it twice.
    std::panic::set_hook(Box::new(|_| {}));
    let done = catch_unwind(AssertUnwindSafe(|| run(command))).unwrap_or_else(|payload| {
        Err(Failure::unmet(format!(
            "the scripted client failed: {}",
            said(&*payload)
        )))
    });
    match done {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("yaar-script: {}", failure.sentence);
            ExitCode::from(failure.status)
        }
    }
}

fn run(command: Command) -> Result<String, Failure> {
    match command {
        Command::WorktreeWithAgent {
            repository,
            branch,
            agent,
            socket,
            deadline,
        } => worktree_with_agent(
            &socket.unwrap_or_else(yaar_link::socket_path),
            &repository,
            &branch,
            &agent,
            deadline,
        ),
        Command::Stop { socket, deadline } => {
            stop(&socket.unwrap_or_else(yaar_link::socket_path), deadline)
        }
    }
}

fn said(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .unwrap_or_else(|| "no reason given".to_owned())
}

fn waiting<T>(what: &str, call: impl FnOnce() -> T) -> Result<T, Failure> {
    catch_unwind(AssertUnwindSafe(call))
        .map_err(|payload| Failure::unmet(format!("{what} did not come: {}", said(&*payload))))
}

fn attach(socket: &Path) -> Result<Client, Failure> {
    let no_keeper = |why: String| {
        Failure::unreachable(format!("no Keeper listens at {}: {why}", socket.display()))
    };
    drop(yaar_link::connect(socket).map_err(|e| no_keeper(e.to_string()))?);
    catch_unwind(|| Client::attach(socket)).map_err(|payload| no_keeper(said(&*payload)))
}

fn ask(
    client: &mut Client,
    what: &str,
    request: Request,
    deadline: Duration,
) -> Result<Reply, Failure> {
    // The send panics too when a leaving Keeper has closed the connection between two requests.
    waiting(what, || {
        let id = client.send(request);
        client.wait_for_reply(id, deadline)
    })?
    .map_err(|refusal| Failure::unmet(format!("the Keeper refused: {refusal}")))
}

fn listed<'a>(
    repositories: &'a [Repository],
    folder: &Path,
) -> Option<(&'a Repository, &'a Workspace)> {
    repositories.iter().find_map(|repository| {
        repository
            .workspaces
            .iter()
            .find(|workspace| {
                workspace.path == folder
                    || workspace
                        .path
                        .canonicalize()
                        .is_ok_and(|path| path == folder)
            })
            .map(|workspace| (repository, workspace))
    })
}

fn workspaces(client: &mut Client, deadline: Duration) -> Result<Vec<Repository>, Failure> {
    match ask(
        client,
        "the list of workspaces",
        Request::ListWorkspaces,
        deadline,
    )? {
        Reply::Workspaces(repositories) => Ok(repositories),
        other => Err(Failure::unmet(format!(
            "asked for the workspaces, the Keeper answered {other:?}"
        ))),
    }
}

fn worktree_with_agent(
    socket: &Path,
    repository: &Path,
    branch: &str,
    agent: &str,
    deadline: Duration,
) -> Result<String, Failure> {
    let mut client = attach(socket)?;
    let folder = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.to_owned());

    let mut repositories = workspaces(&mut client, deadline)?;
    if listed(&repositories, &folder).is_none() {
        ask(
            &mut client,
            "the reply to opening the folder",
            Request::OpenFolder {
                path: folder.clone(),
            },
            deadline,
        )?;
        repositories = workspaces(&mut client, deadline)?;
    }
    let (repository_id, from) = match listed(&repositories, &folder) {
        Some((repository, workspace)) => match &workspace.head {
            Some(Head::Branch(name)) => (repository.id.clone(), name.clone()),
            _ => {
                return Err(Failure::unmet(format!(
                    "{} has no branch checked out, so no worktree can be made from it",
                    folder.display()
                )));
            }
        },
        None => {
            return Err(Failure::unmet(format!(
                "the Keeper lists no workspace at {} after opening it",
                folder.display()
            )));
        }
    };

    let reply = ask(
        &mut client,
        "the reply WorktreeCreated",
        Request::CreateWorktree {
            repository: repository_id,
            from,
            branch: branch.to_owned(),
            agent: agent.to_owned(),
            size: TERMINAL,
        },
        deadline,
    )?;
    let Reply::WorktreeCreated { workspace, .. } = reply else {
        return Err(Failure::unmet(format!(
            "asked for a worktree, the Keeper answered {reply:?}"
        )));
    };

    let running = |agents: &WorkspaceAgents| {
        agents
            .sessions
            .iter()
            .any(|session| session.agent == agent && session.terminal.is_some())
    };
    // The fact may have come while the reply was awaited. Asking first covers a session started
    // before this client was told of anything.
    let listing = match ask(
        &mut client,
        "the list of agent sessions",
        Request::ListAgents,
        deadline,
    )? {
        Reply::Agents(all) => all,
        other => {
            return Err(Failure::unmet(format!(
                "asked for the agents, the Keeper answered {other:?}"
            )));
        }
    };
    let agents = match listing
        .into_iter()
        .find(|agents| agents.workspace == workspace && running(agents))
    {
        Some(agents) => agents,
        None => waiting(
            &format!("the fact that a session of {agent} runs in the new workspace"),
            || client.wait_for_agents(&workspace, deadline, running),
        )?,
    };
    let session = agents
        .sessions
        .iter()
        .find(|session| session.agent == agent && session.terminal.is_some())
        .map(|session| session.id.clone())
        .ok_or_else(|| Failure::unmet("the session that ran is gone".to_owned()))?;

    let made = workspaces(&mut client, deadline)?;
    let path = made
        .iter()
        .flat_map(|repository| &repository.workspaces)
        .find(|candidate| candidate.id == workspace)
        .map(|candidate| candidate.path.clone())
        .ok_or_else(|| {
            Failure::unmet(format!(
                "the Keeper's list holds no workspace {workspace} after it made one"
            ))
        })?;
    Ok(format!(
        "worktree={} workspace={workspace} session={session}",
        path.display()
    ))
}

fn stop(socket: &Path, deadline: Duration) -> Result<String, Failure> {
    let mut client = attach(socket)?;
    waiting("sending the stop", || client.stop_everything())?;
    let end = Instant::now() + deadline;
    let mut last = "the Keeper still answers".to_owned();
    loop {
        match yaar_link::connect(socket) {
            Ok(stream) => drop(stream),
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
                client.vanish();
                return Ok("stopped".to_owned());
            }
            Err(e) => last = e.to_string(),
        }
        if Instant::now() >= end {
            return Err(Failure::unmet(format!(
                "the Keeper at {} still answered after {} seconds: {last}",
                socket.display(),
                deadline.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn a_worktree_command_takes_its_four_flags_and_the_default_deadline() {
        assert_eq!(
            parse(&line(
                "worktree-with-agent --repository /r --branch b --agent claude"
            )),
            Ok(Command::WorktreeWithAgent {
                repository: PathBuf::from("/r"),
                branch: "b".to_owned(),
                agent: "claude".to_owned(),
                socket: None,
                deadline: Duration::from_secs(60),
            })
        );
    }

    #[test]
    fn a_stop_command_takes_a_socket_and_a_deadline_of_its_own() {
        assert_eq!(
            parse(&line("stop --deadline 5 --socket /s")),
            Ok(Command::Stop {
                socket: Some(PathBuf::from("/s")),
                deadline: Duration::from_secs(5),
            })
        );
        assert_eq!(
            parse(&line("stop")),
            Ok(Command::Stop {
                socket: None,
                deadline: Duration::from_secs(20),
            })
        );
    }

    #[test]
    fn what_the_command_line_does_not_say_or_says_wrongly_is_refused() {
        for wrong in [
            "",
            "frobnicate",
            "worktree-with-agent --repository /r --branch b",
            "worktree-with-agent --repository /r --agent claude",
            "worktree-with-agent --repository /r --branch b --agent emacs",
            "worktree-with-agent --repository /r --branch b --agent claude --deadline 0",
            "worktree-with-agent --repository /r --branch b --agent claude --deadline soon",
            "worktree-with-agent --repository /r --repository /s --branch b --agent claude",
            "worktree-with-agent --repository",
            "stop --branch b",
            "stop now",
        ] {
            assert!(parse(&line(wrong)).is_err(), "{wrong:?} was taken");
        }
    }

    fn leaving_keeper(socket: &Path) -> std::thread::JoinHandle<()> {
        let listener = std::os::unix::net::UnixListener::bind(socket).expect("bind");
        std::thread::spawn(move || {
            let mine = yaar_link::Hello::for_this_process("a-keeper-that-leaves");
            // The first connection is the probe that asks whether a Keeper listens; the next is the
            // Window's.
            for stream in listener.incoming() {
                let mut stream = stream.expect("accept");
                if yaar_link::handshake(&mut stream, &mine).is_ok() {
                    return;
                }
            }
        })
    }

    #[test]
    fn a_keeper_that_closes_between_two_requests_is_a_refusal_with_a_sentence_not_a_panic() {
        let folder =
            std::env::temp_dir().join(format!("yaar-script-leaving-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("scratch folder");
        let socket = folder.join("keeper.sock");
        let keeper = leaving_keeper(&socket);
        let mut client = attach(&socket).unwrap_or_else(|f| panic!("attach: {}", f.sentence));
        keeper.join().expect("the Keeper's thread");

        let asked = ask(
            &mut client,
            "the list of workspaces",
            Request::ListWorkspaces,
            Duration::from_secs(2),
        );

        std::fs::remove_dir_all(&folder).expect("remove scratch");
        let failure = asked.expect_err("a closed Keeper answered");
        assert_eq!(failure.status, 1);
        assert!(
            failure.sentence.contains("the list of workspaces"),
            "the sentence does not name the request: {}",
            failure.sentence
        );
    }
}
