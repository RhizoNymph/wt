//! Removing a worktree, its branch, and the empty directories it leaves behind.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::Path;

use super::candidates::Target;
use super::error::CleanError;
use crate::git::Git;

/// Whether `git worktree remove` may discard local changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Force {
    /// git refuses if the worktree has modified or untracked files.
    No,
    Yes,
}

/// Remove `target`'s worktree (via `git`, run in the primary checkout), delete its
/// branch with `-D` (squash merges make `-d` refuse), and prune empty parents up to
/// but excluding `root`.
pub fn remove(git: &Git, target: &Target, force: Force, root: &Path) -> Result<(), CleanError> {
    let mut args: Vec<&OsStr> = vec![OsStr::new("worktree"), OsStr::new("remove")];
    if force == Force::Yes {
        args.push(OsStr::new("--force"));
    }
    args.push(target.path.as_os_str());
    git.run(args)?;
    tracing::info!(path = %target.path.display(), branch = %target.branch, ?force, "removed worktree");
    git.run(["branch", "-D", target.branch.as_str()])
        .map_err(|source| CleanError::BranchDelete {
            branch: target.branch.to_string(),
            source,
        })?;
    tracing::info!(branch = %target.branch, "deleted branch");
    remove_empty_parents(&target.path, root);
    Ok(())
}

/// Remove now-empty directories between `path` and `root` (exclusive). Stops at the
/// first directory that is non-empty or cannot be removed; never touches `root`.
pub fn remove_empty_parents(path: &Path, root: &Path) {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        match std::fs::remove_dir(dir) {
            Ok(()) => tracing::debug!(dir = %dir.display(), "removed empty directory"),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                tracing::debug!(dir = %dir.display(), error = %e, "stopping parent cleanup");
                break;
            }
        }
        current = dir.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_empty_parents_but_not_root() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("root");
        let leaf = root.join("a/b/c");
        std::fs::create_dir_all(&leaf).expect("mkdir");
        std::fs::write(root.join("a/keep"), "k").expect("write");
        std::fs::remove_dir(&leaf).expect("rm leaf");
        remove_empty_parents(&leaf, &root);
        assert!(!root.join("a/b").exists());
        assert!(root.join("a").exists(), "non-empty dir must stay");

        std::fs::remove_file(root.join("a/keep")).expect("rm");
        remove_empty_parents(&root.join("a/x"), &root);
        assert!(!root.join("a").exists());
        assert!(root.exists(), "root must stay");
    }

    #[test]
    fn ignores_paths_outside_root() {
        let tmp = tempfile::tempdir().expect("tmp");
        let outside = tmp.path().join("elsewhere/x");
        std::fs::create_dir_all(&outside).expect("mkdir");
        remove_empty_parents(&outside.join("y"), &tmp.path().join("root"));
        assert!(outside.exists());
    }
}
