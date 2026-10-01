//! `wt checkout`: switch to the worktree for a branch, creating it if needed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;

use crate::branch::BranchName;
use crate::cli::CheckoutArgs;
use crate::error::{GitError, RepoError};
use crate::git::{Git, HeadKind, Worktree};
use crate::progress::Progress;
use crate::repo::Repo;
use crate::shell;

/// Where a newly created worktree's branch comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The local branch already exists; it is checked out as-is.
    Local,
    /// Only `<remote>/<branch>` exists; a local branch is created tracking it.
    Remote { upstream: String },
    /// Brand-new branch created (without upstream) at `start`.
    New { start: String },
}

/// Result of a checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A worktree already had the branch checked out.
    Existing { path: PathBuf },
    /// A worktree was created at `path`.
    Created { path: PathBuf, source: Source },
}

impl Outcome {
    pub fn path(&self) -> &Path {
        match self {
            Self::Existing { path } | Self::Created { path, .. } => path,
        }
    }
}

/// What occupies a branch's target path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Occupant {
    /// A worktree with a different branch (or detached HEAD) lives there.
    Worktree { head: String },
    /// A non-empty directory or a file that is not a worktree.
    Files,
}

impl std::fmt::Display for Occupant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Worktree { head } => write!(f, "a worktree on {head}"),
            Self::Files => f.write_str("existing files"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CheckoutError {
    #[error("cannot create worktree for {branch} at {path}: path is occupied by {occupant}")]
    PathOccupied {
        branch: BranchName,
        path: PathBuf,
        occupant: Occupant,
    },
    #[error("start point {rev:?} does not resolve to a commit")]
    UnknownStartPoint { rev: String },
    #[error("failed to access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Result of trying to refresh a single remote branch.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FetchOutcome {
    Fetched,
    /// The remote is reachable but has no such branch.
    Missing,
    /// The fetch failed for another reason (offline, no such remote, auth).
    Unreachable {
        detail: String,
    },
}

pub fn run(repo: &Repo, args: &CheckoutArgs, progress: &Progress) -> anyhow::Result<ExitCode> {
    let outcome = checkout(repo, &args.branch, args.from.as_deref(), progress);
    progress.finish();
    let outcome = outcome?;
    let path = outcome.path();
    match &outcome {
        Outcome::Existing { .. } => {
            eprintln!(
                "Worktree for {} already exists at {}",
                args.branch,
                path.display()
            );
        }
        Outcome::Created { source, .. } => {
            let how = match source {
                Source::Local => "existing local branch".to_owned(),
                Source::Remote { upstream } => format!("tracking {upstream}"),
                Source::New { start } => format!("new branch from {start}"),
            };
            eprintln!(
                "Created worktree for {} at {} ({how})",
                args.branch,
                path.display()
            );
        }
    }
    shell::request_cd(path).with_context(|| format!("requesting cd to {}", path.display()))?;
    Ok(ExitCode::SUCCESS)
}

/// Find or create the worktree for `branch`.
pub fn checkout(
    repo: &Repo,
    branch: &BranchName,
    from: Option<&str>,
    progress: &Progress,
) -> Result<Outcome, CheckoutError> {
    let path = repo.path_for(branch)?;
    let mut worktrees = repo.worktrees()?;

    if let Some(wt) = find_branch(&worktrees, branch) {
        if wt.path.exists() {
            tracing::debug!(branch = %branch, path = %wt.path.display(), "worktree exists");
            return Ok(Outcome::Existing {
                path: wt.path.clone(),
            });
        }
        tracing::info!(branch = %branch, path = %wt.path.display(), "pruning stale worktree");
        repo.git().run(["worktree", "prune"])?;
        worktrees = repo.worktrees()?;
    }

    if let Some(occupant) = occupant(&path, &worktrees)? {
        return Err(CheckoutError::PathOccupied {
            branch: branch.clone(),
            path,
            occupant,
        });
    }

    let source = resolve_source(repo, branch, from, progress)?;
    progress.phase(format!("creating worktree at {}", path.display()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| CheckoutError::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    add_worktree(repo.git(), branch, &path, &source)?;
    tracing::info!(branch = %branch, path = %path.display(), source = ?source, "created worktree");
    Ok(Outcome::Created { path, source })
}

fn find_branch<'a>(worktrees: &'a [Worktree], branch: &BranchName) -> Option<&'a Worktree> {
    worktrees.iter().find(|wt| wt.branch() == Some(branch))
}

/// What (if anything) prevents creating a worktree at `path`. Empty directories are fine.
fn occupant(path: &Path, worktrees: &[Worktree]) -> Result<Option<Occupant>, CheckoutError> {
    let io_err = |source| CheckoutError::Io {
        path: path.to_owned(),
        source,
    };
    let registered = worktrees
        .iter()
        .find(|wt| wt.kind != HeadKind::Bare && same_path(&wt.path, path));
    if let Some(wt) = registered {
        let head = match &wt.kind {
            HeadKind::Branch(b) => b.to_string(),
            HeadKind::Detached | HeadKind::Bare => "a detached HEAD".to_owned(),
        };
        return Ok(Some(Occupant::Worktree { head }));
    }
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_err(err)),
    };
    if !meta.is_dir() {
        return Ok(Some(Occupant::Files));
    }
    let mut entries = std::fs::read_dir(path).map_err(io_err)?;
    Ok(entries.next().is_some().then_some(Occupant::Files))
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
}

/// Decide where the branch comes from, fetching from the remote when it may help.
fn resolve_source(
    repo: &Repo,
    branch: &BranchName,
    from: Option<&str>,
    progress: &Progress,
) -> Result<Source, CheckoutError> {
    let git = repo.git();
    let remote = repo.remote();

    if git.local_branch_exists(branch)? {
        warn_ignored_from(from, progress);
        return Ok(Source::Local);
    }

    let fetched = fetch_branch(git, remote, branch, progress)?;
    if git.remote_branch_exists(remote, branch)? {
        warn_ignored_from(from, progress);
        return Ok(Source::Remote {
            upstream: format!("{remote}/{branch}"),
        });
    }

    if let Some(rev) = from {
        if git.rev_parse(rev)?.is_none() {
            return Err(CheckoutError::UnknownStartPoint {
                rev: rev.to_owned(),
            });
        }
        return Ok(Source::New {
            start: rev.to_owned(),
        });
    }

    let base = repo.base_branch()?;
    // Only worth another round-trip if the remote answered the first fetch.
    if !matches!(fetched, FetchOutcome::Unreachable { .. }) {
        fetch_branch(git, remote, &base, progress)?;
    }
    let start = if git.remote_branch_exists(remote, &base)? {
        format!("{remote}/{base}")
    } else {
        base.to_string()
    };
    Ok(Source::New { start })
}

fn warn_ignored_from(from: Option<&str>, progress: &Progress) {
    if let Some(rev) = from {
        progress.println(format!(
            "warning: branch already exists; ignoring --from {rev}"
        ));
    }
}

/// `git fetch <remote> <branch>`, classifying failure. Only spawn errors are fatal.
fn fetch_branch(
    git: &Git,
    remote: &str,
    branch: &BranchName,
    progress: &Progress,
) -> Result<FetchOutcome, CheckoutError> {
    progress.phase(format!("fetching {branch} from {remote}"));
    let (output, _) = git.output(["fetch", "--quiet", "--no-tags", remote, branch.as_str()])?;
    let outcome = if output.status.success() {
        FetchOutcome::Fetched
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("couldn't find remote ref") {
            FetchOutcome::Missing
        } else {
            FetchOutcome::Unreachable {
                detail: stderr.trim().to_owned(),
            }
        }
    };
    match &outcome {
        FetchOutcome::Unreachable { detail } => {
            tracing::debug!(remote, branch = %branch, detail = %detail, "fetch failed");
            progress.println(format!(
                "warning: could not fetch {branch} from {remote}; continuing with local refs"
            ));
        }
        other => tracing::debug!(remote, branch = %branch, outcome = ?other, "fetch"),
    }
    Ok(outcome)
}

/// Arguments for `git worktree add` for the given source.
fn worktree_add_args(branch: &BranchName, path: &Path, source: &Source) -> Vec<String> {
    let path = path.to_string_lossy().into_owned();
    let mut args: Vec<String> = ["worktree", "add", "--quiet"].map(Into::into).into();
    match source {
        Source::Local => args.extend([path, branch.to_string()]),
        Source::Remote { upstream } => args.extend([
            "--track".into(),
            "-b".into(),
            branch.to_string(),
            path,
            upstream.clone(),
        ]),
        Source::New { start } => args.extend([
            "--no-track".into(),
            "-b".into(),
            branch.to_string(),
            path,
            start.clone(),
        ]),
    }
    args
}

fn add_worktree(
    git: &Git,
    branch: &BranchName,
    path: &Path,
    source: &Source,
) -> Result<(), GitError> {
    git.run(worktree_add_args(branch, path, source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(name: &str) -> BranchName {
        BranchName::parse(name).expect("valid branch")
    }

    #[test]
    fn add_args_per_source() {
        let p = Path::new("/r/feat/x");
        assert_eq!(
            worktree_add_args(&b("feat/x"), p, &Source::Local),
            ["worktree", "add", "--quiet", "/r/feat/x", "feat/x"]
        );
        assert_eq!(
            worktree_add_args(
                &b("feat/x"),
                p,
                &Source::Remote {
                    upstream: "origin/feat/x".into()
                }
            ),
            [
                "worktree",
                "add",
                "--quiet",
                "--track",
                "-b",
                "feat/x",
                "/r/feat/x",
                "origin/feat/x"
            ]
        );
        assert_eq!(
            worktree_add_args(
                &b("feat/x"),
                p,
                &Source::New {
                    start: "origin/main".into()
                }
            ),
            [
                "worktree",
                "add",
                "--quiet",
                "--no-track",
                "-b",
                "feat/x",
                "/r/feat/x",
                "origin/main"
            ]
        );
    }

    #[test]
    fn occupant_detects_files_and_empty_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("missing");
        assert_eq!(occupant(&missing, &[]).expect("ok"), None);

        let empty = tmp.path().join("empty");
        std::fs::create_dir(&empty).expect("mkdir");
        assert_eq!(occupant(&empty, &[]).expect("ok"), None);

        std::fs::write(empty.join("f"), "x").expect("write");
        assert_eq!(occupant(&empty, &[]).expect("ok"), Some(Occupant::Files));

        let file = tmp.path().join("file");
        std::fs::write(&file, "x").expect("write");
        assert_eq!(occupant(&file, &[]).expect("ok"), Some(Occupant::Files));
    }

    #[test]
    fn occupant_detects_registered_worktree() {
        let wt = Worktree {
            path: PathBuf::from("/nonexistent/wt/feat/x"),
            head: None,
            kind: HeadKind::Branch(b("other")),
            locked: false,
            prunable: true,
        };
        assert_eq!(
            occupant(Path::new("/nonexistent/wt/feat/x"), &[wt]).expect("ok"),
            Some(Occupant::Worktree {
                head: "other".into()
            })
        );
    }
}
