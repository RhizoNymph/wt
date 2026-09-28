//! Safe stashing on the stash stack shared by every worktree of a repository.
//!
//! Entries are identified by commit SHA, never by position: another worktree (or
//! the user) may push entries concurrently, so `stash@{0}` is not necessarily ours.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::GitError;
use crate::git::Git;

use super::report::StashOutcome;

#[derive(Debug, thiserror::Error)]
pub enum StashError {
    #[error("`git stash push` did not create an entry tagged {token:?}")]
    NotCreated { token: String },
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Local changes held in a specific stash entry.
///
/// Consume it with [`StashedChanges::restore`] or [`StashedChanges::keep`]; dropping it
/// otherwise logs an error with the SHA so the changes can still be recovered.
#[derive(Debug)]
#[must_use = "stashed changes must be restored or reported"]
pub struct StashedChanges {
    sha: String,
    armed: bool,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{n}", std::process::id())
}

impl StashedChanges {
    /// Stash tracked and untracked changes in `git`'s worktree and resolve the entry's SHA.
    pub fn push(git: &Git) -> Result<Self, StashError> {
        let token = unique_token();
        let message = format!("wt-sync {token}");
        git.run([
            "stash",
            "push",
            "--include-untracked",
            "--quiet",
            "-m",
            &message,
        ])?;
        let sha = find_by_message(git, &message)?.ok_or(StashError::NotCreated { token })?;
        tracing::info!(dir = %git.dir().display(), sha = %sha, "stashed local changes");
        Ok(Self { sha, armed: true })
    }

    /// Give up on restoring (the worktree is not in a state to apply into) and hand
    /// back the SHA for the report.
    pub fn keep(mut self) -> StashOutcome {
        self.armed = false;
        tracing::warn!(sha = %self.sha, "leaving local changes in stash");
        StashOutcome::Kept {
            sha: self.sha.clone(),
        }
    }

    /// Apply the entry (with its index when possible) and drop exactly that entry.
    /// On conflict the entry is kept and its SHA reported.
    pub fn restore(mut self, git: &Git) -> StashOutcome {
        self.armed = false;
        let sha = self.sha.clone();
        match apply(git, &sha) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(sha = %sha, dir = %git.dir().display(), "stash apply conflicted");
                return StashOutcome::Conflicted { sha };
            }
            Err(error) => {
                tracing::warn!(sha = %sha, error = %error, "stash apply failed");
                return StashOutcome::Conflicted { sha };
            }
        }
        match drop_entry(git, &sha) {
            Ok(()) => StashOutcome::Restored,
            Err(error) => {
                tracing::warn!(sha = %sha, error = %error, "could not drop stash entry");
                StashOutcome::RestoredNotDropped {
                    sha,
                    error: error.to_string(),
                }
            }
        }
    }
}

impl Drop for StashedChanges {
    fn drop(&mut self) {
        if self.armed {
            tracing::error!(
                sha = %self.sha,
                "stashed changes were never restored; recover with `git stash apply <sha>`"
            );
        }
    }
}

/// Apply `sha`; `Ok(false)` means it did not apply cleanly (entry must be kept).
fn apply(git: &Git, sha: &str) -> Result<bool, GitError> {
    let (output, _) = git.output(["stash", "apply", "--index", "--quiet", sha])?;
    if output.status.success() {
        return Ok(true);
    }
    // `--index` refuses when the staged part no longer applies; retry without it
    // only if nothing was written, so a partial apply is never applied twice.
    if !git.status()?.is_clean() {
        return Ok(false);
    }
    tracing::debug!(sha, "stash apply --index failed; retrying without --index");
    let (output, _) = git.output(["stash", "apply", "--quiet", sha])?;
    Ok(output.status.success())
}

/// Stash entries as `(selector, sha, subject)`, newest first.
fn entries(git: &Git) -> Result<Vec<(String, String, String)>, GitError> {
    let out = git.run(["stash", "list", "--format=%gd%x00%H%x00%s"])?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\0');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(sel), Some(sha), Some(subject)) => {
                    Some((sel.to_owned(), sha.to_owned(), subject.to_owned()))
                }
                _ => None,
            }
        })
        .collect())
}

/// SHA of the entry whose subject carries `message` (subject is `On <branch>: <message>`).
fn find_by_message(git: &Git, message: &str) -> Result<Option<String>, GitError> {
    // Fast path: the entry just pushed is normally on top.
    if let Some(top) = git.rev_parse("refs/stash")? {
        let subject = git.run(["log", "-1", "--format=%s", &top])?;
        if subject.ends_with(message) {
            return Ok(Some(top));
        }
    }
    Ok(entries(git)?
        .into_iter()
        .find(|(_, _, subject)| subject.ends_with(message))
        .map(|(_, sha, _)| sha))
}

#[derive(Debug, thiserror::Error)]
enum DropError {
    #[error("entry {0} no longer on the stash stack")]
    Missing(String),
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Drop the entry whose commit is `sha`, wherever it currently sits.
fn drop_entry(git: &Git, sha: &str) -> Result<(), DropError> {
    let selector = entries(git)?
        .into_iter()
        .find(|(_, entry, _)| entry == sha)
        .map(|(sel, _, _)| sel)
        .ok_or_else(|| DropError::Missing(sha.to_owned()))?;
    git.run(["stash", "drop", "--quiet", &selector])?;
    tracing::debug!(sha, selector = %selector, "dropped stash entry");
    Ok(())
}
