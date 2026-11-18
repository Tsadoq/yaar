//! The world of one test: its own folders, its own repositories, its own Keeper.
//!
//! Nothing of one world touches another or the machine's own: a home folder, the app's
//! folder and a runtime folder of the world's own, and git told to read no configuration
//! but what the world sets. Tests therefore run side by side.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use yaar_link::Request;

use crate::client::Client;

/// One test's world. Everything in it lies under one folder with a short path. When the
/// world is dropped its Keeper is killed and the folder removed.
pub struct World {
    root: PathBuf,
    keeper: Option<Child>,
}

impl World {
    /// A new world: empty folders, no repository, no Keeper.
    #[allow(clippy::new_without_default)]
    pub fn new() -> World {
        todo!()
    }

    /// The home folder of the world's user.
    pub fn home(&self) -> PathBuf {
        todo!()
    }

    /// The app's folder in this world, where the Keeper keeps its database.
    pub fn data_dir(&self) -> PathBuf {
        todo!()
    }

    /// The path the Keeper of this world is asked to listen at.
    pub fn socket(&self) -> PathBuf {
        todo!()
    }

    /// A command that runs `program` as the world's user: with the environment of this
    /// process cleared but for the search path, and the world's own in its place. That
    /// names the home, data and runtime folders, the socket and the Keeper program, and
    /// tells git to read neither the machine's configuration nor the user's.
    pub fn command(&self, program: &Path) -> Command {
        todo!()
    }

    /// Starts the built Keeper program on the socket of this world and returns its
    /// process id once it listens. Panics if one already runs.
    pub fn start_keeper(&mut self) -> u32 {
        todo!()
    }

    /// The process id of the Keeper this world started, while it runs.
    pub fn keeper_pid(&mut self) -> Option<u32> {
        todo!()
    }

    /// A scripted client attached to the Keeper of this world, as a Window would attach.
    pub fn client(&self) -> Client {
        todo!()
    }

    /// A new git repository named `name` in this world, with one commit on the branch
    /// `main`.
    pub fn repository(&self, name: &str) -> Repo {
        todo!()
    }

    /// A tap between a Window and the Keeper of this world: a socket of its own that
    /// passes every frame on in both directions and keeps what it saw.
    pub fn tap(&self) -> Tap {
        todo!()
    }
}

/// A git repository of a world, driven as a user drives git: by commands. The author, the committer and the dates are fixed, so the same steps give
/// the same commits.
pub struct Repo {
    root: PathBuf,
    world: PathBuf,
}

impl Repo {
    /// The folder of the worktree.
    pub fn path(&self) -> &Path {
        todo!()
    }

    /// Runs git in the worktree with these arguments and returns what it printed,
    /// without the line end. Panics, with what git wrote, when git fails.
    pub fn git(&self, args: &[&str]) -> String {
        todo!()
    }

    /// Writes a file of the worktree, making the folders above it.
    pub fn write(&self, path: &str, content: &str) {
        todo!()
    }

    /// Stages everything and commits it with this message. Returns the id of the commit.
    pub fn commit(&self, message: &str) -> String {
        todo!()
    }
}

/// A tap on the Link: what a test puts between a Window and a Keeper to read the
/// requests that reach the Keeper, without a door into either.
///
/// It listens at [`Tap::socket`]. Each connection made there is joined to a connection of
/// its own to the Keeper, and every frame is passed on unchanged, both ways.
pub struct Tap {
    socket: PathBuf,
}

impl Tap {
    /// Where a Window is told to find the Keeper, to go through the tap.
    pub fn socket(&self) -> PathBuf {
        todo!()
    }

    /// Every request that passed towards the Keeper so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        todo!()
    }

    /// Waits until a request that `wanted` accepts has passed, at most `limit`. Panics,
    /// with the requests seen, when none comes.
    pub fn wait_for_request(&self, limit: Duration, wanted: impl Fn(&Request) -> bool) -> Request {
        todo!()
    }
}
