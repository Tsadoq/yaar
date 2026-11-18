//! The rules behind the base of a workspace, as plain functions of what git reports.

use yaar_recorder::{Head, Refs};

/// The default branch of a repository, found without asking any server: the base of a
/// workspace whose user set none.
///
/// The first of three steps that answers:
///
/// 1. the default branch a remote recorded, if that branch still exists; `origin` is
///    asked before any other remote;
/// 2. the nearest of `main`, `origin/main`, `master`, `origin/master`, `develop` and
///    `origin/develop`, among those that exist;
/// 3. the nearest of all other branches, local and of the remotes, leaving out the
///    branch that is checked out and the branch it follows.
///
/// "Nearest" is the branch with the fewest commits between the point where it parts from
/// what is checked out and what is checked out. Between two that are equally near, the
/// earlier in the order written above wins, and in step 3 the earlier in git's order,
/// local branches first.
///
/// The answer is the short name of a branch, as [`Refs`] lists it. None when no step
/// answers: there is then nothing to compare the workspace with.
pub fn default_base(head: &Head, refs: &Refs) -> Option<String> {
    todo!()
}

/// Whether a workspace is on its base branch: the branch checked out is `base` itself,
/// or `base` is a branch of a remote and the branch checked out is the local branch of
/// the same name, `main` against `origin/main`. Never when no branch is checked out, or
/// when `base` is a commit.
pub fn on_base_branch(head: &Head, base: &str, refs: &Refs) -> bool {
    todo!()
}
