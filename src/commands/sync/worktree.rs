//! Syncing a single worktree: preflight, optional stash, upstream, base, restore.

use crate::branch::BranchName;
use crate::cli::Integration;
use crate::error::GitError;
use crate::git::{ChangeKind, Git, HeadKind, Worktree};

use super::integrate::{IntegrateError, integrate};
use super::refs::{RefName, operation_in_progress, upstream};
use super::report::{
    BaseStep, FailReason, Operation, SkipReason, StashOutcome, UpstreamStep, WorktreeOutcome,
    WorktreeReport,
};
use super::stash::StashedChanges;

/// Inputs shared by every worktree in one run.
#[derive(Debug)]
pub struct Plan {
    pub base_branch: BranchName,
    pub base_ref: RefName,
    pub integration: Integration,
    pub stash_pop: bool,
}

/// Why the integration steps stopped early.
struct StepFailure {
    upstream: Option<UpstreamStep>,
    reason: FailReason,
    /// The worktree was left mid-operation, so stashed changes must not be applied.
    left_in_progress: bool,
}

impl StepFailure {
    fn new(upstream: Option<UpstreamStep>, err: IntegrateError) -> Self {
        match err {
            IntegrateError::Conflict {
                action,
                source_ref,
                aborted,
            } => Self {
                upstream,
                reason: FailReason::Conflict {
                    action,
                    source: source_ref,
                    aborted,
                },
                left_in_progress: !aborted,
            },
            IntegrateError::Git(e) => Self::git(upstream, e),
        }
    }

    fn git(upstream: Option<UpstreamStep>, err: GitError) -> Self {
        Self {
            upstream,
            reason: FailReason::Git(err.to_string()),
            left_in_progress: false,
        }
    }
}

pub fn sync_worktree(wt: &Worktree, plan: &Plan) -> WorktreeReport {
    let branch = wt.branch().cloned();
    let outcome = match &wt.kind {
        HeadKind::Detached => WorktreeOutcome::Skipped(SkipReason::Detached),
        HeadKind::Bare => WorktreeOutcome::Skipped(SkipReason::Bare),
        HeadKind::Branch(b) if !wt.path.is_dir() => {
            tracing::warn!(branch = %b, path = %wt.path.display(), "worktree directory missing");
            WorktreeOutcome::Skipped(SkipReason::Missing)
        }
        HeadKind::Branch(b) => sync_branch(&Git::new(&wt.path), b, plan),
    };
    tracing::info!(path = %wt.path.display(), outcome = %outcome, "worktree synced");
    WorktreeReport {
        branch,
        path: wt.path.clone(),
        outcome,
    }
}

fn failed_git(err: impl ToString) -> WorktreeOutcome {
    WorktreeOutcome::Failed {
        upstream: None,
        reason: FailReason::Git(err.to_string()),
        stash: StashOutcome::NotNeeded,
    }
}

fn sync_branch(git: &Git, branch: &BranchName, plan: &Plan) -> WorktreeOutcome {
    let status = match git.status() {
        Ok(s) => s,
        Err(e) => return failed_git(e),
    };
    let in_progress = match operation_in_progress(git) {
        Ok(op) => op,
        Err(e) => return failed_git(e),
    };
    let has_conflicts = status
        .entries
        .iter()
        .any(|e| e.kind == ChangeKind::Conflicted);
    if let Some(op) = in_progress.or(has_conflicts.then_some(Operation::Conflicts)) {
        return WorktreeOutcome::Skipped(SkipReason::InProgress(op));
    }

    let stash = if status.is_clean() {
        None
    } else if !plan.stash_pop {
        return WorktreeOutcome::Skipped(SkipReason::Dirty);
    } else {
        match StashedChanges::push(git) {
            Ok(s) => Some(s),
            Err(e) => {
                return WorktreeOutcome::Failed {
                    upstream: None,
                    reason: FailReason::Stash(e.to_string()),
                    stash: StashOutcome::NotNeeded,
                };
            }
        }
    };

    let result = run_steps(git, branch, plan);

    let stash_outcome = match (stash, &result) {
        (None, _) => StashOutcome::NotNeeded,
        (Some(s), Err(f)) if f.left_in_progress => s.keep(),
        (Some(s), _) => s.restore(git),
    };
    match result {
        Ok((upstream, base)) => WorktreeOutcome::Synced {
            upstream,
            base,
            stash: stash_outcome,
        },
        Err(f) => WorktreeOutcome::Failed {
            upstream: f.upstream,
            reason: f.reason,
            stash: stash_outcome,
        },
    }
}

/// Pull the upstream, then integrate the base. Order matters: the branch first
/// catches up with its own remote history so base integration happens on top of it.
fn run_steps(
    git: &Git,
    branch: &BranchName,
    plan: &Plan,
) -> Result<(UpstreamStep, BaseStep), StepFailure> {
    let upstream_step = match upstream(git, branch).map_err(|e| StepFailure::git(None, e))? {
        None => UpstreamStep::NoUpstream,
        Some(up) => match integrate(git, &up, plan.integration) {
            Ok(Some(done)) => UpstreamStep::Integrated(done),
            Ok(None) => UpstreamStep::UpToDate,
            Err(e) => return Err(StepFailure::new(None, e)),
        },
    };
    if *branch == plan.base_branch {
        return Ok((upstream_step, BaseStep::IsBase));
    }
    let base_step = match integrate(git, &plan.base_ref, plan.integration) {
        Ok(Some(done)) => BaseStep::Integrated(done),
        Ok(None) => BaseStep::UpToDate,
        Err(e) => return Err(StepFailure::new(Some(upstream_step), e)),
    };
    Ok((upstream_step, base_step))
}
