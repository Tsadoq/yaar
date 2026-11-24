use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::PathBuf;
use std::time::Duration;

use yaar_link::{
    Base, BaseSource, ChangedFile, Changes, Commit, Comparison, Content, Fact, FileSides,
    FileStatus, Head, Message, Refusal, Repository, TerminalSize, Workspace, write_message,
};
use yaar_system_tests::{
    DEADLINE, Descriptor, Repo, World, attach, descendants_of, descriptors_of, is_alive, wait_until,
};

const ONE: &str = "pub fn number() -> u32 {\n    1\n}\n";
const TWO: &str = "pub fn number() -> u32 {\n    2\n}\n";
const NEW: &str = "pub const NEW: bool = true;\n";
const NOTES: &str = "not committed yet\n";

struct Branch {
    repo: Repo,
    fork: String,
    tip: String,
}

fn a_branch_with_changes(world: &World) -> Branch {
    let repo = world.repository("app");
    repo.write("src/lib.rs", ONE);
    let fork = repo.commit("Add the number");
    repo.git(&["switch", "--quiet", "-c", "feature"]);
    repo.write("src/lib.rs", TWO);
    repo.write("src/new.rs", NEW);
    let tip = repo.commit("Make the number two");
    repo.write("notes.txt", NOTES);
    Branch { repo, fork, tip }
}

fn file(path: &str, status: FileStatus) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        status,
    }
}

fn text(content: &str) -> Content {
    Content::Text(content.as_bytes().to_vec())
}

#[test]
fn a_scripted_client_opens_a_folder_and_receives_the_workspace_with_the_changes_of_its_branch() {
    let mut world = World::new();
    world.start_keeper();
    let branch = a_branch_with_changes(&world);
    let mut client = world.client();
    assert_eq!(client.workspaces(), vec![], "nothing is open yet");

    let (repository, workspace) = client
        .open_folder(branch.repo.path())
        .expect("open the folder");
    let list = vec![Repository {
        id: repository,
        name: "app".to_owned(),
        workspaces: vec![Workspace {
            id: workspace.clone(),
            path: branch.repo.path().to_owned(),
            head: Some(Head::Branch("feature".to_owned())),
        }],
    }];
    assert_eq!(
        client.wait_for_fact(DEADLINE, |fact| matches!(fact, Fact::Workspaces(_))),
        Fact::Workspaces(list.clone())
    );
    assert_eq!(client.workspaces(), list);

    let changes = client.changes(&workspace);
    assert_eq!(
        changes,
        Changes {
            workspace,
            comparison: Comparison::SinceBase {
                base: Base {
                    name: "main".to_owned(),
                    source: BaseSource::Default,
                },
            },
            counted_from: Some(branch.fork),
            commits: vec![Commit {
                id: branch.tip,
                summary: "Make the number two".to_owned(),
            }],
            files: vec![
                file("notes.txt", FileStatus::Added),
                file("src/lib.rs", FileStatus::Modified),
                file("src/new.rs", FileStatus::Added),
            ],
        }
    );

    let [notes, changed, added] = &changes.files[..] else {
        unreachable!("three files, compared above");
    };
    assert_eq!(
        client.file(&changes, changed),
        Ok(FileSides {
            old: text(ONE),
            new: text(TWO),
        })
    );
    assert_eq!(
        client.file(&changes, added),
        Ok(FileSides {
            old: Content::Absent,
            new: text(NEW),
        })
    );
    assert_eq!(
        client.file(&changes, notes),
        Ok(FileSides {
            old: Content::Absent,
            new: text(NOTES),
        })
    );
}

/// The sockets a process holds that something outside it could reach. A macOS system-domain socket
/// is left out: the system's libraries open such sockets on some machines only.
fn sockets_of(pid: u32) -> Vec<Descriptor> {
    descriptors_of(pid)
        .into_iter()
        .filter(|held| {
            !matches!(
                held,
                Descriptor::Pipe
                    | Descriptor::Terminal
                    | Descriptor::File
                    | Descriptor::KernelSocket(_)
            )
        })
        .collect()
}

/// Only digits count, so the echo of the typed line (which holds `$!` and `$$`) is not taken for
/// the answer.
fn job_and_shell(output: &[u8]) -> Option<(u32, u32)> {
    let text = String::from_utf8_lossy(output);
    text.match_indices("job-").find_map(|(at, _)| {
        let line = text[at + "job-".len()..]
            .split_inclusive(['\r', '\n'])
            .next()?;
        let line = line.strip_suffix(['\r', '\n'])?;
        let (job, shell) = line.split_once(" shell-")?;
        Some((job.parse().ok()?, shell.parse().ok()?))
    })
}

struct Job(Option<u32>);

impl Job {
    fn kill(&mut self) {
        if let Some(pid) = self.0.take() {
            // SAFETY: `kill` only sends a signal, to a process this test started.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.kill();
    }
}

fn is_gone(pid: u32) -> bool {
    !is_alive(pid)
}

#[test]
fn the_keeper_and_everything_it_starts_listen_on_the_users_socket_file_only() {
    let mut world = World::new();
    // A login `/bin/sh` on macOS closes inherited descriptors; this world's terminals run a shell
    // that hands them on.
    world.keep_descriptors_in_terminals();
    let keeper = world.start_keeper();
    let branch = a_branch_with_changes(&world);
    let mut client = world.client();
    let (_, workspace) = client
        .open_folder(branch.repo.path())
        .expect("open the folder");
    let changes = client.changes(&workspace);
    client
        .file(&changes, &file("src/lib.rs", FileStatus::Modified))
        .expect("read the file");

    let terminal = client
        .open_terminal(
            &workspace,
            TerminalSize {
                columns: 80,
                rows: 24,
            },
        )
        .expect("open a terminal");
    client.wait_for_output(&terminal, DEADLINE, |bytes| !bytes.is_empty());
    client.type_into(&terminal, b"sleep 300 & echo job-$! shell-$$\r");
    let said = client.wait_for_output(&terminal, DEADLINE, |bytes| job_and_shell(bytes).is_some());
    let (job, shell) = job_and_shell(&said).expect("the job and the shell");
    let mut sleeping = Job(Some(job));
    // The echo of the typed line shows `$((1+1))`, never `2`.
    client.type_into(&terminal, b"exec sh -c 'echo ready-$((1+1)); exec cat'\r");
    client.wait_for_output(&terminal, DEADLINE, |bytes| {
        bytes.windows(7).any(|window| window == b"ready-2")
    });

    let at_the_socket_file = |held: &Descriptor| matches!(held, Descriptor::UnixSocket { path: Some(path), .. } if *path == world.socket());
    let listens = |held: &&Descriptor| {
        matches!(
            held,
            Descriptor::UnixSocket {
                listens: Some(true),
                ..
            }
        )
    };
    let attached = sockets_of(keeper);
    assert_eq!(attached.len(), 2, "{attached:?}");
    assert!(attached.iter().all(at_the_socket_file), "{attached:?}");
    assert!(attached.iter().filter(listens).count() <= 1, "{attached:?}");
    let own = descriptors_of(keeper);
    assert!(own.contains(&Descriptor::Terminal), "{own:?}");

    let socket_file = std::fs::symlink_metadata(world.socket()).expect("the socket file");
    assert!(socket_file.file_type().is_socket());
    // SAFETY: `geteuid` only reads and cannot fail.
    assert_eq!(socket_file.uid(), unsafe { libc::geteuid() });
    assert_eq!(socket_file.mode() & 0o077, 0, "{:o}", socket_file.mode());

    let started = descendants_of(keeper);
    assert!(started.len() >= 3, "{started:?}");
    assert!(started.contains(&shell), "{shell} not in {started:?}");
    assert!(started.contains(&job), "{job} not in {started:?}");
    for pid in started {
        let held = descriptors_of(pid);
        if pid == shell || pid == job {
            assert!(held.contains(&Descriptor::Terminal), "{pid}: {held:?}");
        } else {
            assert!(held.contains(&Descriptor::Pipe), "{pid}: {held:?}");
        }
        assert_eq!(sockets_of(pid), vec![], "{pid}: {held:?}");
    }

    drop(client);
    assert!(
        wait_until(DEADLINE, || sockets_of(keeper).len() == 1),
        "{:?}",
        sockets_of(keeper)
    );
    let left = sockets_of(keeper);
    let [only] = &left[..] else {
        panic!("not one socket: {left:?}");
    };
    assert!(at_the_socket_file(only), "{only:?}");
    assert!(
        !matches!(
            only,
            Descriptor::UnixSocket {
                listens: Some(false),
                ..
            }
        ),
        "{only:?}"
    );
    assert_eq!(world.client().keeper().pid, keeper);
    assert!(
        is_alive(shell),
        "the terminal's program left with the Window"
    );
    assert!(is_alive(job), "the job left with the Window");
    let started = descendants_of(keeper);
    assert!(started.len() >= 3, "{started:?}");
    for pid in started {
        assert_eq!(sockets_of(pid), vec![], "{pid}");
    }

    // Hang-up of the terminal ends `cat` but not the job, which job control put in a group of its
    // own.
    sleeping.kill();
    drop(world);
    assert!(
        wait_until(DEADLINE, || is_gone(job) && is_gone(shell)),
        "the job ({job}) or the shell ({shell}) is still there"
    );
}

#[test]
fn a_base_set_by_one_window_is_told_to_the_other_and_is_still_set_after_the_keeper_restarts() {
    let mut world = World::new();
    let first_keeper = world.start_keeper();
    let branch = a_branch_with_changes(&world);
    let mut one = world.client();
    let mut other = world.client();
    // A Window is told every fact from its first reply on, so `other` asks first.
    other.clis().expect("the Keeper serves the Window");
    let (repository, workspace) = one
        .open_folder(branch.repo.path())
        .expect("open the folder");

    assert_eq!(
        one.set_base(&workspace, "no-such-branch"),
        Err(Refusal::UnknownRevision {
            revision: "no-such-branch".to_owned()
        })
    );
    // HEAD is a name that moves: it is kept as the commit it names now.
    assert_eq!(one.set_base(&workspace, "HEAD"), Ok(()));
    let changed = Fact::ChangesChanged {
        workspace: workspace.clone(),
    };
    assert_eq!(
        other.wait_for_fact(DEADLINE, |fact| matches!(fact, Fact::ChangesChanged { .. })),
        changed
    );
    let since_the_tip = Changes {
        workspace: workspace.clone(),
        comparison: Comparison::SinceBase {
            base: Base {
                name: branch.tip.clone(),
                source: BaseSource::User,
            },
        },
        counted_from: Some(branch.tip),
        commits: Vec::new(),
        files: vec![file("notes.txt", FileStatus::Added)],
    };
    assert_eq!(other.changes(&workspace), since_the_tip);

    drop((one, other));
    let (mut window, _) = attach(&world.socket());
    write_message(&mut window, &Message::Shutdown).expect("ask for the stop");
    assert!(
        wait_until(Duration::from_secs(5), || world.keeper_pid().is_none()),
        "the Keeper did not stop"
    );
    assert!(
        world.data_dir().join("store.db").is_file(),
        "the database is not in the app's folder of the world"
    );
    assert_ne!(world.start_keeper(), first_keeper);

    let mut client = world.client();
    let list = client.workspaces();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].id, repository);
    assert_eq!(list[0].workspaces.len(), 1, "{list:?}");
    assert_eq!(list[0].workspaces[0].id, workspace);
    assert_eq!(client.changes(&workspace), since_the_tip);
}
