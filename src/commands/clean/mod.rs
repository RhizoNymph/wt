//! `wt clean`: remove worktrees whose branches have been merged.
//!
//! See `docs/features/clean.md` for the full flow and the merge-detection safety rule.

mod candidates;
mod error;
mod github;
mod merged;
mod remove;
mod report;
mod scratch;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;

use crate::cli::{CleanArgs, DirtyPolicy};
use crate::git::Git;
use crate::repo::Repo;
use crate::shell;

pub use error::{CleanError, GhError};
pub use merged::{MergeEvidence, MergeStatus};
pub use report::CleanReport;

use candidates::Target;
use merged::{Detection, MergeDetector};
use remove::Force;
use report::{
    DetectionMode, DirtyCounts, Failure, FetchOutcome, Outcome, Removed, Scratched, SkippedDirty,
};
use scratch::ScratchPlan;

/// Everything per-worktree processing needs.
struct Ctx<'a> {
    repo: &'a Repo,
    detector: MergeDetector<'a>,
    policy: DirtyPolicy,
    dry_run: bool,
}

pub fn run(repo: &Repo, args: &CleanArgs) -> anyhow::Result<ExitCode> {
    // Captured first: the directory may not exist once its worktree is removed.
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|d| d.canonicalize().ok());
    let base = repo.base_branch().context("resolving the base branch")?;
    let fetch = fetch_remote(repo);
    let detection = detect_source(repo.primary());
    let detection_mode = match &detection {
        Detection::GitHub(index) => DetectionMode::GitHub {
            merged_prs: index.merged_count(),
        },
        Detection::GitOnly(reason) => DetectionMode::GitOnly {
            reason: reason.to_string(),
        },
    };
    let ctx = Ctx {
        repo,
        detector: MergeDetector::new(repo.git(), detection, repo.remote(), &base)
            .context("resolving base refs")?,
        policy: args.dirty_policy(),
        dry_run: args.dry_run,
    };
    tracing::debug!(base = %base, policy = ?ctx.policy, dry_run = ctx.dry_run, detection = ?ctx.detector.detection(), "clean");

    let worktrees = repo.managed_worktrees().context("listing worktrees")?;
    let classified = candidates::classify(repo, &base, worktrees);

    let mut report = CleanReport::new(ctx.dry_run, detection_mode, fetch);
    report.skipped = classified.skipped;
    prune_stale(&ctx, classified.prunable, &mut report);
    for target in classified.targets {
        match process(&ctx, &target) {
            Ok(outcome) => report.record(outcome),
            Err(error) => {
                tracing::error!(branch = %target.branch, path = %target.path.display(), error = %error, "clean failed");
                report.errors.push(Failure {
                    branch: Some(target.branch),
                    path: target.path,
                    error,
                });
            }
        }
    }

    print!("{report}");
    if let Some(cwd) = cwd {
        let inside_removed = report
            .removed_paths()
            .any(|p| cwd.starts_with(p) && !cwd.exists());
        if inside_removed {
            shell::request_cd(repo.primary()).context("requesting a directory change")?;
        }
    }
    Ok(if report.has_errors() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Refresh remote-tracking refs once; failure is not fatal (local refs still work).
fn fetch_remote(repo: &Repo) -> FetchOutcome {
    let remote = repo.remote().to_owned();
    match repo.git().run(["fetch", "--quiet", remote.as_str()]) {
        Ok(_) => FetchOutcome::Fetched { remote },
        Err(e) => {
            tracing::warn!(remote = %remote, error = %e, "fetch failed; using local refs");
            FetchOutcome::Failed {
                remote,
                error: e.to_string(),
            }
        }
    }
}

fn detect_source(primary: &Path) -> Detection {
    match github::merged_prs(primary) {
        Ok(index) => {
            tracing::info!(
                merged_prs = index.merged_count(),
                "using GitHub pull requests"
            );
            Detection::GitHub(index)
        }
        Err(e @ GhError::NotInstalled { .. }) => {
            tracing::info!(reason = %e, "GitHub CLI unavailable; using git ancestry only");
            Detection::GitOnly(e)
        }
        Err(e) => {
            tracing::warn!(reason = %e, "GitHub CLI failed; using git ancestry only");
            Detection::GitOnly(e)
        }
    }
}

fn prune_stale(ctx: &Ctx<'_>, prunable: Vec<PathBuf>, report: &mut CleanReport) {
    if prunable.is_empty() {
        return;
    }
    if !ctx.dry_run {
        if let Err(e) = ctx.repo.git().run(["worktree", "prune"]) {
            report.errors.push(Failure {
                branch: None,
                path: ctx.repo.root().to_path_buf(),
                error: e.into(),
            });
            return;
        }
        tracing::info!(count = prunable.len(), "pruned stale worktree entries");
    }
    report.pruned = prunable;
}

fn process(ctx: &Ctx<'_>, target: &Target) -> Result<Outcome, CleanError> {
    let evidence = match ctx.detector.evaluate(&target.branch, &target.tip)? {
        MergeStatus::Merged(e) => e,
        MergeStatus::NotMerged => {
            tracing::debug!(branch = %target.branch, "not merged");
            return Ok(Outcome::NotMerged);
        }
    };
    tracing::debug!(branch = %target.branch, evidence = %evidence, "merged");
    let status = Git::new(&target.path).status()?;
    if status.is_clean() {
        remove_unless_dry(ctx, target, Force::No)?;
        return Ok(Outcome::Removed(Removed {
            branch: target.branch.clone(),
            path: target.path.clone(),
            evidence,
            discarded: None,
        }));
    }
    let counts = DirtyCounts::of(&status);
    match ctx.policy {
        DirtyPolicy::Skip => Ok(Outcome::SkippedDirty(SkippedDirty {
            branch: target.branch.clone(),
            path: target.path.clone(),
            evidence,
            counts,
        })),
        DirtyPolicy::Delete => {
            remove_unless_dry(ctx, target, Force::Yes)?;
            Ok(Outcome::Removed(Removed {
                branch: target.branch.clone(),
                path: target.path.clone(),
                evidence,
                discarded: Some(counts),
            }))
        }
        DirtyPolicy::Scratch => {
            let plan = ScratchPlan::new(&target.path, &status)?;
            let dest = save_scratch(ctx, target, &plan)?;
            // Only reached once every file is copied: a failed copy keeps the worktree.
            remove_unless_dry(ctx, target, Force::Yes)?;
            Ok(Outcome::Scratched(Scratched {
                branch: target.branch.clone(),
                path: target.path.clone(),
                evidence,
                dest,
                copied: plan.files.len(),
                not_copied: plan.not_copied,
            }))
        }
    }
}

fn save_scratch(
    ctx: &Ctx<'_>,
    target: &Target,
    plan: &ScratchPlan,
) -> Result<Option<PathBuf>, CleanError> {
    if plan.files.is_empty() {
        return Ok(None);
    }
    let base = scratch::base_dest(&ctx.repo.scratch_dir(), &target.branch);
    let dest = if ctx.dry_run {
        scratch::preview_dest(&base)?
    } else {
        scratch::save(plan, &target.path, &base)?
    };
    Ok(Some(dest))
}

fn remove_unless_dry(ctx: &Ctx<'_>, target: &Target, force: Force) -> Result<(), CleanError> {
    if ctx.dry_run {
        return Ok(());
    }
    remove::remove(ctx.repo.git(), target, force, ctx.repo.root())
}
