//! Ref resolution for sync: the base ref to integrate, upstreams, and git-dir state.

use std::path::PathBuf;

use crate::branch::BranchName;
use crate::error::GitError;
use crate::git::Git;

use super::report::Operation;

/// A ref by full name (used in git commands, unambiguous) and short name (for display).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefName {
    pub full: String,
    pub short: String,
}

impl RefName {
    fn local(branch: &BranchName) -> Self {
        Self {
            full: branch.full_ref(),
            short: branch.to_string(),
        }
    }

    fn remote(remote: &str, branch: &BranchName) -> Self {
        Self {
            full: format!("refs/remotes/{remote}/{branch}"),
            short: format!("{remote}/{branch}"),
        }
    }
}

/// Which refs may be trusted as the newest view of the base branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    /// Just fetched: the remote-tracking ref is authoritative.
    Fetched,
    /// Offline: prefer whichever of local / remote-tracking is newer.
    LocalFallback,
}

/// Resolve the ref to integrate as the base, or `None` if neither side exists.
///
/// `Fetched`: `<remote>/<base>` if present, else local `<base>`.
/// `LocalFallback`: the descendant of the two when one contains the other,
/// local `<base>` when they diverge.
pub fn resolve_base(
    git: &Git,
    remote: &str,
    base: &BranchName,
    source: BaseSource,
) -> Result<Option<RefName>, GitError> {
    let remote_ref = RefName::remote(remote, base);
    let local_ref = RefName::local(base);
    let has_remote = git.ref_exists(&remote_ref.full)?;
    let has_local = git.ref_exists(&local_ref.full)?;
    Ok(match (has_local, has_remote, source) {
        (_, true, BaseSource::Fetched) => Some(remote_ref),
        (true, true, BaseSource::LocalFallback) => {
            if git.is_ancestor(&local_ref.full, &remote_ref.full)? {
                Some(remote_ref)
            } else {
                Some(local_ref)
            }
        }
        (true, false, _) => Some(local_ref),
        (false, true, BaseSource::LocalFallback) => Some(remote_ref),
        (false, false, _) => None,
    })
}

/// The configured upstream of `branch` if its ref exists.
pub fn upstream(git: &Git, branch: &BranchName) -> Result<Option<RefName>, GitError> {
    let Some(short) = git.upstream_of(branch)? else {
        return Ok(None);
    };
    let spec = format!("{branch}@{{upstream}}");
    let (output, _) = git.output(["rev-parse", "--symbolic-full-name", &spec])?;
    let full = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((output.status.success() && !full.is_empty()).then_some(RefName { full, short }))
}

/// Absolute path of a per-worktree git-dir file (e.g. `MERGE_HEAD`).
pub fn git_path(git: &Git, name: &str) -> Result<PathBuf, GitError> {
    let rel = git.run(["rev-parse", "--git-path", name])?;
    Ok(git.dir().join(rel))
}

/// Operation left in progress in the worktree, if any.
pub fn operation_in_progress(git: &Git) -> Result<Option<Operation>, GitError> {
    let markers = [
        ("MERGE_HEAD", Operation::Merge),
        ("rebase-merge", Operation::Rebase),
        ("rebase-apply", Operation::Rebase),
        ("CHERRY_PICK_HEAD", Operation::CherryPick),
        ("REVERT_HEAD", Operation::Revert),
    ];
    for (name, op) in markers {
        if git_path(git, name)?.exists() {
            return Ok(Some(op));
        }
    }
    Ok(None)
}
