//! Thin, typed wrapper around the `git` CLI.

pub mod status;
pub mod worktree;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::branch::BranchName;
use crate::error::GitError;

pub use status::{ChangeKind, StatusEntry, WorktreeStatus};
pub use worktree::{HeadKind, Worktree};

/// Runs git commands with a fixed working directory.
///
/// Every invocation sets `LC_ALL=C` (stable, parseable messages) and
/// `GIT_TERMINAL_PROMPT=0` (network operations fail instead of prompting).
#[derive(Debug, Clone)]
pub struct Git {
    dir: PathBuf,
}

/// Commits unique to each side of a comparison (`git rev-list --left-right --count a...b`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    pub ahead: u64,
    pub behind: u64,
}

impl Divergence {
    pub fn is_diverged(&self) -> bool {
        self.ahead > 0 && self.behind > 0
    }
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn command<I, S>(&self, args: I) -> (Command, String)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.dir)
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0");
        let mut rendered = Vec::new();
        for arg in args {
            rendered.push(arg.as_ref().to_string_lossy().into_owned());
            cmd.arg(arg);
        }
        (cmd, rendered.join(" "))
    }

    /// Run git and return the raw output regardless of exit status.
    pub fn output<I, S>(&self, args: I) -> Result<(Output, String), GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let (mut cmd, rendered) = self.command(args);
        tracing::debug!(dir = %self.dir.display(), args = %rendered, "git");
        let output = cmd.output().map_err(|source| GitError::Spawn {
            program: "git".into(),
            source,
        })?;
        Ok((output, rendered))
    }

    /// Run git, requiring success, and return stdout with trailing newline trimmed.
    pub fn run<I, S>(&self, args: I) -> Result<String, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let (output, rendered) = self.output(args)?;
        if !output.status.success() {
            return Err(self.failed(rendered, &output));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\n', '\r'])
            .to_owned())
    }

    /// Run a predicate-style git command: exit 0 → true, exit 1 → false, anything else → error.
    pub fn check<I, S>(&self, args: I) -> Result<bool, GitError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let (output, rendered) = self.output(args)?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(self.failed(rendered, &output)),
        }
    }

    pub fn failed(&self, args: String, output: &Output) -> GitError {
        GitError::Failed {
            args,
            dir: self.dir.clone(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        }
    }

    /// Resolve a revision to a commit id, or `None` if it does not exist.
    pub fn rev_parse(&self, rev: &str) -> Result<Option<String>, GitError> {
        let spec = format!("{rev}^{{commit}}");
        let (output, _) = self.output(["rev-parse", "--verify", "--quiet", &spec])?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()))
    }

    /// Whether a fully-qualified ref (e.g. `refs/heads/x`) exists.
    pub fn ref_exists(&self, full_ref: &str) -> Result<bool, GitError> {
        self.check(["show-ref", "--verify", "--quiet", full_ref])
    }

    pub fn local_branch_exists(&self, branch: &BranchName) -> Result<bool, GitError> {
        self.ref_exists(&branch.full_ref())
    }

    pub fn remote_branch_exists(
        &self,
        remote: &str,
        branch: &BranchName,
    ) -> Result<bool, GitError> {
        self.ref_exists(&format!("refs/remotes/{remote}/{branch}"))
    }

    /// Whether `ancestor` is reachable from `descendant`.
    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, GitError> {
        self.check(["merge-base", "--is-ancestor", ancestor, descendant])
    }

    /// Commits in `ours` not in `theirs` (ahead) and vice versa (behind).
    pub fn divergence(&self, ours: &str, theirs: &str) -> Result<Divergence, GitError> {
        let range = format!("{ours}...{theirs}");
        let args = ["rev-list", "--left-right", "--count", range.as_str()];
        let out = self.run(args)?;
        let parse_err = || GitError::Parse {
            args: args.join(" "),
            detail: out.clone(),
        };
        let mut parts = out.split_whitespace().map(str::parse::<u64>);
        match (parts.next(), parts.next(), parts.next()) {
            (Some(Ok(ahead)), Some(Ok(behind)), None) => Ok(Divergence { ahead, behind }),
            _ => Err(parse_err()),
        }
    }

    /// Branch checked out in this directory, or `None` when detached.
    pub fn current_branch(&self) -> Result<Option<BranchName>, GitError> {
        let (output, _) = self.output(["symbolic-ref", "--quiet", "HEAD"])?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(BranchName::from_full_ref(
            String::from_utf8_lossy(&output.stdout).trim(),
        ))
    }

    /// Upstream of `branch` as a short name like `origin/feat/x`, if configured and present.
    pub fn upstream_of(&self, branch: &BranchName) -> Result<Option<String>, GitError> {
        let spec = format!("{branch}@{{upstream}}");
        let (output, _) =
            self.output(["rev-parse", "--abbrev-ref", "--symbolic-full-name", &spec])?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|s| !s.is_empty()))
    }

    /// `git config --get <key>`, or `None` if unset.
    pub fn config_get(&self, key: &str) -> Result<Option<String>, GitError> {
        let (output, rendered) = self.output(["config", "--get", key])?;
        match output.status.code() {
            Some(0) => Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            )),
            Some(1) => Ok(None),
            _ => Err(self.failed(rendered, &output)),
        }
    }

    pub fn status(&self) -> Result<WorktreeStatus, GitError> {
        let args = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
        let (output, rendered) = self.output(args)?;
        if !output.status.success() {
            return Err(self.failed(rendered, &output));
        }
        WorktreeStatus::parse(&output.stdout).map_err(|detail| GitError::Parse {
            args: rendered,
            detail,
        })
    }

    pub fn worktrees(&self) -> Result<Vec<Worktree>, GitError> {
        let args = ["worktree", "list", "--porcelain", "-z"];
        let (output, rendered) = self.output(args)?;
        if !output.status.success() {
            return Err(self.failed(rendered, &output));
        }
        worktree::parse_porcelain(&output.stdout).map_err(|detail| GitError::Parse {
            args: rendered,
            detail,
        })
    }
}
