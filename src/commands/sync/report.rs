//! Typed per-worktree outcomes and the printed `SyncReport`.

use std::fmt;
use std::path::PathBuf;

use crate::branch::BranchName;

use super::refs::RefName;

/// Whether the run fetched from the remote or fell back to local refs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Remote,
    /// Fetch failed; remote-tracking refs are whatever was last fetched.
    Local {
        fetch_error: String,
    },
}

/// How commits from another ref were brought into the branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    FastForward,
    Merged,
    Rebased,
}

/// A successful integration of `source` that brought in `commits` new commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integrated {
    pub how: Applied,
    pub source: String,
    pub commits: u64,
}

impl fmt::Display for Integrated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (verb, prep) = match self.how {
            Applied::FastForward => ("fast-forwarded", "from"),
            Applied::Merged => ("merged", ""),
            Applied::Rebased => ("rebased", "onto"),
        };
        if prep.is_empty() {
            write!(f, "{verb} {} (+{})", self.source, self.commits)
        } else {
            write!(f, "{verb} {prep} {} (+{})", self.source, self.commits)
        }
    }
}

/// Result of pulling the branch's own upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamStep {
    NoUpstream,
    UpToDate,
    Integrated(Integrated),
}

/// Result of integrating the base branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseStep {
    /// The worktree is on the base branch itself; only its upstream is pulled.
    IsBase,
    UpToDate,
    Integrated(Integrated),
}

/// Kind of git operation left in progress in a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    /// Unmerged index entries without a recognisable operation.
    Conflicts,
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
            Self::Conflicts => "unresolved conflicts",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Detached,
    Bare,
    /// The worktree directory no longer exists.
    Missing,
    Dirty,
    InProgress(Operation),
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Detached => f.write_str("detached HEAD"),
            Self::Bare => f.write_str("bare repository"),
            Self::Missing => f.write_str("directory missing"),
            Self::Dirty => f.write_str("dirty (use --stash-pop)"),
            Self::InProgress(op) => write!(f, "{op} in progress"),
        }
    }
}

/// Which history-rewriting action hit a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    Merge,
    Rebase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailReason {
    /// `aborted` is false when `git merge/rebase --abort` itself failed and the
    /// worktree is left mid-operation.
    Conflict {
        action: ConflictAction,
        source: String,
        aborted: bool,
    },
    Stash(String),
    Git(String),
}

impl fmt::Display for FailReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict {
                action,
                source,
                aborted,
            } => {
                let what = match action {
                    ConflictAction::Merge => format!("conflict merging {source}"),
                    ConflictAction::Rebase => format!("conflict rebasing onto {source}"),
                };
                let state = if *aborted {
                    "aborted"
                } else {
                    "abort FAILED, resolve manually"
                };
                write!(f, "{what} ({state})")
            }
            Self::Stash(e) => write!(f, "could not stash changes: {e}"),
            Self::Git(e) => write!(f, "{e}"),
        }
    }
}

/// What happened to stashed local changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashOutcome {
    /// The worktree was clean; nothing was stashed.
    NotNeeded,
    Restored,
    /// Restored, but the stash entry could not be dropped.
    RestoredNotDropped {
        sha: String,
        error: String,
    },
    /// Applying produced conflicts; the entry is kept.
    Conflicted {
        sha: String,
    },
    /// Not applied because the worktree was left mid-operation; the entry is kept.
    Kept {
        sha: String,
    },
}

impl StashOutcome {
    fn needs_attention(&self) -> bool {
        matches!(self, Self::Conflicted { .. } | Self::Kept { .. })
    }
}

impl fmt::Display for StashOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotNeeded => Ok(()),
            Self::Restored => f.write_str("stash restored"),
            Self::RestoredNotDropped { sha, error } => {
                write!(
                    f,
                    "stash restored, but entry {sha} was not dropped: {error}"
                )
            }
            Self::Conflicted { sha } => write!(
                f,
                "restoring stash conflicted; changes kept in stash {sha} (git stash apply {sha})"
            ),
            Self::Kept { sha } => write!(f, "local changes kept in stash {sha}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeOutcome {
    Synced {
        upstream: UpstreamStep,
        base: BaseStep,
        stash: StashOutcome,
    },
    Skipped(SkipReason),
    Failed {
        /// Upstream step, if it completed before the failure.
        upstream: Option<UpstreamStep>,
        reason: FailReason,
        stash: StashOutcome,
    },
}

impl WorktreeOutcome {
    /// Failures and unrestored stashes; skips are not failures.
    pub fn needs_attention(&self) -> bool {
        match self {
            Self::Synced { stash, .. } => stash.needs_attention(),
            Self::Skipped(_) => false,
            Self::Failed { .. } => true,
        }
    }
}

fn upstream_part(step: &UpstreamStep) -> Option<String> {
    match step {
        UpstreamStep::Integrated(i) => Some(i.to_string()),
        UpstreamStep::NoUpstream | UpstreamStep::UpToDate => None,
    }
}

impl fmt::Display for WorktreeOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Synced {
                upstream,
                base,
                stash,
            } => {
                let mut parts: Vec<String> = upstream_part(upstream).into_iter().collect();
                if let BaseStep::Integrated(i) = base {
                    parts.push(i.to_string());
                }
                if parts.is_empty() {
                    f.write_str("up to date")?;
                } else {
                    f.write_str(&parts.join(", then "))?;
                }
                if *upstream == UpstreamStep::NoUpstream {
                    f.write_str(" (no upstream)")?;
                }
                if *stash != StashOutcome::NotNeeded {
                    write!(f, "; {stash}")?;
                }
                Ok(())
            }
            Self::Skipped(reason) => write!(f, "skipped: {reason}"),
            Self::Failed {
                upstream,
                reason,
                stash,
            } => {
                if let Some(done) = upstream.as_ref().and_then(upstream_part) {
                    write!(f, "{done}, then ")?;
                }
                write!(f, "failed: {reason}")?;
                if *stash != StashOutcome::NotNeeded {
                    write!(f, "; {stash}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeReport {
    /// `None` for detached/bare worktrees.
    pub branch: Option<BranchName>,
    pub path: PathBuf,
    pub outcome: WorktreeOutcome,
}

impl WorktreeReport {
    fn label(&self) -> String {
        match (&self.branch, &self.outcome) {
            (Some(b), _) => b.to_string(),
            (None, WorktreeOutcome::Skipped(SkipReason::Bare)) => "(bare)".into(),
            (None, _) => "(detached)".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncReport {
    pub remote: String,
    pub mode: Mode,
    pub base: RefName,
    pub worktrees: Vec<WorktreeReport>,
}

impl SyncReport {
    pub fn has_failures(&self) -> bool {
        self.worktrees.iter().any(|w| w.outcome.needs_attention())
    }
}

impl fmt::Display for SyncReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.mode {
            Mode::Remote => writeln!(
                f,
                "Remote mode: fetched {}; base {}",
                self.remote, self.base.short
            )?,
            Mode::Local { fetch_error } => {
                writeln!(
                    f,
                    "Local mode: could not fetch {} ({fetch_error}); using local refs, which may be stale; base {}",
                    self.remote, self.base.short
                )?;
            }
        }
        let labels: Vec<String> = self.worktrees.iter().map(WorktreeReport::label).collect();
        let width = labels.iter().map(String::len).max().unwrap_or(0);
        for (wt, label) in self.worktrees.iter().zip(&labels) {
            writeln!(
                f,
                "  {label:<width$}  {}  {}",
                wt.path.display(),
                wt.outcome
            )?;
        }
        let count = |pred: fn(&WorktreeOutcome) -> bool| {
            self.worktrees.iter().filter(|w| pred(&w.outcome)).count()
        };
        let skipped = count(|o| matches!(o, WorktreeOutcome::Skipped(_)));
        let failed = count(WorktreeOutcome::needs_attention);
        let synced = self.worktrees.len() - skipped - failed;
        write!(f, "{synced} synced, {skipped} skipped, {failed} failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integrated(how: Applied, source: &str, commits: u64) -> Integrated {
        Integrated {
            how,
            source: source.into(),
            commits,
        }
    }

    #[test]
    fn synced_outcomes_render() {
        let o = WorktreeOutcome::Synced {
            upstream: UpstreamStep::Integrated(integrated(
                Applied::FastForward,
                "origin/feat/x",
                3,
            )),
            base: BaseStep::Integrated(integrated(Applied::Merged, "origin/main", 5)),
            stash: StashOutcome::Restored,
        };
        assert_eq!(
            o.to_string(),
            "fast-forwarded from origin/feat/x (+3), then merged origin/main (+5); stash restored"
        );
        let o = WorktreeOutcome::Synced {
            upstream: UpstreamStep::NoUpstream,
            base: BaseStep::UpToDate,
            stash: StashOutcome::NotNeeded,
        };
        assert_eq!(o.to_string(), "up to date (no upstream)");
        assert!(!o.needs_attention());
        let o = WorktreeOutcome::Synced {
            upstream: UpstreamStep::UpToDate,
            base: BaseStep::Integrated(integrated(Applied::Rebased, "origin/main", 2)),
            stash: StashOutcome::NotNeeded,
        };
        assert_eq!(o.to_string(), "rebased onto origin/main (+2)");
    }

    #[test]
    fn failures_and_skips_render() {
        let o = WorktreeOutcome::Failed {
            upstream: None,
            reason: FailReason::Conflict {
                action: ConflictAction::Merge,
                source: "origin/main".into(),
                aborted: true,
            },
            stash: StashOutcome::NotNeeded,
        };
        assert_eq!(
            o.to_string(),
            "failed: conflict merging origin/main (aborted)"
        );
        assert!(o.needs_attention());
        let o = WorktreeOutcome::Skipped(SkipReason::Dirty);
        assert_eq!(o.to_string(), "skipped: dirty (use --stash-pop)");
        assert!(!o.needs_attention());
    }

    #[test]
    fn unrestored_stash_needs_attention() {
        let o = WorktreeOutcome::Synced {
            upstream: UpstreamStep::UpToDate,
            base: BaseStep::IsBase,
            stash: StashOutcome::Conflicted { sha: "abc".into() },
        };
        assert!(o.needs_attention());
        assert!(o.to_string().contains("abc"));
    }
}
