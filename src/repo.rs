//! Repository context: primary checkout, worktree root, and managed worktrees.

use std::path::{Component, Path, PathBuf};

use crate::branch::BranchName;
use crate::config::Config;
use crate::error::RepoError;
use crate::git::{Git, HeadKind, Worktree};

/// Directory under the worktree root reserved for `clean --scratch` output.
pub const SCRATCH_DIR: &str = "scratch";

#[derive(Debug, Clone)]
pub struct Repo {
    primary: PathBuf,
    root: PathBuf,
    config: Config,
    git: Git,
}

impl Repo {
    /// Locate the repository containing `cwd` and load its configuration.
    pub fn discover(cwd: &Path) -> Result<Self, RepoError> {
        let probe = Git::new(cwd);
        let (output, _) = probe.output(["rev-parse", "--git-dir"])?;
        if !output.status.success() {
            return Err(RepoError::NotARepo(cwd.to_owned()));
        }
        // git always lists the main worktree first.
        let first = probe
            .worktrees()?
            .into_iter()
            .next()
            .ok_or(RepoError::NoPrimary)?;
        let primary = canonical(&first.path)?;
        let config = Config::load(&primary)?;
        let root = normalize(&primary.join(&config.worktree_dir));
        Ok(Self {
            git: Git::new(&primary),
            primary,
            root,
            config,
        })
    }

    /// The main worktree (where the repository's `.git` lives).
    pub fn primary(&self) -> &Path {
        &self.primary
    }

    /// Directory under which branch worktrees are created.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn remote(&self) -> &str {
        &self.config.remote
    }

    /// Git runner in the primary checkout.
    pub fn git(&self) -> &Git {
        &self.git
    }

    pub fn scratch_dir(&self) -> PathBuf {
        self.root.join(SCRATCH_DIR)
    }

    /// Where `branch`'s worktree lives: `<root>/<branch with / as dirs>`.
    pub fn path_for(&self, branch: &BranchName) -> Result<PathBuf, RepoError> {
        let path = self.root.join(branch.rel_path());
        if branch.first_component() == SCRATCH_DIR {
            return Err(RepoError::ReservedPath {
                branch: branch.to_string(),
                path,
            });
        }
        Ok(path)
    }

    /// Every worktree git knows about, including the primary.
    pub fn worktrees(&self) -> Result<Vec<Worktree>, RepoError> {
        Ok(self.git.worktrees()?)
    }

    /// Non-bare worktrees located under the worktree root (outside the scratch dir),
    /// with paths canonicalized where they exist on disk.
    pub fn managed_worktrees(&self) -> Result<Vec<Worktree>, RepoError> {
        let scratch = self.scratch_dir();
        Ok(self
            .worktrees()?
            .into_iter()
            .filter(|wt| wt.kind != HeadKind::Bare)
            .map(|mut wt| {
                if let Ok(p) = wt.path.canonicalize() {
                    wt.path = p;
                }
                wt
            })
            .filter(|wt| wt.path.starts_with(&self.root) && !wt.path.starts_with(&scratch))
            .collect())
    }

    pub fn is_primary(&self, wt: &Worktree) -> bool {
        wt.path == self.primary
    }

    /// Base branch: configured, else `<remote>/HEAD`, else local `main`/`master`.
    pub fn base_branch(&self) -> Result<BranchName, RepoError> {
        if let Some(b) = &self.config.base_branch {
            return Ok(b.clone());
        }
        let remote = self.remote();
        let head_ref = format!("refs/remotes/{remote}/HEAD");
        let (output, _) = self.git.output(["symbolic-ref", "--quiet", &head_ref])?;
        if output.status.success() {
            let full = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if let Some(b) = full
                .strip_prefix(&format!("refs/remotes/{remote}/"))
                .and_then(|name| BranchName::parse(name).ok())
            {
                return Ok(b);
            }
        }
        for candidate in ["main", "master"] {
            let b = BranchName::parse(candidate).map_err(|_| RepoError::NoBaseBranch {
                remote: remote.to_owned(),
            })?;
            if self.git.local_branch_exists(&b)? {
                return Ok(b);
            }
        }
        Err(RepoError::NoBaseBranch {
            remote: remote.to_owned(),
        })
    }
}

fn canonical(path: &Path) -> Result<PathBuf, RepoError> {
    path.canonicalize().map_err(|source| RepoError::Io {
        path: path.to_owned(),
        source,
    })
}

/// Lexically resolve `.` and `..` (the root may not exist yet, so no canonicalize).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_resolves_dots() {
        assert_eq!(normalize(Path::new("/a/b/main/..")), PathBuf::from("/a/b"));
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(
            normalize(Path::new("/a/b/.worktrees")),
            PathBuf::from("/a/b/.worktrees")
        );
    }
}
