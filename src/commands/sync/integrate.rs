//! Bringing another ref into the checked-out branch: fast-forward, merge or rebase,
//! aborting on conflict so the worktree returns to its prior state.

use crate::cli::Integration;
use crate::error::GitError;
use crate::git::Git;

use super::refs::{RefName, git_path};
use super::report::{Applied, ConflictAction, Integrated};

#[derive(Debug, thiserror::Error)]
pub enum IntegrateError {
    /// The operation stopped on conflicts; `aborted` says whether rollback succeeded.
    #[error("conflict integrating {source_ref} (aborted: {aborted})")]
    Conflict {
        action: ConflictAction,
        source_ref: String,
        aborted: bool,
    },
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Bring `target` into HEAD. Returns `None` when HEAD already contains it.
///
/// Behind-only → `merge --ff-only`; diverged → merge (`--no-edit`) or rebase.
pub fn integrate(
    git: &Git,
    target: &RefName,
    integration: Integration,
) -> Result<Option<Integrated>, IntegrateError> {
    let div = git.divergence("HEAD", &target.full)?;
    if div.behind == 0 {
        return Ok(None);
    }
    let how = if div.ahead == 0 {
        git.run(["merge", "--ff-only", "--quiet", &target.full])?;
        Applied::FastForward
    } else {
        match integration {
            Integration::Merge => {
                merge(git, target)?;
                Applied::Merged
            }
            Integration::Rebase => {
                rebase(git, target)?;
                Applied::Rebased
            }
        }
    };
    tracing::info!(
        dir = %git.dir().display(),
        source = %target.short,
        commits = div.behind,
        how = ?how,
        "integrated"
    );
    Ok(Some(Integrated {
        how,
        source: target.short.clone(),
        commits: div.behind,
    }))
}

fn merge(git: &Git, target: &RefName) -> Result<(), IntegrateError> {
    // `--ff` overrides a user `merge.ff=only`/`false`; we only get here when diverged.
    let (output, rendered) = git.output(["merge", "--no-edit", "--ff", "--quiet", &target.full])?;
    if output.status.success() {
        return Ok(());
    }
    if !git_path(git, "MERGE_HEAD")?.exists() {
        // Refused before starting (e.g. would overwrite files): nothing changed.
        return Err(git.failed(rendered, &output).into());
    }
    let aborted = abort(git, ["merge", "--abort"]);
    Err(IntegrateError::Conflict {
        action: ConflictAction::Merge,
        source_ref: target.short.clone(),
        aborted,
    })
}

fn rebase(git: &Git, target: &RefName) -> Result<(), IntegrateError> {
    let (output, rendered) = git.output(["rebase", "--quiet", &target.full])?;
    if output.status.success() {
        return Ok(());
    }
    let started =
        git_path(git, "rebase-merge")?.exists() || git_path(git, "rebase-apply")?.exists();
    if !started {
        return Err(git.failed(rendered, &output).into());
    }
    let aborted = abort(git, ["rebase", "--abort"]);
    Err(IntegrateError::Conflict {
        action: ConflictAction::Rebase,
        source_ref: target.short.clone(),
        aborted,
    })
}

fn abort(git: &Git, args: [&str; 2]) -> bool {
    match git.run(args) {
        Ok(_) => {
            tracing::warn!(dir = %git.dir().display(), op = args[0], "conflict; aborted");
            true
        }
        Err(error) => {
            tracing::warn!(dir = %git.dir().display(), op = args[0], error = %error, "abort failed");
            false
        }
    }
}
