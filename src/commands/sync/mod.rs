//! `wt sync`: fetch once, then bring every managed worktree up to date with its
//! upstream and the base branch.

mod integrate;
mod refs;
mod report;
mod stash;
mod worktree;

use std::process::ExitCode;

use crate::branch::BranchName;
use crate::cli::{FetchPolicy, SyncArgs};
use crate::error::{GitError, RepoError};
use crate::git::Git;
use crate::progress::Progress;
use crate::repo::Repo;

pub use refs::RefName;
pub use report::{
    Applied, BaseStep, ConflictAction, FailReason, Integrated, Mode, Operation, SkipReason,
    StashOutcome, SyncReport, UpstreamStep, WorktreeOutcome, WorktreeReport,
};

use refs::{BaseSource, resolve_base};
use worktree::{Plan, sync_worktree};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("could not fetch {remote} and --remote-only was given; nothing was changed: {detail}")]
    Fetch { remote: String, detail: String },
    #[error("base branch {base:?} exists neither locally nor as {remote}/{base}")]
    NoBaseRef { base: String, remote: String },
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

pub fn run(repo: &Repo, args: &SyncArgs, progress: &Progress) -> anyhow::Result<ExitCode> {
    let report = sync(repo, args, progress);
    progress.finish();
    let report = report?;
    println!("{report}");
    Ok(if report.has_failures() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

pub fn sync(repo: &Repo, args: &SyncArgs, progress: &Progress) -> Result<SyncReport, SyncError> {
    let remote = repo.remote().to_owned();
    progress.phase(format!("fetching {remote}"));
    let mode = fetch(repo.git(), &remote, args.fetch_policy())?;
    let base_branch = repo.base_branch()?;
    let source = match mode {
        Mode::Remote => BaseSource::Fetched,
        Mode::Local { .. } => BaseSource::LocalFallback,
    };
    let base_ref = resolve_base(repo.git(), &remote, &base_branch, source)?.ok_or_else(|| {
        SyncError::NoBaseRef {
            base: base_branch.to_string(),
            remote: remote.clone(),
        }
    })?;
    tracing::info!(remote = %remote, base = %base_ref.short, mode = ?mode, "sync plan");

    let plan = Plan {
        base_branch,
        base_ref,
        integration: args.integration(),
        stash_pop: args.stash_pop,
    };
    let mut worktrees = repo.managed_worktrees()?;
    // Base worktrees first so its local branch is current before others integrate it.
    worktrees.sort_by_key(|wt| !is_on(wt.branch(), &plan.base_branch));
    // Sequential on purpose: all worktrees share one git dir and stash stack.
    progress.items(worktrees.len());
    let reports = worktrees
        .iter()
        .map(|wt| {
            progress.item(item_label(wt));
            let report = sync_worktree(wt, &plan, progress);
            progress.item_done();
            report
        })
        .collect();

    Ok(SyncReport {
        remote,
        mode,
        base: plan.base_ref,
        worktrees: reports,
    })
}

/// Branch name, or the path for worktrees without one (detached/bare).
fn item_label(wt: &crate::git::Worktree) -> String {
    wt.branch()
        .map_or_else(|| wt.path.display().to_string(), ToString::to_string)
}

fn is_on(branch: Option<&BranchName>, base: &BranchName) -> bool {
    branch == Some(base)
}

/// `git fetch --prune <remote>` once in the primary; worktrees share its refs.
fn fetch(git: &Git, remote: &str, policy: FetchPolicy) -> Result<Mode, SyncError> {
    let (output, rendered) = git.output(["fetch", "--prune", "--quiet", remote])?;
    if output.status.success() {
        tracing::info!(remote, "fetched");
        return Ok(Mode::Remote);
    }
    let detail = match git.failed(rendered, &output) {
        GitError::Failed { stderr, .. } if !stderr.is_empty() => {
            // The first `fatal:` line names the problem; later lines are generic advice.
            let first = stderr.lines().find(|l| !l.trim().is_empty());
            stderr
                .lines()
                .find(|l| l.starts_with("fatal:"))
                .or(first)
                .unwrap_or_default()
                .to_owned()
        }
        other => other.to_string(),
    };
    match policy {
        FetchPolicy::RemoteOnly => Err(SyncError::Fetch {
            remote: remote.to_owned(),
            detail,
        }),
        FetchPolicy::FallBackToLocal => {
            tracing::warn!(remote, error = %detail, "fetch failed; using local refs");
            Ok(Mode::Local {
                fetch_error: detail,
            })
        }
    }
}
