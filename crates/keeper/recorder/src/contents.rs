//! The one process a worktree or a state store keeps to hand out the contents of files in
//! commits.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, BufReader, Read as _, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, PoisonError};

use crate::git::{Answer, Git, GitError, Place, could_not_start, shown};

/// What the process answered for one name.
pub(crate) enum Read {
    /// The content of the file by that name.
    Blob(Vec<u8>),
    /// There is no file by that name: nothing at all, or a folder, or a repository of
    /// its own.
    NoBlob,
}

/// What to ask a process that hands out contents for the file `name` (a commit, a colon and
/// a path), for a git that has the newer forms or has not. A git without `-Z` ends a name
/// at a line feed and drops a carriage return before it, so a name that holds either is
/// asked by the id of the object it names: `run` is given the arguments of the run of git
/// that finds it. None means there is no such object, hence no content.
pub(crate) fn asked(
    has_newer_forms: bool,
    name: Vec<u8>,
    run: impl FnOnce(&[&OsStr]) -> Result<Answer, GitError>,
) -> Result<Option<Vec<u8>>, GitError> {
    if has_newer_forms || !name.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
        return Ok(Some(name));
    }
    let spelled = OsStr::from_bytes(&name);
    let answer = run(&[
        OsStr::new("rev-parse"),
        OsStr::new("-q"),
        OsStr::new("--verify"),
        OsStr::new("--end-of-options"),
        spelled,
    ])?;
    match answer.code {
        Some(0) => Ok(Some(answer.stdout.trim_ascii_end().to_vec())),
        Some(1) => Ok(None),
        _ => Err(answer.failed()),
    }
}

/// Asks the process in `kept` for the object `name`, starting it with `start` when none is
/// running. A process that has died, or whose answer makes no sense any more, is ended and
/// a new one asked, once: a second failure is the answer.
///
/// The lock on `kept` is held for the whole question, so one process answers one question
/// at a time; it guards nothing else.
pub(crate) fn read_kept(
    kept: &Mutex<Option<Contents>>,
    start: impl Fn() -> Result<Contents, GitError>,
    name: &[u8],
) -> Result<Read, GitError> {
    let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
    let mut second_try = false;
    loop {
        let contents = match kept.as_mut() {
            Some(contents) => contents,
            None => kept.insert(start()?),
        };
        match contents.read(name) {
            Ok(read) => return Ok(read),
            Err(e) => {
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

/// Ends the process in `kept`, if one runs, and waits for it.
pub(crate) fn stop_kept(kept: &mut Mutex<Option<Contents>>) {
    let kept = kept.get_mut().unwrap_or_else(PoisonError::into_inner);
    if let Some(contents) = kept.take() {
        contents.stop();
    }
}

/// A running `git cat-file` that is asked name by name.
#[derive(Debug)]
pub(crate) struct Contents {
    child: Child,
    asks: ChildStdin,
    answers: BufReader<ChildStdout>,
    // The command, as one would type it, for when its answer is not understood.
    command: String,
    // What ends a name asked and every answer: a zero byte with `-Z`, a line feed without.
    end: u8,
}

impl Contents {
    /// Starts the process in the worktree at `root`.
    pub(crate) fn start(
        git: &Git,
        root: &Path,
        hooks_off: &[OsString],
    ) -> Result<Contents, GitError> {
        Self::start_in(
            git.command(Place::Folder(root), hooks_off),
            git.has_newer_forms(),
        )
    }

    /// Starts the process from a run of git that is prepared and not yet started, of a git
    /// that has the newer forms or has not.
    pub(crate) fn start_in(
        mut command: Command,
        has_newer_forms: bool,
    ) -> Result<Contents, GitError> {
        // `-Z` ends every name and every answer with a zero byte, so a path may hold a
        // line feed; git has it from the newer forms on. Without it each ends at a line
        // feed, and `asked` keeps the names that hold one away. What git complains of
        // meanwhile has no reader.
        command.args(["cat-file", "--batch"]);
        if has_newer_forms {
            command.arg("-Z");
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let shown = shown(&command);
        let mut child = command.spawn().map_err(could_not_start)?;
        match (child.stdin.take(), child.stdout.take()) {
            (Some(asks), Some(answers)) => Ok(Contents {
                child,
                asks,
                answers: BufReader::new(answers),
                command: shown,
                end: if has_newer_forms { 0 } else { b'\n' },
            }),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                Err(io::Error::other("git was started without its pipes").into())
            }
        }
    }

    /// Asks for the object `name`, which is a commit, a colon and a path. An error
    /// means the process is of no use any more.
    pub(crate) fn read(&mut self, name: &[u8]) -> Result<Read, GitError> {
        let mut ask = name.to_vec();
        ask.push(self.end);
        self.asks.write_all(&ask)?;
        self.asks.flush()?;

        let mut header = Vec::new();
        self.answers.read_until(self.end, &mut header)?;
        if header.pop() != Some(self.end) {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "git ended").into());
        }
        // What git does not find it names back, word for word.
        if header.strip_suffix(b" missing") == Some(name) {
            return Ok(Read::NoBlob);
        }
        // What it finds: the id of the object, its kind and its size, then the object.
        let mut fields = header.rsplitn(3, |byte| *byte == b' ');
        let size = fields
            .next()
            .and_then(|size| std::str::from_utf8(size).ok()?.parse().ok());
        if let (Some(size), Some(kind), Some(id)) = (size, fields.next(), fields.next())
            && id.iter().all(u8::is_ascii_hexdigit)
        {
            let mut content = Vec::new();
            (&mut self.answers).take(size).read_to_end(&mut content)?;
            let mut end = [1];
            self.answers.read_exact(&mut end)?;
            if content.len() as u64 != size || end != [self.end] {
                return Err(self.unreadable("an object shorter than its size"));
            }
            return Ok(if kind == b"blob" {
                Read::Blob(content)
            } else {
                Read::NoBlob
            });
        }
        // A repository inside the repository is a name with nothing to read behind it.
        if header.ends_with(b" submodule") || header.ends_with(b" ambiguous") {
            return Ok(Read::NoBlob);
        }
        Err(self.unreadable(format!("the answer {:?}", String::from_utf8_lossy(&header))))
    }

    /// Ends the process and waits for it.
    pub(crate) fn stop(mut self) {
        // Closing its input lets git end by itself; killing it makes sure it does.
        drop(self.asks);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn unreadable(&self, what: impl Into<String>) -> GitError {
        GitError::Unreadable {
            command: self.command.clone(),
            what: what.into(),
        }
    }
}
