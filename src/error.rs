//! Structured error types shared across subsystems.

use std::path::PathBuf;

/// Failure while invoking or interpreting a `git` subprocess.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("failed to spawn `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`git {args}` failed in {dir} (exit {code:?}): {stderr}")]
    Failed {
        args: String,
        dir: PathBuf,
        code: Option<i32>,
        stderr: String,
    },
    #[error("unexpected output from `git {args}`: {detail}")]
    Parse { args: String, detail: String },
}

/// A string that is not a valid git branch name.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid branch name {name:?}: {reason}")]
pub struct BranchNameError {
    pub name: String,
    pub reason: &'static str,
}

/// Failure loading or parsing configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid config file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid base branch in config: {0}")]
    BaseBranch(#[from] BranchNameError),
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Failure locating the repository or resolving paths inside it.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("{0} is not inside a git repository")]
    NotARepo(PathBuf),
    #[error("could not determine the primary checkout: `git worktree list` returned no entries")]
    NoPrimary,
    #[error(
        "could not determine base branch: set `base_branch` in config or `git remote set-head {remote} -a`"
    )]
    NoBaseBranch { remote: String },
    #[error("branch {branch:?} maps to reserved path {path}")]
    ReservedPath { branch: String, path: PathBuf },
    #[error("failed to resolve {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Git(#[from] GitError),
}
