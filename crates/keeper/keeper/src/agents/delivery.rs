//! What a report waits for and the app's answer: the review riding with the user's message,
//! review-file reads, reply-tool calls and the agent's reply to a thread.

use std::time::{Duration, Instant};

use yaar_agent::{Answer, Asked, Asks};
use yaar_link::{AgentReport, Reply};
use yaar_review_engine::ThreadReply;
use yaar_store::{SentBy, SessionId, SessionRecord, ThreadId};
use yaar_switchboard::State;

use super::{Core, Told};
use crate::lock;
use crate::wiring::Kept;

/// The hook program gives up at 2 s and what it prints after goes nowhere: a later send would mark
/// comments sent that the agent never received.
const DELIVER_BY: Duration = Duration::from_millis(1500);

fn late(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) > DELIVER_BY
}

fn waited_too_long(at: Instant) -> Option<Duration> {
    let now = Instant::now();
    late(at, now).then(|| now.saturating_duration_since(at))
}

impl Core {
    pub(super) fn answer_asked(
        &self,
        kept: &Kept,
        report: &AgentReport,
        asked: &Asked,
        at: Instant,
        told: &mut Told,
    ) -> Reply {
        match &asked.asks {
            Asks::Message => self.deliver(kept, report, asked, at, told),
            Asks::ReadOwnFile | Asks::OwnTool => self.answer(report, asked, &Answer::Allow),
            Asks::Reply(reply) => {
                let Some(session) = self.session_of(kept, report, asked) else {
                    return self.answer(
                        report,
                        asked,
                        &Answer::Refused {
                            why: "the app knows no session of this agent",
                        },
                    );
                };
                let given = ThreadReply {
                    thread: ThreadId::new(reply.thread.as_str()),
                    text: reply.text.clone(),
                    addressed: reply.addressed,
                };
                match kept.reviews.agent_replied(&session.id, &given) {
                    Ok(_) => {
                        told.reviews_changed(&session.workspace);
                        self.answer(
                            report,
                            asked,
                            &Answer::Replied {
                                thread: &reply.thread,
                                addressed: reply.addressed,
                                characters: reply.text.chars().count(),
                            },
                        )
                    }
                    Err(e) => self.answer(
                        report,
                        asked,
                        &Answer::Refused {
                            why: &e.to_string(),
                        },
                    ),
                }
            }
        }
    }

    fn answer(&self, report: &AgentReport, asked: &Asked, answer: &Answer<'_>) -> Reply {
        let adapters = lock(&self.adapters);
        match adapters
            .iter()
            .find(|adapter| adapter.name() == report.agent)
        {
            Some(adapter) => Reply::Answer {
                bytes: adapter.answer(asked, answer),
            },
            None => Reply::Done,
        }
    }

    fn deliver(
        &self,
        kept: &Kept,
        report: &AgentReport,
        asked: &Asked,
        at: Instant,
        told: &mut Told,
    ) -> Reply {
        let Some(session) = self.session_of(kept, report, asked) else {
            return Reply::Done;
        };
        let review = match kept.reviews.review_of_session(&session.id) {
            Ok(Some(review)) => review,
            Ok(None) => return Reply::Done,
            Err(e) => {
                eprintln!("yaar-keeper: {}: {e}", report.agent);
                return Reply::Done;
            }
        };
        // One lock of the Adapters for the delivery and the send: the engine calls the delivery
        // while it holds its own turn.
        let adapters = lock(&self.adapters);
        let Some(adapter) = adapters
            .iter()
            .find(|adapter| adapter.name() == report.agent)
        else {
            return Reply::Done;
        };
        let Some(folder) = self.folder_of(&report.agent) else {
            return Reply::Done;
        };
        // Checked last, as near the send as it can be.
        if let Some(waited) = waited_too_long(at) {
            eprintln!(
                "yaar-keeper: {}: a message waited {waited:?} for its answer, more than {DELIVER_BY:?}: \
                 nothing was sent and the review stays attached",
                report.agent
            );
            return Reply::Done;
        }
        let mut delivery = adapter.delivery(folder, asked);
        match kept
            .reviews
            .send_attached(&review, SentBy::Delivered, &mut *delivery)
        {
            Ok(Some((bytes, sent))) => {
                self.round_sent_in(kept, &review, sent.round.number, &session);
                told.reviews_changed(&session.workspace);
                told.round_to_record(&session.workspace, &review, sent.round.number);
                Reply::Answer { bytes }
            }
            Ok(None) => Reply::Done,
            Err(e) => {
                eprintln!("yaar-keeper: {}: {e}", report.agent);
                Reply::Done
            }
        }
    }

    pub(crate) fn round_sent_in(
        &self,
        kept: &Kept,
        review: &yaar_store::ReviewId,
        round: u32,
        session: &SessionRecord,
    ) {
        let open = lock(&self.switchboard)
            .session(&yaar_switchboard::SessionId(session.id.as_str().to_owned()))
            .and_then(|known| known.turn);
        let Some((turn, _)) = open else {
            return;
        };
        if let Err(e) = kept.store.set_round_turn(review, round, &turn.0) {
            eprintln!("yaar-keeper: the turn of round {round} was not kept: {e}");
        }
    }

    /// The reply tool's server reads the CLI session id once and Claude Code keeps it across
    /// `/clear`, so a report can name a stale session: when the named one does not run or has no
    /// review, the running session of the same CLI with the report's process (`pid`) is meant.
    fn session_of(
        &self,
        kept: &Kept,
        report: &AgentReport,
        asked: &Asked,
    ) -> Option<SessionRecord> {
        let named = kept
            .store
            .session_of_cli(&report.agent, &asked.session.0)
            .unwrap_or_else(|e| {
                eprintln!("yaar-keeper: {}: {e}", report.agent);
                None
            });
        let running = lock(&self.switchboard)
            .sessions()
            .into_iter()
            .filter(|known| known.agent == report.agent && known.state != State::NotRunning)
            .collect::<Vec<_>>();
        if let Some(named) = &named
            && running.iter().any(|known| known.id.0 == named.id.as_str())
            && matches!(kept.reviews.review_of_session(&named.id), Ok(Some(_)))
        {
            return Some(named.clone());
        }
        let process = running
            .iter()
            .find(|known| known.pid == Some(report.pid) && known.cli != asked.session);
        match process {
            Some(known) => kept
                .store
                .session(&SessionId::new(known.id.0.clone()))
                .ok()
                .flatten()
                .or(named),
            None => named,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Weak};

    use yaar_system_tests::recorded;

    use super::*;
    use crate::Options;
    use crate::agents::Agents;

    #[test]
    fn a_report_is_late_once_it_has_waited_longer_than_the_limit_and_not_before() {
        let at = Instant::now();
        assert!(!late(at, at));
        assert!(!late(at, at + DELIVER_BY));
        assert!(late(at, at + DELIVER_BY + Duration::from_millis(1)));
        assert!(late(at, at + Duration::from_secs(10)));
    }

    #[test]
    fn the_wait_of_a_report_is_given_when_it_is_too_long_and_not_otherwise() {
        assert_eq!(waited_too_long(Instant::now()), None);
        let old = Instant::now()
            .checked_sub(DELIVER_BY * 2)
            .expect("the machine has been up for longer than three seconds");
        let waited = waited_too_long(old).expect("a report that waited twice the limit is late");
        assert!(waited >= DELIVER_BY * 2, "{waited:?}");
    }

    #[test]
    fn a_clock_that_reads_earlier_than_the_report_is_not_late() {
        let now = Instant::now();
        assert!(!late(now + Duration::from_secs(1), now));
    }

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(folder: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(folder)
            .stdout(Stdio::null())
            .status()
            .expect("run git");
        assert!(status.success(), "git {arguments:?}");
    }

    #[test]
    fn a_message_that_waited_too_long_is_sent_no_review_and_what_is_attached_stays_attached() {
        // `/tmp` is a link on macOS, and the Keeper names a worktree by its resolved path.
        let root = Path::new("/tmp")
            .canonicalize()
            .expect("the temporary folder")
            .join(format!("yaar-keeper-late-{}", std::process::id()));
        let _scratch = Scratch(root.clone());
        // SAFETY: nothing before this reads the environment, and nextest runs each test in a
        // process of its own, so no other thread can read it.
        unsafe {
            for (name, value) in [
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_AUTHOR_NAME", "A Test"),
                ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
                ("GIT_AUTHOR_DATE", "2026-01-01T12:00:00Z"),
                ("GIT_COMMITTER_NAME", "A Test"),
                ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
                ("GIT_COMMITTER_DATE", "2026-01-01T12:00:00Z"),
            ] {
                std::env::set_var(name, value);
            }
            std::env::set_var("HOME", root.join("home"));
            for name in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
                "GIT_CONFIG",
            ] {
                std::env::remove_var(name);
            }
        }
        let repo = root.join("repo");
        std::fs::create_dir_all(root.join("home")).expect("make the folders");
        std::fs::create_dir_all(&repo).expect("make the folders");
        git(&repo, &["init", "--quiet", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").expect("write a file");
        git(&repo, &["add", "--all"]);
        git(&repo, &["commit", "--quiet", "-m", "first"]);

        let options = Options {
            socket: root.join("k.sock"),
            linger: Duration::from_secs(60),
            app_version: "test-keeper".to_owned(),
            data_dir: root.join("data"),
            companions: Some(root.join("companions")),
            home: root.join("home"),
            login_shell: None,
        };
        let git_found = yaar_recorder::Git::find().expect("git is installed");
        let store = yaar_store::Store::open(&{
            std::fs::create_dir_all(&options.data_dir).expect("make the data folder");
            options.data_dir.join(yaar_store::FILE_NAME)
        })
        .expect("open the database");
        let kept = Kept {
            reviews: yaar_review_engine::Reviews::new(
                store.clone(),
                git_found.clone(),
                options.data_dir.join("states"),
            ),
            workspaces: yaar_workspaces::Workspaces::new(git_found, store.clone()),
            store,
        };
        let watching = Arc::new(crate::watching::Watching::new(Weak::new()));
        let agents = Agents::start(&options, Some(&kept), watching, Weak::new());
        let core = &agents.core;
        let workspace = kept
            .workspaces
            .open_folder(&repo)
            .expect("open the folder")
            .workspace;

        let events = recorded::hook_events("claude/review-file-read.jsonl");
        let report = |record: &recorded::HookRecord| AgentReport {
            agent: yaar_adapter_claude::NAME.to_owned(),
            terminal: None,
            pid: std::process::id(),
            payload: record.in_project(&repo).input_bytes(),
        };
        let mut told = Told::default();
        assert_eq!(
            core.report_taken(&kept, &report(&events[0]), Instant::now(), &mut told),
            Reply::Done
        );
        let session = kept
            .store
            .session_of_cli(yaar_adapter_claude::NAME, events[0].session())
            .expect("read the session")
            .expect("the session started");
        let review = kept
            .reviews
            .review_of_session(&session.id)
            .expect("read the review")
            .expect("the session has a review");
        let worktree = kept
            .workspaces
            .worktree(&workspace)
            .expect("read the worktree")
            .expect("the worktree");
        let comment = yaar_review_engine::CommentId::new(format!("{:032x}", 2));
        kept.reviews
            .save_draft(
                Some(&worktree),
                &review,
                &yaar_review_engine::DraftSave {
                    thread: yaar_review_engine::ThreadId::new(format!("{:032x}", 1)),
                    comment: comment.clone(),
                    on: yaar_review_engine::DraftOn::NewThread(yaar_review_engine::NewAnchor {
                        tree: None,
                        path: PathBuf::from("a.txt"),
                        side: yaar_review_engine::Side::New,
                        first: 1,
                        last: 1,
                        text: "one".to_owned(),
                    }),
                    text: "Why is this `one`?".to_owned(),
                },
            )
            .expect("save the draft");
        kept.reviews
            .attach(
                &review,
                yaar_review_engine::Attachment::Review,
                &agents.sessions_now(&kept, &yaar_link::WorkspaceId::new(workspace.as_str())),
            )
            .expect("attach the review");

        let message = report(&events[2]);
        let late_by = Instant::now()
            .checked_sub(DELIVER_BY * 2)
            .expect("the machine has been up for longer than three seconds");
        let mut told = Told::default();
        let answer = core.report_taken(&kept, &message, late_by, &mut told);
        assert_eq!(answer, Reply::Done);
        assert!(told.reviews.is_empty() && told.rounds.is_empty());
        let held = kept.reviews.review(&review).expect("read the review");
        assert_eq!(
            held.attached(),
            Some(&yaar_review_engine::Attachment::Review)
        );
        assert!(held.rounds().is_empty(), "{:?}", held.rounds());
        assert_eq!(
            held.comment(&comment).map(|(_, comment)| comment.round),
            Some(None)
        );

        // The same message on time is sent the review. Its time is ahead of the clock, so no
        // machine is slow enough to make it late.
        let mut told = Told::default();
        let answer = core.report_taken(&kept, &message, Instant::now() + DELIVER_BY, &mut told);
        assert!(matches!(answer, Reply::Answer { .. }), "{answer:?}");
        let sent = kept.reviews.review(&review).expect("read the review");
        assert_eq!(sent.attached(), None);
        assert_eq!(sent.rounds().len(), 1);
    }

    #[test]
    fn the_limit_leaves_the_hook_program_time_to_print_what_it_is_answered() {
        assert!(DELIVER_BY + Duration::from_millis(500) <= Duration::from_secs(2));
    }
}
