//! One worktree, read through git.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::contents::{Contents, Read};
use crate::git::{Answer, Git, GitError, Place, finish, finish_with, unreadable};
use crate::index::IndexCopy;

/// Names one commit: its full id, in lowercase hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitId(String);

impl CommitId {
    /// The id with these digits.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The digits of the id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What is checked out in a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// A branch, by its short name. `commit` is none while the branch has no commit yet,
    /// as in a repository that was just made.
    Branch {
        /// The short name of the branch: `main`, `feature/login`.
        name: String,
        /// The commit the branch points at.
        commit: Option<CommitId>,
    },
    /// No branch: a commit.
    Detached(CommitId),
}

/// One commit, as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Its id.
    pub id: CommitId,
    /// The first line of its message.
    pub summary: String,
}

/// A worktree as its repository lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The folder of the worktree.
    pub path: PathBuf,
    /// What is checked out in it.
    pub head: Head,
    /// Whether it is the main worktree, the one the repository was made with.
    pub main: bool,
    /// The git folder of this worktree alone, as [`Worktree::git_dir`] gives it.
    pub git_dir: PathBuf,
}

/// A local branch, seen from what is checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBranch {
    /// Its short name: `main`.
    pub name: String,
    /// How many commits the checked-out commit has that this branch does not: the
    /// distance from the point where the two part to what is checked out.
    pub ahead: u32,
    /// The branch it follows, by its short name: `origin/main`. None also when the
    /// branch it was set to follow is no longer there.
    pub upstream: Option<String>,
}

/// A branch of a remote as the repository last saw it, seen from what is checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranch {
    /// Its short name, the remote first: `origin/main`.
    pub name: String,
    /// The remote it belongs to: `origin`.
    pub remote: String,
    /// How many commits the checked-out commit has that this branch does not.
    pub ahead: u32,
}

/// The branches around a worktree: everything the rule that finds the base needs, read
/// from the repository as it is, with no call to any server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Refs {
    /// The names of the remotes.
    pub remotes: Vec<String>,
    /// The default branch each remote recorded, by its short name, `origin/main`, in the
    /// order of `remotes`. Listed as recorded, whether or not that branch still exists.
    pub remote_defaults: Vec<String>,
    /// Every local branch, in git's order.
    pub local: Vec<LocalBranch>,
    /// Every branch of every remote, in git's order.
    pub remote: Vec<RemoteBranch>,
}

/// What happened to a path between a commit and the files on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The file is new. A file git does not track yet, and does not ignore, is new.
    Added,
    /// The content or the kind of the file changed.
    Modified,
    /// The file is gone.
    Deleted,
    /// git pairs the file with one that is gone, by its own rule for renames.
    Renamed {
        /// The path of the file that is gone.
        from: PathBuf,
    },
}

/// One path that differs between a commit and the files on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedPath {
    /// The path now, relative to the folder of the worktree.
    pub path: PathBuf,
    /// What happened to it.
    pub change: Change,
}

/// Names one branch as a branch, by its short name as [`Refs`] lists it.
///
/// git reads a short name as a tag before it reads it as a branch. Asked through this,
/// it is asked for the branch and for nothing else: a tag that happens to have the name
/// is not looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch<'a> {
    /// A local branch: `main`.
    Local(&'a str),
    /// A branch of a remote, the remote first: `origin/main`.
    Remote(&'a str),
}

impl Branch<'_> {
    /// The name git has for the branch in full. A name given in full is taken as it is,
    /// before any shorter reading of it.
    fn full_name(self) -> String {
        match self {
            Branch::Local(name) => format!("refs/heads/{name}"),
            Branch::Remote(name) => format!("refs/remotes/{name}"),
        }
    }
}

/// Which commits to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commits<'a> {
    /// Those reachable from what is checked out and not from this commit.
    Since(&'a CommitId),
    /// Those reachable from what is checked out and not from this branch.
    NotOn(Branch<'a>),
    /// Those reachable from what is checked out and from no branch of any remote, as the
    /// repository last saw the remotes.
    OnNoRemoteBranch,
}

/// Why the commit control made no commit.
#[derive(Debug)]
pub enum CommitError {
    /// Nothing is uncommitted: there is nothing to commit.
    NothingToCommit,
    /// git ended with an error: a hook rejected the commit, no identity is set, or the
    /// message was empty. Nothing was committed.
    Refused {
        /// What git wrote to its standard error, as it wrote it; when it wrote nothing
        /// there, what it wrote to its standard output, as git says some refusals, such
        /// as that there are no changes added to commit, only there.
        stderr: String,
    },
    /// git could not be run.
    Git(GitError),
}

impl CommitError {
    /// The refusal for a git that ended with an error: its error stream, or its output
    /// stream when the error stream says nothing.
    fn refused_by(answer: Answer) -> Self {
        let stderr = if answer.stderr.trim().is_empty() {
            String::from_utf8_lossy(&answer.stdout).into_owned()
        } else {
            answer.stderr
        };
        Self::Refused { stderr }
    }
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToCommit => write!(f, "nothing is uncommitted"),
            Self::Refused { stderr } => write!(f, "git made no commit: {}", stderr.trim()),
            Self::Git(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CommitError {}

impl From<GitError> for CommitError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// Why a folder could not be opened as a worktree.
#[derive(Debug)]
pub enum OpenError {
    /// There is no folder at this path, or it cannot be read.
    NoSuchFolder,
    /// The folder lies in no git repository and no worktree.
    NotARepository,
    /// The folder is a repository with no files checked out.
    Bare,
    /// git could not say.
    Git(GitError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchFolder => write!(f, "there is no such folder"),
            Self::NotARepository => write!(f, "the folder holds no git repository and no worktree"),
            Self::Bare => write!(f, "the repository has no files checked out"),
            Self::Git(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<GitError> for OpenError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

/// One worktree, read through git.
///
/// It holds no answer of git: every call asks again, so it always tells what is there
/// now. It may be used from several threads at once.
///
/// What it does keep is one running git process that hands out the contents of files in
/// commits, started at the first [`Worktree::content_at`] and asked again at each later
/// one, so that reading many files starts git once. The process ends, and is waited for,
/// when the `Worktree` is dropped. Whoever reads files of a worktree again and again
/// keeps its `Worktree`.
///
/// It also keeps which hooks the configuration defined when it was opened, to turn each
/// off by name in every run. [`Worktree::changed_paths`], the one call during which git
/// does what a hook can wait for, asks for them again, so that a hook defined since is
/// turned off too.
#[derive(Debug)]
pub struct Worktree {
    git: Git,
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
    // The setting that turns off each hook the configuration defined when this was
    // opened.
    hooks_off: Vec<OsString>,
    // The running process that hands out file contents, once one was asked for.
    contents: Mutex<Option<Contents>>,
}

impl Worktree {
    /// Opens the worktree that `folder` lies in: the folder itself or any folder above
    /// it, as git finds it. A folder inside the git folder of a repository lies in no
    /// worktree.
    pub fn open(git: &Git, folder: &Path) -> Result<Worktree, OpenError> {
        if !folder.is_dir() {
            return Err(OpenError::NoSuchFolder);
        }
        // git prints the answers in the order asked and stops at the first it cannot
        // give: where there is no worktree, that is the last one.
        let answer = git.run(
            Place::Folder(folder),
            &[],
            [
                "rev-parse",
                "--is-bare-repository",
                "--is-inside-work-tree",
                "--path-format=absolute",
                "--git-dir",
                "--git-common-dir",
                "--show-toplevel",
            ],
        )?;
        let printed = answer.stdout.clone();
        let mut lines = printed
            .strip_suffix(b"\n")
            .unwrap_or(&printed)
            .split(|b| *b == b'\n');
        let (bare, inside) = (lines.next(), lines.next());
        if answer.code != Some(0) {
            return Err(match (bare, inside) {
                (Some(b"true"), _) => OpenError::Bare,
                (Some(_), Some(b"false")) => OpenError::NotARepository,
                _ if answer.stderr.contains("not a git repository") => OpenError::NotARepository,
                _ => OpenError::Git(answer.failed()),
            });
        }
        let (Some(git_dir), Some(common_dir), Some(root), None) =
            (lines.next(), lines.next(), lines.next(), lines.next())
        else {
            return Err(answer
                .unreadable("not the three folders that were asked for")
                .into());
        };
        let resolved = |path: &[u8]| {
            fs::canonicalize(OsStr::from_bytes(path)).map_err(|e| OpenError::Git(e.into()))
        };
        let root = resolved(root)?;
        Ok(Worktree {
            git: git.clone(),
            hooks_off: hooks_off(git, Place::Folder(&root))?,
            root,
            git_dir: resolved(git_dir)?,
            common_dir: resolved(common_dir)?,
            contents: Mutex::new(None),
        })
    }

    /// The folder of the worktree, with every link in its path resolved.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The git folder of this worktree alone, with every link resolved: `.git` for the
    /// main worktree, a folder inside it for any other. git lists nothing that is put
    /// here, and moves the folder with the worktree.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// The git folder all worktrees of the repository share, with every link resolved.
    /// Two worktrees belong to one repository when this is the same.
    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    /// What is checked out.
    pub fn head(&self) -> Result<Head, GitError> {
        let symbolic = self.run(["symbolic-ref", "-q", "HEAD"])?;
        let commit = self.head_commit()?;
        match symbolic.code {
            Some(0) => {
                let name = text(&symbolic.stdout);
                let name = name.strip_prefix("refs/heads/").unwrap_or(&name);
                Ok(Head::Branch {
                    name: name.to_owned(),
                    commit,
                })
            }
            // With `-q` git says by its exit code alone that no branch is checked out.
            Some(1) => match commit {
                Some(commit) => Ok(Head::Detached(commit)),
                None => Err(symbolic.unreadable("neither a branch nor a commit is checked out")),
            },
            _ => Err(symbolic.failed()),
        }
    }

    /// Every worktree of the repository that has its files on disk, this one included:
    /// the main one first, then in git's order. A repository with no files checked out
    /// is not a worktree and is not listed; nor is a worktree whose folder is gone.
    pub fn list(&self) -> Result<Vec<Listed>, GitError> {
        let answer = self.run(["worktree", "list", "--porcelain", "-z"])?;
        Ok(listed(&answer.printed()?, &self.common_dir))
    }

    /// Every worktree of the repository whose shared git folder is `common_dir`, as
    /// [`Worktree::list`] gives them from inside one of them. None when there is no
    /// repository at that place any more.
    pub fn list_of(git: &Git, common_dir: &Path) -> Result<Vec<Listed>, GitError> {
        let Ok(common_dir) = fs::canonicalize(common_dir) else {
            return Ok(Vec::new());
        };
        let place = Place::GitDir(&common_dir);
        let answer = git.run(place, &[], ["rev-parse", "--git-dir"])?;
        if answer.code != Some(0) {
            return if answer.stderr.contains("not a git repository") {
                Ok(Vec::new())
            } else {
                Err(answer.failed())
            };
        }
        let hooks_off = hooks_off(git, place)?;
        let answer = git.run(place, &hooks_off, ["worktree", "list", "--porcelain", "-z"])?;
        Ok(listed(&answer.printed()?, &common_dir))
    }

    /// The branches around this worktree, each with its distance from what is checked
    /// out.
    pub fn refs(&self) -> Result<Refs, GitError> {
        let remotes: Vec<String> = text(&self.run(["remote"])?.printed()?)
            .lines()
            .map(str::to_owned)
            .collect();

        let mut remote_defaults = Vec::new();
        for remote in &remotes {
            let head = format!("refs/remotes/{remote}/HEAD");
            let answer = self.run(["symbolic-ref", "-q", &head])?;
            match answer.code {
                Some(0) => {
                    let target = text(&answer.stdout);
                    let short = target.strip_prefix("refs/remotes/").unwrap_or(&target);
                    remote_defaults.push(short.to_owned());
                }
                // The remote recorded no default branch.
                Some(1) => {}
                _ => return Err(answer.failed()),
            }
        }

        // The distance is counted from what is checked out, so it can only be asked
        // for where something is committed.
        const WITH_DISTANCE: &str = "--format=%(refname)%00%(upstream)%00%(ahead-behind:HEAD)";
        const WITHOUT: &str = "--format=%(refname)%00%(upstream)%00";
        let list = |format: &str| self.run(["for-each-ref", format, "refs/heads", "refs/remotes"]);
        let mut answer = list(WITH_DISTANCE)?;
        if answer.code != Some(0) && self.head_commit()?.is_none() {
            answer = list(WITHOUT)?;
        }
        let command = answer.command().to_owned();
        let printed = text(&answer.printed()?);
        let unreadable = |what: String| unreadable(&command, what);

        let mut rows = Vec::new();
        for line in printed.lines() {
            let mut fields = line.split('\0');
            let (Some(name), Some(upstream), Some(distance)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(unreadable(format!("the line {line:?}")));
            };
            // git prints the side of the branch first and that of what is checked out
            // second: the second number is how far ahead the checked-out commit is.
            let ahead = match distance.split_once(' ') {
                Some((_, ahead)) => ahead
                    .parse::<u32>()
                    .map_err(|_| unreadable(format!("the distance {distance:?}")))?,
                None => 0,
            };
            rows.push((name, upstream, ahead));
        }

        let mut refs = Refs {
            remotes,
            remote_defaults,
            ..Refs::default()
        };
        for &(name, upstream, ahead) in &rows {
            if let Some(short) = name.strip_prefix("refs/heads/") {
                // A branch can be set to follow one that is no longer there.
                let upstream = rows
                    .iter()
                    .any(|(other, _, _)| !upstream.is_empty() && *other == upstream)
                    .then(|| short_name(upstream).to_owned());
                refs.local.push(LocalBranch {
                    name: short.to_owned(),
                    ahead,
                    upstream,
                });
            } else if let Some(short) = name.strip_prefix("refs/remotes/") {
                // The longest name wins: a remote may be named `team/fork`.
                let remote = refs
                    .remotes
                    .iter()
                    .filter(|remote| {
                        short
                            .strip_prefix(remote.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                    })
                    .max_by_key(|remote| remote.len())
                    .cloned()
                    .unwrap_or_else(|| short.split('/').next().unwrap_or(short).to_owned());
                // `<remote>/HEAD` is the default the remote recorded, not a branch.
                if short != format!("{remote}/HEAD") {
                    refs.remote.push(RemoteBranch {
                        name: short.to_owned(),
                        remote,
                        ahead,
                    });
                }
            }
        }
        Ok(refs)
    }

    /// The commit a branch points at; none when there is no such branch.
    pub fn commit_of(&self, branch: Branch<'_>) -> Result<Option<CommitId>, GitError> {
        self.resolve(&branch.full_name())
    }

    /// The commit a branch, a tag or a commit id names, as the user would write it to
    /// git; none when git knows nothing by that name. Where a tag and a branch have the
    /// name, that is the tag, as it is for the user: [`Worktree::commit_of`] asks for a
    /// branch.
    pub fn resolve(&self, revision: &str) -> Result<Option<CommitId>, GitError> {
        // No argument of a program can hold a zero byte, and no name git knows does.
        if revision.contains('\0') {
            return Ok(None);
        }
        let commit = format!("{revision}^{{commit}}");
        let answer = self.run(["rev-parse", "-q", "--verify", "--end-of-options", &commit])?;
        match answer.code {
            Some(0) => Ok(Some(CommitId::new(text(&answer.stdout)))),
            Some(1) => Ok(None),
            _ => Err(answer.failed()),
        }
    }

    /// The point where what is checked out left `base`: the newest commit both have.
    /// None when the two share no commit, or when nothing is committed yet.
    pub fn fork_point(&self, base: &CommitId) -> Result<Option<CommitId>, GitError> {
        let answer = self.run(["merge-base", "--end-of-options", "HEAD", base.as_str()])?;
        match answer.code {
            Some(0) => Ok(Some(CommitId::new(text(&answer.stdout)))),
            Some(1) => Ok(None),
            _ if self.head_commit()?.is_none() => Ok(None),
            _ => Err(answer.failed()),
        }
    }

    /// The commits asked for, newest first.
    pub fn commits(&self, which: Commits<'_>) -> Result<Vec<Commit>, GitError> {
        // `^name` leaves out what `name` reaches, and cannot be taken for an option
        // whatever the name is.
        let leave_out = match which {
            Commits::Since(commit) => format!("^{commit}"),
            Commits::NotOn(branch) => format!("^{}", branch.full_name()),
            Commits::OnNoRemoteBranch => "--remotes".to_owned(),
        };
        let mut args = vec![
            "rev-list",
            "--no-commit-header",
            "--format=%H%x00%s",
            "HEAD",
        ];
        if which == Commits::OnNoRemoteBranch {
            args.push("--not");
        }
        args.push(&leave_out);
        let answer = self.run(args)?;
        if answer.code != Some(0) && self.head_commit()?.is_none() {
            return Ok(Vec::new());
        }
        let command = answer.command().to_owned();
        let printed = answer.printed()?;
        printed
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                let mut fields = line.splitn(2, |byte| *byte == 0);
                match (fields.next(), fields.next()) {
                    (Some(id), Some(summary)) => Ok(Commit {
                        id: CommitId::new(text(id)),
                        summary: String::from_utf8_lossy(summary).into_owned(),
                    }),
                    _ => Err(unreadable(
                        &command,
                        format!("the line {:?}", String::from_utf8_lossy(line)),
                    )),
                }
            })
            .collect()
    }

    /// Every path that differs between `since` and the files on disk, ordered by path
    /// as git orders paths, byte by byte: what was committed after `since`, what is
    /// changed and not committed, staged or not, and the files git does not track and
    /// does not ignore. With `since` none, as in a repository with no commit, every file
    /// on disk that git does not ignore is new.
    ///
    /// The paths and their pairing into renames are those of git's own diff between the
    /// commit and the worktree; the untracked files are those `git status` lists one by
    /// one. A repository inside the worktree is one path, the folder that holds it, and
    /// is not looked into.
    ///
    /// git works on a copy of its list of the files, made for this call: when it
    /// brings the list up to date, as its diff does, the user's own is neither written
    /// nor locked.
    pub fn changed_paths(&self, since: Option<&CommitId>) -> Result<Vec<ChangedPath>, GitError> {
        let index = IndexCopy::of(&self.git_dir.join("index"))?;
        // git writes the copy when it has brought it up to date, and a hook can wait
        // for that: the hooks are those defined now, not those of when this was opened.
        let hooks_off = hooks_off(&self.git, Place::Folder(&self.root))?;
        let run = |args: &[&str]| {
            let mut command = self.git.command(Place::Folder(&self.root), &hooks_off);
            command.env("GIT_INDEX_FILE", index.file()).args(args);
            finish(command)
        };

        // Keyed by the bytes of the path, which is the order git lists paths in.
        let mut changes = BTreeMap::new();
        if let Some(since) = since {
            let answer = run(&[
                "diff",
                "--name-status",
                "-z",
                "-M",
                "--no-ext-diff",
                "--end-of-options",
                since.as_str(),
                "--",
            ])?;
            let command = answer.command().to_owned();
            changes = name_status(&command, &answer.printed()?)?;
        }

        // With a commit to count from, the files of git's list are in its diff already;
        // with none, they are new like the untracked ones.
        let others = if since.is_some() {
            run(&["ls-files", "--others", "--exclude-standard", "-z"])?
        } else {
            run(&[
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ])?
        };
        for path in others.printed()?.split(|byte| *byte == 0) {
            // A repository inside the worktree is listed as its folder, with a slash.
            let path = path.strip_suffix(b"/").unwrap_or(path);
            if path.is_empty() {
                continue;
            }
            let path = os_string(path);
            if since.is_none() && fs::symlink_metadata(self.root.join(&path)).is_err() {
                // In git's list and no longer on disk: not a file on disk.
                continue;
            }
            changes
                .entry(path)
                // In the commit, taken out of git's list, and still on disk: git's diff
                // calls it deleted and lists the file as untracked. It is one file, that
                // was there and is there.
                .and_modify(|change| *change = Change::Modified)
                .or_insert(Change::Added);
        }

        Ok(changes
            .into_iter()
            .map(|(path, change)| ChangedPath {
                path: PathBuf::from(path),
                change,
            })
            .collect())
    }

    /// The content of the file at `path` in `commit`; none when the commit has no file
    /// there, and also when git knows no such commit, which [`Worktree::resolve`] tells
    /// apart. A link is given as the path it points at, as git stores it. `commit` is a
    /// full id, as [`CommitId`] says: any other way to name a commit is a commit git
    /// does not know.
    ///
    /// Answered by the one process this worktree keeps for contents; a process that has
    /// died is started again once.
    pub fn content_at(&self, commit: &CommitId, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let (Some(path), true) = (inside(path), is_full_id(commit.as_str())) else {
            return Ok(None);
        };
        let mut name = commit.as_str().as_bytes().to_vec();
        name.push(b':');
        name.extend_from_slice(path.as_os_str().as_bytes());

        let mut kept = self.contents.lock().unwrap_or_else(PoisonError::into_inner);
        let mut second_try = false;
        loop {
            let contents = match kept.as_mut() {
                Some(contents) => contents,
                None => kept.insert(Contents::start(&self.git, &self.root, &self.hooks_off)?),
            };
            match contents.read(&name) {
                Ok(Read::Blob(content)) => return Ok(Some(content)),
                Ok(Read::NoBlob) => return Ok(None),
                Err(e) => {
                    // The process died, or what it said makes no sense any more: end it,
                    // and ask a new one, once.
                    if let Some(contents) = kept.take() {
                        contents.stop();
                    }
                    if second_try {
                        return Err(e);
                    }
                    second_try = true;
                }
            }
        }
    }

    /// The content of the file at `path` on disk; none when there is no file there. A
    /// link is given as the path it points at, and is not followed. `path` is relative to
    /// the folder of the worktree and must not lead out of it: where it does, by its
    /// words or through a link on its way, there is no file.
    pub fn content_on_disk(&self, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let Some(path) = inside(path) else {
            return Ok(None);
        };
        // A folder on the way that is a link leads somewhere else, perhaps out of the
        // worktree. git sees a link there, not the files behind it.
        let mut folder = self.root.clone();
        if let Some(above) = path.parent() {
            for name in above.components() {
                folder.push(name);
                match fs::symlink_metadata(&folder) {
                    Ok(found) if found.is_dir() => {}
                    Ok(_) => return Ok(None),
                    Err(e) => return nothing_there(e),
                }
            }
        }
        let file = self.root.join(&path);
        let found = match fs::symlink_metadata(&file) {
            Ok(found) => found,
            Err(e) => return nothing_there(e),
        };
        let read = if found.file_type().is_symlink() {
            fs::read_link(&file).map(|target| target.into_os_string().into_vec())
        } else if found.is_file() {
            fs::read(&file)
        } else {
            // A folder, which is a repository of its own, or something that is no file.
            return Ok(None);
        };
        match read {
            Ok(content) => Ok(Some(content)),
            Err(e) => nothing_there(e),
        }
    }

    /// Makes an ordinary git commit of every uncommitted change of the worktree, with
    /// `message`, on what is checked out, and gives its id: what `git add --all` and then
    /// `git commit` with that message make when the user types them.
    ///
    /// This is the user's own command, made through the app, and so it is not run the way
    /// the app reads and records: it runs with the user's hooks, identity and signing, as
    /// their git would, and it writes their list of the files. Only the variables that
    /// name another repository than this worktree's are left out, as for every run.
    ///
    /// Refused with [`CommitError::NothingToCommit`] when nothing is uncommitted, and
    /// with [`CommitError::Refused`] and git's own words when git ends with an error, as
    /// when a hook rejects the commit or no identity is set.
    pub fn commit_all(&self, message: &str) -> Result<CommitId, CommitError> {
        // Asked as reading asks: on a copy of the list of files, so that the question
        // neither writes nor locks the user's own.
        let uncommitted = {
            let index = IndexCopy::of(&self.git_dir.join("index")).map_err(GitError::from)?;
            let hooks_off = hooks_off(&self.git, Place::Folder(&self.root))?;
            let mut command = self.git.command(Place::Folder(&self.root), &hooks_off);
            command.env("GIT_INDEX_FILE", index.file()).args([
                "status",
                "--porcelain",
                "-z",
                "--untracked-files=all",
            ]);
            !finish(command)?.printed()?.is_empty()
        };
        if !uncommitted {
            return Err(CommitError::NothingToCommit);
        }

        let mut add = self.git.plain(&self.root);
        add.args(["add", "--all"]);
        let answer = finish(add)?;
        if answer.code != Some(0) {
            return Err(CommitError::refused_by(answer));
        }
        // The message goes in on the standard input: it may hold any character.
        let mut commit = self.git.plain(&self.root);
        commit.args(["commit", "--file=-"]);
        let answer = finish_with(commit, message.as_bytes())?;
        if answer.code != Some(0) {
            return Err(CommitError::refused_by(answer));
        }
        match self.head_commit()? {
            Some(id) => Ok(id),
            None => Err(answer
                .unreadable("no commit is checked out after the commit")
                .into()),
        }
    }

    /// One run of git in this worktree.
    fn run<I, S>(&self, args: I) -> Result<Answer, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.git
            .run(Place::Folder(&self.root), &self.hooks_off, args)
    }

    /// The commit that is checked out; none where nothing is committed yet.
    pub(crate) fn head_commit(&self) -> Result<Option<CommitId>, GitError> {
        let answer = self.run(["rev-parse", "-q", "--verify", "HEAD^{commit}"])?;
        match answer.code {
            Some(0) => Ok(Some(CommitId::new(text(&answer.stdout)))),
            Some(1) => Ok(None),
            _ => Err(answer.failed()),
        }
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let kept = self
            .contents
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(contents) = kept.take() {
            contents.stop();
        }
    }
}

/// Reads what `git diff --name-status -z` printed, the paths keyed by their bytes, which
/// is the order git lists paths in. `command` is the run, for the words of a failure.
pub(crate) fn name_status(
    command: &str,
    printed: &[u8],
) -> Result<BTreeMap<OsString, Change>, GitError> {
    let mut changes = BTreeMap::new();
    let mut fields = printed.split(|byte| *byte == 0).filter(|f| !f.is_empty());
    while let Some(status) = fields.next() {
        let mut path = || {
            fields
                .next()
                .map(os_string)
                .ok_or_else(|| unreadable(command, "a status with no path"))
        };
        let (path, change) = match status[0] {
            b'A' => (path()?, Change::Added),
            b'D' => (path()?, Change::Deleted),
            // A file whose kind changed, or that a merge left undecided, is a changed
            // file.
            b'M' | b'T' | b'U' => (path()?, Change::Modified),
            b'R' => {
                let from = PathBuf::from(path()?);
                (path()?, Change::Renamed { from })
            }
            // A copy leaves its source where it was: the copy is a new file.
            b'C' => {
                path()?;
                (path()?, Change::Added)
            }
            _ => {
                return Err(unreadable(
                    command,
                    format!("the status {:?}", String::from_utf8_lossy(status)),
                ));
            }
        };
        changes.insert(path, change);
    }
    Ok(changes)
}

/// The setting that turns off each hook the configuration defines, one by one: turning
/// off the hooks folder does not reach them.
pub(crate) fn hooks_off(git: &Git, place: Place<'_>) -> Result<Vec<OsString>, GitError> {
    let answer = git.run(
        place,
        &[],
        [
            "config",
            "--name-only",
            "-z",
            "--get-regexp",
            r"^hook\..*\.command$",
        ],
    )?;
    match answer.code {
        Some(0) => Ok(answer
            .stdout
            .split(|byte| *byte == 0)
            .filter_map(|key| key.strip_suffix(b".command"))
            .map(|hook| {
                let mut key = hook.to_vec();
                key.extend_from_slice(b".enabled");
                OsString::from_vec(key)
            })
            .collect()),
        // git says by its exit code that no key matched.
        Some(1) => Ok(Vec::new()),
        _ => Err(answer.failed()),
    }
}

/// Reads git's list of worktrees: records that end with an empty field, each a run of
/// fields that end with a zero byte.
fn listed(printed: &[u8], common_dir: &Path) -> Vec<Listed> {
    let mut all = Vec::new();
    let mut first = true;
    for record in printed
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>()
        .split(|f| f.is_empty())
    {
        if record.is_empty() {
            continue;
        }
        let main = std::mem::take(&mut first);
        let field = |name: &[u8]| {
            record
                .iter()
                .find_map(|field| match field.strip_prefix(name) {
                    Some(rest) if rest.is_empty() => Some(rest),
                    Some(rest) => rest.strip_prefix(b" "),
                    None => None,
                })
        };
        // A repository with no files checked out, and a worktree git would forget
        // because its folder is gone, are not worktrees to read.
        if field(b"bare").is_some() || field(b"prunable").is_some() {
            continue;
        }
        let (Some(path), Some(commit)) = (field(b"worktree"), field(b"HEAD")) else {
            continue;
        };
        let Ok(path) = fs::canonicalize(OsStr::from_bytes(path)) else {
            continue;
        };
        // The folder may hold something else by now, or a worktree of another
        // repository: a worktree of this one keeps its git folder in the shared one.
        let Some(git_dir) = git_dir_of(&path) else {
            continue;
        };
        let of_this_repository = if main {
            git_dir == common_dir
        } else {
            git_dir.parent() == Some(&common_dir.join("worktrees"))
        };
        if !of_this_repository {
            continue;
        }
        // git writes zeros where nothing is committed yet.
        let commit = Some(text(commit))
            .filter(|id| id.bytes().any(|digit| digit != b'0'))
            .map(CommitId::new);
        let head = match (field(b"branch"), commit) {
            (Some(branch), commit) => {
                let name = text(branch);
                Head::Branch {
                    name: name.strip_prefix("refs/heads/").unwrap_or(&name).to_owned(),
                    commit,
                }
            }
            (None, Some(commit)) => Head::Detached(commit),
            (None, None) => continue,
        };
        all.push(Listed {
            path,
            head,
            main,
            git_dir,
        });
    }
    all
}

/// The git folder of the worktree at `folder`, read from the worktree itself: its
/// `.git` is that folder, or a file that names it.
fn git_dir_of(folder: &Path) -> Option<PathBuf> {
    let dot_git = folder.join(".git");
    if dot_git.is_dir() {
        return fs::canonicalize(dot_git).ok();
    }
    let named = fs::read(&dot_git).ok()?;
    let named = named.strip_prefix(b"gitdir: ")?;
    let named = named.strip_suffix(b"\n").unwrap_or(named);
    // A path that is not a full one is counted from the worktree.
    fs::canonicalize(folder.join(OsStr::from_bytes(named))).ok()
}

/// `path` as a path inside a worktree, or none when it is not one: when it is a full
/// path, is empty, or climbs with `..`.
pub(crate) fn inside(path: &Path) -> Option<PathBuf> {
    let mut inside = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => inside.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!inside.as_os_str().is_empty()).then_some(inside)
}

/// Whether this is the full id of a commit, in either of git's two lengths.
pub(crate) fn is_full_id(id: &str) -> bool {
    matches!(id.len(), 40 | 64) && id.bytes().all(|digit| digit.is_ascii_hexdigit())
}

/// A failure to read that means there is no file, as none; any other as itself.
fn nothing_there(e: io::Error) -> Result<Option<Vec<u8>>, GitError> {
    match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory | io::ErrorKind::IsADirectory => {
            Ok(None)
        }
        _ => Err(e.into()),
    }
}

/// What git printed on one line, without the line feed.
fn text(printed: &[u8]) -> String {
    String::from_utf8_lossy(printed).trim_end().to_owned()
}

pub(crate) fn os_string(bytes: &[u8]) -> OsString {
    OsString::from_vec(bytes.to_vec())
}

/// The short name of a branch, from its full one.
fn short_name(full: &str) -> &str {
    full.strip_prefix("refs/heads/")
        .or_else(|| full.strip_prefix("refs/remotes/"))
        .unwrap_or(full)
}
