mod adding;

pub use adding::{AddError, Added, resolved};

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use crate::contents::{Contents, Read, asked, read_kept, stop_kept};
use crate::git::{Answer, Git, GitError, Place, finish, finish_with, unreadable};
use crate::index::{IndexBackup, IndexCopy};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitId(String);

impl CommitId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch {
        name: String,
        commit: Option<CommitId>,
    },
    Detached(CommitId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: CommitId,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub path: PathBuf,
    pub head: Head,
    pub main: bool,
    pub git_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBranch {
    pub name: String,
    /// How many commits the checked-out commit has that this branch does not.
    pub ahead: u32,
    /// None also when the branch it was set to follow is no longer there.
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranch {
    pub name: String,
    pub remote: String,
    pub ahead: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Refs {
    pub remotes: Vec<String>,
    /// The default branch each remote recorded, in the order of `remotes`, whether or not it still
    /// exists.
    pub remote_defaults: Vec<String>,
    pub local: Vec<LocalBranch>,
    pub remote: Vec<RemoteBranch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    Deleted,
    Renamed { from: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedPath {
    pub path: PathBuf,
    pub change: Change,
}

/// Names one branch as a branch, by its short name as [`Refs`] lists it. git reads a short name as
/// a tag before a branch; asked through this, it is asked for the branch only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch<'a> {
    Local(&'a str),
    Remote(&'a str),
}

impl Branch<'_> {
    fn full_name(self) -> String {
        match self {
            Branch::Local(name) => format!("refs/heads/{name}"),
            Branch::Remote(name) => format!("refs/remotes/{name}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commits<'a> {
    Since(&'a CommitId),
    NotOn(Branch<'a>),
    OnNoRemoteBranch,
}

#[derive(Debug)]
pub enum CommitError {
    NothingToCommit,
    Refused {
        /// What git wrote to its standard error; when it wrote nothing there, its standard output,
        /// where some refusals (no changes added to commit) appear.
        stderr: String,
    },
    Git(GitError),
}

impl CommitError {
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

#[derive(Debug)]
pub enum OpenError {
    NoSuchFolder,
    NotARepository,
    Bare,
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

/// One worktree, read through git: every call asks again. Keeps one running `git cat-file` for file
/// contents, started at the first [`Worktree::content_at`] and ended on drop.
#[derive(Debug)]
pub struct Worktree {
    git: Git,
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
    hooks_off: Vec<OsString>,
    contents: Mutex<Option<Contents>>,
}

impl Worktree {
    /// A folder inside the git folder of a repository lies in no worktree.
    pub fn open(git: &Git, folder: &Path) -> Result<Worktree, OpenError> {
        if !folder.is_dir() {
            return Err(OpenError::NoSuchFolder);
        }
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

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

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
            Some(1) => match commit {
                Some(commit) => Ok(Head::Detached(commit)),
                None => Err(symbolic.unreadable("neither a branch nor a commit is checked out")),
            },
            _ => Err(symbolic.failed()),
        }
    }

    pub fn list(&self) -> Result<Vec<Listed>, GitError> {
        let printed = list_printed(&self.git, |args| self.run(args))?;
        Ok(listed(&printed, &self.common_dir))
    }

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
        let printed = list_printed(git, |args| git.run(place, &hooks_off, args))?;
        Ok(listed(&printed, &common_dir))
    }

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
                Some(1) => {}
                _ => return Err(answer.failed()),
            }
        }

        let has_newer_forms = self.git.has_newer_forms();
        const WITH_DISTANCE: &str = "--format=%(refname)%00%(upstream)%00%(ahead-behind:HEAD)";
        const WITHOUT: &str = "--format=%(refname)%00%(upstream)%00";
        // A git without the atom is given the commit of each branch instead, and counts
        // from it with a run of its own.
        const WITH_COMMIT: &str = "--format=%(refname)%00%(upstream)%00%(objectname)";
        let list = |format: &str| self.run(["for-each-ref", format, "refs/heads", "refs/remotes"]);
        let mut answer = list(if has_newer_forms {
            WITH_DISTANCE
        } else {
            WITH_COMMIT
        })?;
        if has_newer_forms && answer.code != Some(0) && self.head_commit()?.is_none() {
            answer = list(WITHOUT)?;
        }
        let command = answer.command().to_owned();
        let printed = text(&answer.printed()?);
        let unreadable = |what: String| unreadable(&command, what);

        let nothing_committed = !has_newer_forms && self.head_commit()?.is_none();
        let mut counted: BTreeMap<&str, u32> = BTreeMap::new();
        let mut rows = Vec::new();
        for line in printed.lines() {
            let mut fields = line.split('\0');
            let (Some(name), Some(upstream), Some(distance)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(unreadable(format!("the line {line:?}")));
            };
            let ahead = if has_newer_forms {
                // git prints the side of the branch first and that of what is checked out second.
                match distance.split_once(' ') {
                    Some((_, ahead)) => ahead
                        .parse::<u32>()
                        .map_err(|_| unreadable(format!("the distance {distance:?}")))?,
                    None => 0,
                }
            } else if nothing_committed {
                0
            } else if let Some(ahead) = counted.get(distance) {
                *ahead
            } else {
                let range = format!("{distance}..HEAD");
                let answer = self.run(["rev-list", "--count", "--end-of-options", &range])?;
                let printed = text(&answer.printed()?);
                let ahead = printed
                    .parse::<u32>()
                    .map_err(|_| unreadable(format!("the count {printed:?}")))?;
                counted.insert(distance, ahead);
                ahead
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

    pub fn commit_of(&self, branch: Branch<'_>) -> Result<Option<CommitId>, GitError> {
        self.resolve(&branch.full_name())
    }

    /// Where a tag and a branch have the name, the tag wins, as for the user.
    pub fn resolve(&self, revision: &str) -> Result<Option<CommitId>, GitError> {
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

    pub fn fork_point(&self, base: &CommitId) -> Result<Option<CommitId>, GitError> {
        let answer = self.run(["merge-base", "--end-of-options", "HEAD", base.as_str()])?;
        match answer.code {
            Some(0) => Ok(Some(CommitId::new(text(&answer.stdout)))),
            Some(1) => Ok(None),
            _ if self.head_commit()?.is_none() => Ok(None),
            _ => Err(answer.failed()),
        }
    }

    pub fn commits(&self, which: Commits<'_>) -> Result<Vec<Commit>, GitError> {
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

    /// Ordered by path, byte by byte as git orders paths. With `since` none, every file on disk
    /// that git does not ignore is new. git works on a copy of its index; the user's own is not
    /// written.
    pub fn changed_paths(&self, since: Option<&CommitId>) -> Result<Vec<ChangedPath>, GitError> {
        let index = IndexCopy::of(&self.git_dir.join("index"))?;
        // git writes the copy when it has brought it up to date, and a hook can wait for that: the
        // hooks are those defined now, not those of when this was opened.
        let hooks_off = hooks_off(&self.git, Place::Folder(&self.root))?;
        let run = |args: &[&str]| {
            let mut command = self.git.command(Place::Folder(&self.root), &hooks_off);
            command.env("GIT_INDEX_FILE", index.file()).args(args);
            finish(command)
        };

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
            let path = path.strip_suffix(b"/").unwrap_or(path);
            if path.is_empty() {
                continue;
            }
            let path = os_string(path);
            if since.is_none() && fs::symlink_metadata(self.root.join(&path)).is_err() {
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

    /// None when `commit` has no file at `path`, and also when git knows no such commit, which
    /// [`Worktree::resolve`] tells apart. `commit` is a full id: any other way to name a commit
    /// is one git does not know.
    pub fn content_at(&self, commit: &CommitId, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let (Some(path), true) = (inside(path), is_full_id(commit.as_str())) else {
            return Ok(None);
        };
        let mut name = commit.as_str().as_bytes().to_vec();
        name.push(b':');
        name.extend_from_slice(path.as_os_str().as_bytes());
        let Some(name) = asked(self.git.has_newer_forms(), name, |args| self.run(args))? else {
            return Ok(None);
        };

        let start = || Contents::start(&self.git, &self.root, &self.hooks_off);
        match read_kept(&self.contents, start, &name)? {
            Read::Blob(content) => Ok(Some(content)),
            Read::NoBlob => Ok(None),
        }
    }

    pub fn content_on_disk(&self, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let Some(path) = inside(path) else {
            return Ok(None);
        };
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
            return Ok(None);
        };
        match read {
            Ok(content) => Ok(Some(content)),
            Err(e) => nothing_there(e),
        }
    }

    /// Makes an ordinary commit of every uncommitted change, as `git add --all` and then `git
    /// commit` with `message` would, with the user's hooks, identity and signing. A refusal leaves
    /// the index byte for byte as before.
    pub fn commit_all(&self, message: &str) -> Result<CommitId, CommitError> {
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

        let backup = IndexBackup::take(&self.git_dir.join("index")).map_err(GitError::from)?;
        let mut add = self.git.plain(&self.root);
        // `--no-ignore-errors`: the user's `add.ignoreErrors` would make git stage what it can,
        // write the index and only then exit in error.
        add.args(["add", "--all", "--no-ignore-errors"]);
        // A `git add` that fails, as when a file cannot be added or the lock is held, has written
        // nothing: it writes the index last. Putting the bytes back would overwrite what another
        // git wrote since, so nothing is put back.
        let answer = finish(add)?;
        if answer.code != Some(0) {
            return Err(CommitError::refused_by(answer));
        }
        let mut commit = self.git.plain(&self.root);
        commit.args(["commit", "--file=-"]);
        let answer = match finish_with(commit, message.as_bytes()) {
            Ok(answer) => answer,
            Err(e) => {
                let _ = backup.restore();
                return Err(e.into());
            }
        };
        if answer.code != Some(0) {
            return Err(refused_and_put_back(answer, backup));
        }
        match self.head_commit()? {
            Some(id) => Ok(id),
            None => Err(answer
                .unreadable("no commit is checked out after the commit")
                .into()),
        }
    }

    fn run<I, S>(&self, args: I) -> Result<Answer, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.git
            .run(Place::Folder(&self.root), &self.hooks_off, args)
    }

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
        stop_kept(&mut self.contents);
    }
}

fn refused_and_put_back(answer: Answer, backup: IndexBackup) -> CommitError {
    let refusal = CommitError::refused_by(answer);
    match (backup.restore(), refusal) {
        (Err(e), CommitError::Refused { stderr }) => CommitError::Refused {
            stderr: format!("{stderr}\nThe list of staged files could not be put back: {e}"),
        },
        (_, refusal) => refusal,
    }
}

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

/// The `hook.<name>.enabled` key of each hook the configuration defines, to turn it off:
/// turning off the hooks folder does not reach such hooks.
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
        Some(1) => Ok(Vec::new()),
        _ => Err(answer.failed()),
    }
}

fn list_printed(
    git: &Git,
    run: impl FnOnce(&[&str]) -> Result<Answer, GitError>,
) -> Result<Vec<u8>, GitError> {
    if git.has_newer_forms() {
        run(&["worktree", "list", "--porcelain", "-z"])?.printed()
    } else {
        Ok(zero_ended(
            &run(&["worktree", "list", "--porcelain"])?.printed()?,
        ))
    }
}

/// The list of an older git (`--porcelain` without `-z`) in the form `-z` gives. Such a git prints
/// paths unquoted, so a line with no field's name continues the field before it; a path whose
/// second line looks like a field is misread.
fn zero_ended(printed: &[u8]) -> Vec<u8> {
    const NAMES: [&[u8]; 7] = [
        b"worktree",
        b"HEAD",
        b"branch",
        b"detached",
        b"bare",
        b"locked",
        b"prunable",
    ];
    let named = |line: &[u8]| {
        NAMES.iter().any(|name| {
            line.strip_prefix(*name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(b" "))
        })
    };
    let mut fields: Vec<Vec<u8>> = Vec::new();
    for line in printed
        .strip_suffix(b"\n")
        .unwrap_or(printed)
        .split(|byte| *byte == b'\n')
    {
        match fields.last_mut() {
            Some(last) if !line.is_empty() && !named(line) && !last.is_empty() => {
                last.push(b'\n');
                last.extend_from_slice(line);
            }
            _ => fields.push(line.to_vec()),
        }
    }
    let mut zero_ended = Vec::new();
    for field in fields {
        zero_ended.extend_from_slice(&field);
        zero_ended.push(0);
    }
    zero_ended
}

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
        if field(b"bare").is_some() || field(b"prunable").is_some() {
            continue;
        }
        let (Some(path), Some(commit)) = (field(b"worktree"), field(b"HEAD")) else {
            continue;
        };
        let Ok(path) = fs::canonicalize(OsStr::from_bytes(path)) else {
            continue;
        };
        // The folder may hold something else by now, or a worktree of another repository: one of
        // this repository keeps its git folder in the shared one.
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

fn git_dir_of(folder: &Path) -> Option<PathBuf> {
    let dot_git = folder.join(".git");
    if dot_git.is_dir() {
        return fs::canonicalize(dot_git).ok();
    }
    let named = fs::read(&dot_git).ok()?;
    let named = named.strip_prefix(b"gitdir: ")?;
    let named = named.strip_suffix(b"\n").unwrap_or(named);
    fs::canonicalize(folder.join(OsStr::from_bytes(named))).ok()
}

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

pub(crate) fn is_full_id(id: &str) -> bool {
    matches!(id.len(), 40 | 64) && id.bytes().all(|digit| digit.is_ascii_hexdigit())
}

fn nothing_there(e: io::Error) -> Result<Option<Vec<u8>>, GitError> {
    match e.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory | io::ErrorKind::IsADirectory => {
            Ok(None)
        }
        _ => Err(e.into()),
    }
}

fn text(printed: &[u8]) -> String {
    String::from_utf8_lossy(printed).trim_end().to_owned()
}

pub(crate) fn os_string(bytes: &[u8]) -> OsString {
    OsString::from_vec(bytes.to_vec())
}

fn short_name(full: &str) -> &str {
    full.strip_prefix("refs/heads/")
        .or_else(|| full.strip_prefix("refs/remotes/"))
        .unwrap_or(full)
}
