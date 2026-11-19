//! System tests of opening a repository and reading its changes: the built Keeper, a
//! real repository, and the scripted client in place of the Window.

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

/// A repository checked out on a branch with changes of every sort.
struct Branch {
    repo: Repo,
    /// The commit of `main` that `feature` left.
    fork: String,
    /// The one commit of `feature`.
    tip: String,
}

/// A repository named `app`: on `main` a file `src/lib.rs`; then a branch `feature`
/// whose one commit changes that file and adds `src/new.rs`; and a file `notes.txt`
/// that is not committed. `feature` is checked out.
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

// CLIENT-01.
#[test]
fn a_scripted_client_opens_a_folder_and_receives_the_workspace_with_the_changes_of_its_branch() {
    let mut world = World::new();
    world.start_keeper();
    let branch = a_branch_with_changes(&world);
    let mut client = world.client();
    assert_eq!(client.workspaces(), vec![], "nothing is open yet");

    // The folder becomes a workspace, and the client is told the list that holds it.
    let (repository, workspace) = client
        .open_folder(branch.repo.path())
        .expect("open the folder");
    let list = vec![Repository {
        id: repository,
        name: "app".to_owned(),
        workspaces: vec![Workspace {
            id: workspace.clone(),
            path: branch.repo.path().to_owned(),
            head: Head::Branch("feature".to_owned()),
        }],
    }];
    assert_eq!(
        client.wait_for_fact(DEADLINE, |fact| matches!(fact, Fact::Workspaces(_))),
        Fact::Workspaces(list.clone())
    );
    assert_eq!(client.workspaces(), list);

    // The changes of its branch: since the point where it left `main`, what is committed
    // on it and what is not.
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

    // And their content: both versions of the changed file, and each new file with no
    // old version.
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

/// The sockets among what a process holds open that something outside it could reach.
///
/// A socket of macOS's system domain is left out: it leads to the kernel of the same
/// machine and nothing can connect to it. The system's libraries open such sockets on
/// their own, not on every machine and not at every moment, so a count that held them
/// would say nothing of the Keeper. A socket of any other kind stays in, a network port
/// first of all.
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

/// The two numbers in the answer to `echo job-$! shell-$$`: the process of the job and
/// the shell's own. Only digits count, so the terminal's echo of the typed line, which
/// holds `$!` and `$$`, is never taken for the answer, and a number counts only once the
/// line that holds it has ended.
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

/// A background job of a shell in a terminal. No hang-up of the terminal reaches it, so
/// a test that fails with it running would leave it for the time of its sleep: it is
/// ended when the guard is dropped, and by [`Job::kill`] when the test is done with it.
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

/// Whether the process is gone: not running, and not one that ended and waits to be
/// collected.
fn is_gone(pid: u32) -> bool {
    !is_alive(pid)
}

// CLIENT-03.
#[test]
fn the_keeper_and_everything_it_starts_listen_on_the_users_socket_file_only() {
    let mut world = World::new();
    // A login `/bin/sh` on macOS closes the descriptors it inherits, so a socket that
    // leaked from the Keeper into a terminal's programs would never reach `cat` or
    // `sleep` and the listing below could not see it. The terminals of this world run a
    // shell that hands them on.
    world.keep_descriptors_in_terminals();
    let keeper = world.start_keeper();
    let branch = a_branch_with_changes(&world);
    let mut client = world.client();
    let (_, workspace) = client
        .open_folder(branch.repo.path())
        .expect("open the folder");
    let changes = client.changes(&workspace);
    // The old version of a file is read through a git process that the Keeper keeps
    // running: everything it starts so far.
    client
        .file(&changes, &file("src/lib.rs", FileStatus::Modified))
        .expect("read the file");

    // A terminal, its shell run in the world as `/bin/sh`. The shell starts a job that
    // outlives its hang-up and then becomes `cat`, so that the programs of the terminal
    // are a shell-turned-`cat` and a `sleep`, both started by the Keeper's Dock.
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
    // The shell's own process becomes `cat`: `exec` keeps the pid. The process that
    // prints the line is the one that turns into `cat`, so `cat` runs once the line is
    // there. The echo of the typed line shows `$((1+1))`, never `2`.
    client.type_into(&terminal, b"exec sh -c 'echo ready-$((1+1)); exec cat'\r");
    client.wait_for_output(&terminal, DEADLINE, |bytes| {
        bytes.windows(7).any(|window| window == b"ready-2")
    });

    // One listing of every descriptor of the Keeper, with a Window attached. It holds
    // two sockets, both of the unix kind and at the socket file of the world: the one it
    // listens on and the connection of this client. That they show is the proof that
    // the listing sees the process. No socket is of another kind: no network port. Its
    // pseudo-terminal masters show, and are no sockets.
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

    // That file is the user's own: a socket that belongs to the user of this test and
    // that nobody else may connect to.
    let socket_file = std::fs::symlink_metadata(world.socket()).expect("the socket file");
    assert!(socket_file.file_type().is_socket());
    // SAFETY: `geteuid` only reads and cannot fail.
    assert_eq!(socket_file.uid(), unsafe { libc::geteuid() });
    assert_eq!(socket_file.mode() & 0o077, 0, "{:o}", socket_file.mode());

    // The same for every process that descends from the Keeper: the git process, `cat`
    // (the shell it replaced) and the `sleep`, three at least. None holds a socket of
    // any kind. The pipes through which git talks to the Keeper show that the listing
    // sees it, the terminal that the other two run on shows that it sees them.
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

    // With the Window gone, one socket is left, and it is the one that listens: where
    // the system says which socket listens it says so of this one, and everywhere a new
    // client reaches the same Keeper through it.
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
    // The git process and the programs of the terminal outlived the Window, and still
    // hold no socket.
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

    // A world that kills its Keeper ends `cat` by the hang-up of its terminal, but no
    // hang-up reaches the job, which a shell with job control put in a group of its
    // own. The test ends it, and leaves when both are gone.
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
    let (repository, workspace) = one
        .open_folder(branch.repo.path())
        .expect("open the folder");

    // What git does not know is refused with its name, and tells nobody anything.
    assert_eq!(
        one.set_base(&workspace, "no-such-branch"),
        Err(Refusal::UnknownRevision {
            revision: "no-such-branch".to_owned()
        })
    );
    // The commit checked out, by a name that moves: it is kept as the commit it names
    // now.
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

    // The Keeper is stopped and another is started in the same world.
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

    // It knows the repository and the workspace by the same ids, and the base.
    let mut client = world.client();
    let list = client.workspaces();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].id, repository);
    assert_eq!(list[0].workspaces.len(), 1, "{list:?}");
    assert_eq!(list[0].workspaces[0].id, workspace);
    assert_eq!(client.changes(&workspace), since_the_tip);
}
