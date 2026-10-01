//! Sorting managed worktrees into clean candidates, skips and stale entries.

use std::fmt;
use std::path::PathBuf;

use crate::branch::BranchName;
use crate::git::{HeadKind, Worktree};
use crate::repo::Repo;

/// A worktree eligible for merge detection and removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub branch: BranchName,
    pub path: PathBuf,
    /// Commit the branch points at.
    pub tip: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Locked,
    Detached,
    /// Branch with no commits yet.
    Unborn,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Locked => "locked",
            Self::Detached => "detached HEAD",
            Self::Unborn => "no commits",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub branch: Option<BranchName>,
    pub reason: SkipReason,
}

#[derive(Debug, Default)]
pub struct Classified {
    pub targets: Vec<Target>,
    pub skipped: Vec<Skipped>,
    /// Registered worktrees whose directory is gone (`git worktree prune` removes them).
    pub prunable: Vec<PathBuf>,
}

/// Classify `worktrees` (from [`Repo::managed_worktrees`]). The primary checkout and
/// the base branch's worktree are excluded silently; they are never cleaned.
pub fn classify(repo: &Repo, base: &BranchName, worktrees: Vec<Worktree>) -> Classified {
    let mut out = Classified::default();
    for wt in worktrees {
        if repo.is_primary(&wt) {
            continue;
        }
        let branch = wt.branch().cloned();
        if branch.as_ref() == Some(base) {
            tracing::debug!(path = %wt.path.display(), branch = %base, "skipping base branch worktree");
            continue;
        }
        let skip = |reason| Skipped {
            path: wt.path.clone(),
            branch: branch.clone(),
            reason,
        };
        if wt.locked {
            out.skipped.push(skip(SkipReason::Locked));
            continue;
        }
        if wt.prunable {
            out.prunable.push(wt.path);
            continue;
        }
        match (wt.kind, wt.head) {
            (HeadKind::Branch(branch), Some(tip)) => out.targets.push(Target {
                branch,
                path: wt.path,
                tip,
            }),
            (HeadKind::Branch(_), None) => out.skipped.push(skip(SkipReason::Unborn)),
            (HeadKind::Detached | HeadKind::Bare, _) => {
                out.skipped.push(skip(SkipReason::Detached))
            }
        }
    }
    out.targets.sort_by(|a, b| a.path.cmp(&b.path));
    out.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    out.prunable.sort();
    out
}
