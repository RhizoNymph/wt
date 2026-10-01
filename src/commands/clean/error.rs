//! Typed errors for `wt clean`.

use std::path::PathBuf;

use crate::error::GitError;

/// A failure while cleaning one worktree. Recorded in the report; other worktrees continue.
#[derive(Debug, thiserror::Error)]
pub enum CleanError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("failed to read {path}: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to copy {from} to {to}: {source}")]
    Copy {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("no unused scratch directory for {base} after {attempts} attempts")]
    ScratchExhausted { base: PathBuf, attempts: u32 },
    #[error("worktree removed, but deleting branch {branch} failed: {source}")]
    BranchDelete {
        branch: String,
        #[source]
        source: GitError,
    },
}

/// Why the GitHub CLI could not provide merged pull requests.
#[derive(Debug, thiserror::Error)]
pub enum GhError {
    #[error("`{program}` is not installed")]
    NotInstalled { program: String },
    #[error("failed to run `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{program} pr list` failed (exit {code:?}): {stderr}")]
    Failed {
        program: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("unexpected output from `{program} pr list`: {source}")]
    Parse {
        program: String,
        #[source]
        source: serde_json::Error,
    },
}
