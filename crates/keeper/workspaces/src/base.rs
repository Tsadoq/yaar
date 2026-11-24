use yaar_recorder::{Head, Refs};

/// / The default a remote recorded (`origin` first) if its branch exists, else the nearest
/// / of `USUAL`, else the nearest other branch (not the one checked out or followed).
/// / Nearest: the checked-out commit has the fewest commits it lacks; ties go to the earlier.
pub fn default_base(head: &Head, refs: &Refs) -> Option<String> {
    recorded_default(refs)
        .or_else(|| nearest_of_the_usual(refs))
        .or_else(|| nearest_of_the_others(head, refs))
}

/// Tie-break order for equally near names.
const USUAL: [&str; 6] = [
    "main",
    "origin/main",
    "master",
    "origin/master",
    "develop",
    "origin/develop",
];

/// A recorded default whose branch is gone (the server renamed it) is skipped.
fn recorded_default(refs: &Refs) -> Option<String> {
    let mut existing = refs
        .remote_defaults
        .iter()
        .filter_map(|default| refs.remote.iter().find(|branch| &branch.name == default));
    existing
        .clone()
        .find(|branch| branch.remote == "origin")
        .or_else(|| existing.next())
        .map(|branch| branch.name.clone())
}

/// The checked-out branch counts: on `main` the base is `main`.
fn nearest_of_the_usual(refs: &Refs) -> Option<String> {
    nearest(USUAL.iter().filter_map(|usual| {
        let local = refs.local.iter().find(|branch| branch.name == *usual);
        let remote = refs.remote.iter().find(|branch| branch.name == *usual);
        local
            .map(|branch| (&branch.name, branch.ahead))
            .or(remote.map(|branch| (&branch.name, branch.ahead)))
    }))
}

fn nearest_of_the_others(head: &Head, refs: &Refs) -> Option<String> {
    let checked_out = match head {
        Head::Branch { name, .. } => refs.local.iter().find(|branch| &branch.name == name),
        Head::Detached(_) => None,
    };
    let left_out = |name: &String| {
        checked_out.is_some_and(|own| &own.name == name || own.upstream.as_ref() == Some(name))
    };
    let local = refs.local.iter().map(|branch| (&branch.name, branch.ahead));
    let remote = refs
        .remote
        .iter()
        .map(|branch| (&branch.name, branch.ahead));
    nearest(local.chain(remote).filter(|(name, _)| !left_out(name)))
}

fn nearest<'a>(branches: impl Iterator<Item = (&'a String, u32)>) -> Option<String> {
    branches
        .min_by_key(|(_, ahead)| *ahead)
        .map(|(name, _)| name.clone())
}

/// Whether `head` is `base`, or the local twin of the remote branch `base` (`main` for
/// `origin/main`). Never when detached or when `base` is a commit.
pub fn on_base_branch(head: &Head, base: &str, refs: &Refs) -> bool {
    let Head::Branch { name, .. } = head else {
        return false;
    };
    name == base
        || refs
            .remote
            .iter()
            .any(|branch| branch.name == base && base == format!("{}/{name}", branch.remote))
}

#[cfg(test)]
mod tests {
    use yaar_recorder::{CommitId, LocalBranch, RemoteBranch};

    use super::*;

    fn on(name: &str) -> Head {
        Head::Branch {
            name: name.to_owned(),
            commit: Some(CommitId::new("c0ffee")),
        }
    }

    fn detached() -> Head {
        Head::Detached(CommitId::new("c0ffee"))
    }

    fn refs(
        local: &[(&str, u32, Option<&str>)],
        remote: &[(&str, u32)],
        defaults: &[&str],
    ) -> Refs {
        let remote_of = |name: &str| name.split('/').next().expect("a remote").to_owned();
        let mut remotes: Vec<String> = remote
            .iter()
            .map(|(name, _)| remote_of(name))
            .chain(defaults.iter().map(|name| remote_of(name)))
            .collect();
        remotes.sort();
        remotes.dedup();
        Refs {
            remotes,
            remote_defaults: defaults.iter().map(|name| (*name).to_owned()).collect(),
            local: local
                .iter()
                .map(|(name, ahead, upstream)| LocalBranch {
                    name: (*name).to_owned(),
                    ahead: *ahead,
                    upstream: upstream.map(str::to_owned),
                })
                .collect(),
            remote: remote
                .iter()
                .map(|(name, ahead)| RemoteBranch {
                    name: (*name).to_owned(),
                    remote: remote_of(name),
                    ahead: *ahead,
                })
                .collect(),
        }
    }

    struct Recorded {
        case: &'static str,
        head: Head,
        refs: Refs,
        default_branch: &'static str,
        named: &'static str,
    }

    fn recorded() -> Vec<Recorded> {
        let main = Some("origin/main");
        vec![
            Recorded {
                case: "c01: a clone, a branch made on main",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("main", 2, main)],
                    &[("origin/main", 2)],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c02: a clone, the branch follows origin/main",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, main), ("main", 2, main)],
                    &[("origin/main", 2)],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c03: a clone, the branch was pushed and follows its own copy",
                head: on("feat-pushed"),
                refs: refs(
                    &[
                        ("feat-pushed", 0, Some("origin/feat-pushed")),
                        ("main", 2, main),
                    ],
                    &[("origin/feat-pushed", 0), ("origin/main", 2)],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c05b: a remote added to a new repository and fetched by a recent git",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None)],
                    &[("origin/feat-pushed", 2), ("origin/main", 2)],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c08: a branch made from another feature branch",
                head: on("feat-b"),
                refs: refs(
                    &[("feat-a", 1, None), ("feat-b", 0, None), ("main", 3, main)],
                    &[("origin/feat-pushed", 3), ("origin/main", 3)],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c08b: the same, the parent branch named outright and pushed",
                head: on("feat-b"),
                refs: refs(
                    &[
                        ("feat-a", 1, Some("origin/feat-a")),
                        ("feat-b", 0, None),
                        ("main", 3, main),
                    ],
                    &[
                        ("origin/feat-a", 1),
                        ("origin/feat-pushed", 3),
                        ("origin/main", 3),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c09: no branch checked out, in a linked worktree, one commit made",
                head: detached(),
                refs: refs(
                    &[("main", 1, main)],
                    &[
                        ("origin/feat-a", 1),
                        ("origin/feat-pushed", 1),
                        ("origin/main", 1),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c14: a linked worktree made on a new branch from main",
                head: on("feat"),
                refs: refs(
                    &[
                        ("feat", 0, None),
                        ("feat-a", 2, None),
                        ("feat-b", 2, None),
                        ("main", 2, main),
                    ],
                    &[
                        ("origin/feat-a", 2),
                        ("origin/feat-pushed", 2),
                        ("origin/main", 2),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c14b: a linked worktree made on a new branch from a feature branch",
                head: on("feat-b"),
                refs: refs(
                    &[
                        ("feat", 3, None),
                        ("feat-a", 1, None),
                        ("feat-b", 0, None),
                        ("main", 3, main),
                    ],
                    &[
                        ("origin/feat-a", 3),
                        ("origin/feat-pushed", 3),
                        ("origin/main", 3),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c15: a branch that was only on the remote, checked out by its name",
                head: on("feat-remote"),
                refs: refs(
                    &[
                        ("feat-remote", 0, Some("origin/feat-remote")),
                        ("main", 2, main),
                    ],
                    &[
                        ("origin/feat-a", 2),
                        ("origin/feat-pushed", 2),
                        ("origin/feat-remote", 0),
                        ("origin/main", 2),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c16: as c01, with the record of how the branch was made gone",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("main", 2, main)],
                    &[
                        ("origin/feat-a", 2),
                        ("origin/feat-pushed", 2),
                        ("origin/feat-remote", 2),
                        ("origin/main", 2),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c17: on main itself, one commit not pushed",
                head: on("main"),
                refs: refs(
                    &[("main", 0, main)],
                    &[
                        ("origin/feat-a", 1),
                        ("origin/feat-pushed", 1),
                        ("origin/feat-remote", 1),
                        ("origin/main", 1),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c18: main moved on and was merged into the branch",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("main", 4, main)],
                    &[
                        ("origin/feat-a", 6),
                        ("origin/feat-pushed", 6),
                        ("origin/feat-remote", 6),
                        ("origin/main", 6),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c19: the local main is old, the branch left the newer origin/main",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("main", 6, main)],
                    &[
                        ("origin/feat-a", 6),
                        ("origin/feat-pushed", 6),
                        ("origin/feat-remote", 6),
                        ("origin/main", 2),
                    ],
                    &["origin/main"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c04: a clone whose recorded default was deleted",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("main", 2, main)],
                    &[("origin/feat-pushed", 2), ("origin/main", 2)],
                    &[],
                ),
                default_branch: "main",
                named: "main",
            },
            Recorded {
                case: "c05: a remote added to a new repository, fetched with no default kept",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None)],
                    &[("origin/feat-pushed", 2), ("origin/main", 2)],
                    &[],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c06: no remote, the default branch is main",
                head: on("feat"),
                refs: refs(&[("feat", 0, None), ("main", 2, None)], &[], &[]),
                default_branch: "main",
                named: "main",
            },
            Recorded {
                case: "c13: the server renamed master to main, the recorded default is stale",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("master", 3, None)],
                    &[("origin/main", 2)],
                    // Still recorded, but its branch is gone: the recorded default must be skipped.
                    &["origin/master"],
                ),
                default_branch: "main",
                named: "origin/main",
            },
            Recorded {
                case: "c07: no remote, the default branch is trunk",
                head: on("feat"),
                refs: refs(&[("feat", 0, None), ("trunk", 2, None)], &[], &[]),
                default_branch: "trunk",
                named: "trunk",
            },
            Recorded {
                case: "c10: a clone, the default branch is trunk",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("trunk", 2, Some("origin/trunk"))],
                    &[("origin/trunk", 2)],
                    &["origin/trunk"],
                ),
                default_branch: "trunk",
                named: "origin/trunk",
            },
            Recorded {
                case: "c10b: the same, the recorded default deleted",
                head: on("feat"),
                refs: refs(
                    &[("feat", 0, None), ("trunk", 2, Some("origin/trunk"))],
                    &[("origin/trunk", 2)],
                    &[],
                ),
                default_branch: "trunk",
                named: "trunk",
            },
            Recorded {
                case: "c11: the default branch is develop, ahead of main",
                head: on("feat"),
                refs: refs(
                    &[("develop", 2, Some("origin/develop")), ("feat", 0, None)],
                    &[("origin/develop", 2), ("origin/main", 4)],
                    &["origin/develop"],
                ),
                default_branch: "develop",
                named: "origin/develop",
            },
            Recorded {
                case: "c11b: the same, the recorded default deleted",
                head: on("feat"),
                refs: refs(
                    &[("develop", 2, Some("origin/develop")), ("feat", 0, None)],
                    &[("origin/develop", 2), ("origin/main", 4)],
                    &[],
                ),
                default_branch: "develop",
                named: "develop",
            },
            Recorded {
                case: "c12: the default branch is develop, main is on the same commit",
                head: on("feat"),
                refs: refs(
                    &[("develop", 2, Some("origin/develop")), ("feat", 0, None)],
                    &[("origin/develop", 2), ("origin/main", 2)],
                    &[],
                ),
                default_branch: "develop",
                // Named wrong on purpose: with nothing recorded and two candidates equally near,
                // nothing tells the default branch apart; the user sets the base there.
                named: "origin/main",
            },
        ]
    }

    #[test]
    fn the_rule_names_the_recorded_base_for_each_recorded_shape_of_repository() {
        let recorded = recorded();
        assert_eq!(recorded.len(), 24);
        let mut wrong = Vec::new();
        for shape in &recorded {
            let named = default_base(&shape.head, &shape.refs);
            assert_eq!(named.as_deref(), Some(shape.named), "{}", shape.case);

            let default_branch = shape.default_branch;
            if ![default_branch, &format!("origin/{default_branch}")].contains(&shape.named) {
                wrong.push(&shape.case[..3]);
            }
        }
        assert_eq!(wrong, ["c12"]);
    }

    #[test]
    fn origin_is_asked_for_its_default_before_any_other_remote() {
        let both = refs(
            &[("feat", 0, None)],
            &[("fork/trunk", 1), ("origin/main", 5)],
            &["fork/trunk", "origin/main"],
        );
        assert_eq!(
            default_base(&on("feat"), &both).as_deref(),
            Some("origin/main")
        );

        let only_the_fork = refs(
            &[("feat", 0, None)],
            &[("fork/trunk", 1), ("origin/main", 5)],
            &["fork/trunk"],
        );
        assert_eq!(
            default_base(&on("feat"), &only_the_fork).as_deref(),
            Some("fork/trunk")
        );
    }

    #[test]
    fn among_the_usual_names_the_nearest_wins_and_the_earlier_name_wins_a_tie() {
        let near = refs(
            &[("feat", 0, None), ("main", 9, None), ("master", 7, None)],
            &[("origin/main", 8), ("origin/master", 3)],
            &[],
        );
        assert_eq!(
            default_base(&on("feat"), &near).as_deref(),
            Some("origin/master")
        );

        let tie = refs(
            &[("develop", 4, None), ("feat", 0, None), ("master", 4, None)],
            &[("origin/develop", 4)],
            &[],
        );
        assert_eq!(default_base(&on("feat"), &tie).as_deref(), Some("master"));

        let other_nearer = refs(
            &[("feat", 0, None), ("main", 6, None), ("spike", 1, None)],
            &[],
            &[],
        );
        assert_eq!(
            default_base(&on("feat"), &other_nearer).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn the_branch_that_is_checked_out_is_its_own_base_when_it_has_a_usual_name() {
        let alone = refs(&[("main", 0, None)], &[], &[]);
        assert_eq!(default_base(&on("main"), &alone).as_deref(), Some("main"));
        assert!(on_base_branch(&on("main"), "main", &alone));
    }

    #[test]
    fn of_all_other_branches_the_nearest_wins_but_never_the_one_checked_out_or_followed() {
        let others = refs(
            &[
                ("spike", 0, Some("origin/spike")),
                ("trunk", 5, None),
                ("wip", 2, None),
            ],
            &[("origin/spike", 0), ("origin/trunk", 2)],
            &[],
        );
        // `wip` and `origin/trunk` are equally near: a local branch comes first.
        assert_eq!(default_base(&on("spike"), &others).as_deref(), Some("wip"));
        assert_eq!(default_base(&detached(), &others).as_deref(), Some("spike"));
    }

    #[test]
    fn a_lone_branch_of_no_usual_name_has_no_base() {
        let alone = refs(&[("trunk", 0, None)], &[], &[]);
        assert_eq!(default_base(&on("trunk"), &alone), None);

        let with_its_copy = refs(
            &[("trunk", 0, Some("origin/trunk"))],
            &[("origin/trunk", 0)],
            &[],
        );
        assert_eq!(default_base(&on("trunk"), &with_its_copy), None);

        let unborn = Head::Branch {
            name: "main".to_owned(),
            commit: None,
        };
        assert_eq!(default_base(&unborn, &Refs::default()), None);
    }

    #[test]
    fn a_workspace_is_on_its_base_branch_when_that_branch_or_its_local_twin_is_checked_out() {
        let clone = refs(
            &[("feat", 2, None), ("main", 0, Some("origin/main"))],
            &[("origin/feat", 2), ("origin/main", 1), ("fork/main", 3)],
            &["origin/main"],
        );
        assert!(on_base_branch(&on("main"), "main", &clone));
        assert!(on_base_branch(&on("main"), "origin/main", &clone));
        assert!(on_base_branch(&on("main"), "fork/main", &clone));
        assert!(on_base_branch(&on("feat"), "origin/feat", &clone));

        assert!(!on_base_branch(&on("feat"), "main", &clone));
        assert!(!on_base_branch(&on("feat"), "origin/main", &clone));
        assert!(!on_base_branch(&on("main"), "gone/main", &clone));
        assert!(!on_base_branch(&detached(), "main", &clone));
        assert!(!on_base_branch(&detached(), "c0ffee", &clone));
        assert!(!on_base_branch(&on("main"), "c0ffee", &clone));
    }
}
