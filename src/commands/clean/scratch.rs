//! Saving a dirty worktree's changed files under `<root>/scratch/<branch>/`.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use super::error::CleanError;
use crate::branch::BranchName;
use crate::git::{ChangeKind, WorktreeStatus};

/// Suffixes `-2` .. `-MAX_SUFFIX` are tried when the natural scratch dir exists.
const MAX_SUFFIX: u32 = 1000;

/// A dirty path that will not be copied because nothing exists on disk for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotCopied {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

/// Path component names (e.g. `target`, `node_modules`) never copied from gitignored
/// content. Dirty (non-ignored) files are always copied regardless.
#[derive(Debug, Clone, Copy)]
pub struct Exclude<'a>(pub &'a [String]);

impl Exclude<'_> {
    fn matches_name(&self, name: &OsStr) -> bool {
        self.0.iter().any(|n| OsStr::new(n) == name)
    }

    /// Whether any component of the relative path is an excluded name.
    pub fn matches(&self, rel: &Path) -> bool {
        rel.components()
            .any(|c| matches!(c, Component::Normal(n) if self.matches_name(n)))
    }
}

/// Gitignored paths in `worktree` worth saving: everything except excluded names.
pub fn ignored_to_save(worktree: &Path, exclude: Exclude<'_>) -> Result<Vec<PathBuf>, CleanError> {
    Ok(crate::git::Git::new(worktree)
        .ignored_paths()?
        .into_iter()
        .filter(|p| !exclude.matches(p))
        .collect())
}

/// What a scratch save copies. Paths are relative to the worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchPlan {
    /// Dirty paths from `git status` that exist on disk; copied in full.
    pub files: Vec<PathBuf>,
    /// Gitignored paths (already filtered); directories are copied recursively,
    /// skipping excluded names inside them.
    pub ignored: Vec<PathBuf>,
    pub not_copied: Vec<NotCopied>,
}

impl ScratchPlan {
    /// Every dirty path that exists on disk is copied (modified, added, renamed,
    /// untracked, and conflicted files that still exist) plus the given ignored
    /// paths; dirty paths missing from disk are reported.
    pub fn new(
        worktree: &Path,
        status: &WorktreeStatus,
        ignored: Vec<PathBuf>,
    ) -> Result<Self, CleanError> {
        let mut plan = Self {
            files: Vec::new(),
            ignored,
            not_copied: Vec::new(),
        };
        for entry in &status.entries {
            let abs = worktree.join(&entry.path);
            let exists = abs.try_exists().map_err(|source| CleanError::Inspect {
                path: abs.clone(),
                source,
            })? || abs.symlink_metadata().is_ok();
            if exists {
                plan.files.push(entry.path.clone());
            } else {
                plan.not_copied.push(NotCopied {
                    path: entry.path.clone(),
                    kind: entry.kind.clone(),
                });
            }
        }
        Ok(plan)
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.ignored.is_empty()
    }

    /// Number of top-level paths copied (an ignored directory counts once).
    pub fn copied(&self) -> usize {
        self.files.len() + self.ignored.len()
    }
}

/// Natural scratch location for `branch`: `<scratch_root>/<branch components>`.
pub fn base_dest(scratch_root: &Path, branch: &BranchName) -> PathBuf {
    scratch_root.join(branch.rel_path())
}

/// Candidate destinations in preference order: `base`, then `base-2`, `base-3`, ...
fn candidates(base: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    let name = base
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::iter::once(base.to_path_buf())
        .chain((2..=MAX_SUFFIX).map(move |n| base.with_file_name(format!("{name}-{n}"))))
}

fn exhausted(base: &Path) -> CleanError {
    CleanError::ScratchExhausted {
        base: base.to_path_buf(),
        attempts: MAX_SUFFIX,
    }
}

/// First candidate that does not exist yet (dry run: nothing is created).
pub fn preview_dest(base: &Path) -> Result<PathBuf, CleanError> {
    candidates(base)
        .find(|p| p.symlink_metadata().is_err())
        .ok_or_else(|| exhausted(base))
}

/// Atomically claim a fresh destination directory: existing scratch dirs are never
/// written into.
fn claim_dest(base: &Path) -> Result<PathBuf, CleanError> {
    if let Some(parent) = base.parent() {
        std::fs::create_dir_all(parent).map_err(|source| CleanError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    for candidate in candidates(base) {
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(CleanError::CreateDir {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Err(exhausted(base))
}

/// Copy the plan's files from `worktree` into a newly claimed directory derived from
/// `base`, preserving relative paths. Returns the directory used.
pub fn save(
    plan: &ScratchPlan,
    worktree: &Path,
    base: &Path,
    exclude: Exclude<'_>,
) -> Result<PathBuf, CleanError> {
    let dest = claim_dest(base)?;
    for rel in &plan.files {
        copy_path(&worktree.join(rel), &dest.join(rel), None)?;
    }
    for rel in &plan.ignored {
        copy_path(&worktree.join(rel), &dest.join(rel), Some(exclude))?;
    }
    tracing::info!(dest = %dest.display(), files = plan.files.len(), ignored = plan.ignored.len(), "saved files to scratch");
    Ok(dest)
}

/// Copy a file, symlink or directory tree; `exclude` skips matching names while recursing.
fn copy_path(from: &Path, to: &Path, exclude: Option<Exclude<'_>>) -> Result<(), CleanError> {
    let copy_err = |source| CleanError::Copy {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    };
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|source| CleanError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let meta = from.symlink_metadata().map_err(copy_err)?;
    if meta.file_type().is_symlink() {
        return copy_symlink(from, to).map_err(copy_err);
    }
    if meta.is_dir() {
        // Untracked directories git reports whole (e.g. nested repositories).
        std::fs::create_dir_all(to).map_err(copy_err)?;
        let entries = std::fs::read_dir(from).map_err(copy_err)?;
        for entry in entries {
            let entry = entry.map_err(copy_err)?;
            let name = entry.file_name();
            if exclude.is_some_and(|ex| ex.matches_name(&name)) {
                continue;
            }
            copy_path(&entry.path(), &to.join(&name), exclude)?;
        }
        return Ok(());
    }
    std::fs::copy(from, to).map(|_| ()).map_err(copy_err)
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
}

#[cfg(not(unix))]
fn copy_symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::copy(from, to).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::StatusEntry;

    #[test]
    fn exclude_matches_any_component() {
        let names = vec!["target".to_owned(), "node_modules".to_owned()];
        let ex = Exclude(&names);
        assert!(ex.matches(Path::new("target")));
        assert!(ex.matches(Path::new("web/node_modules/x.js")));
        assert!(!ex.matches(Path::new("targets/x")));
        assert!(!ex.matches(Path::new(".env")));
    }

    #[test]
    fn save_skips_excluded_names_inside_ignored_dirs() {
        let tmp = tempfile::tempdir().expect("tmp");
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(wt.join("local/node_modules")).expect("mkdir");
        std::fs::write(wt.join("local/keep.txt"), "k").expect("write");
        std::fs::write(wt.join("local/node_modules/x.js"), "x").expect("write");
        let plan = ScratchPlan {
            files: vec![],
            ignored: vec![PathBuf::from("local")],
            not_copied: vec![],
        };
        let names = vec!["node_modules".to_owned()];
        let dest = save(&plan, &wt, &tmp.path().join("s"), Exclude(&names)).expect("save");
        assert!(dest.join("local/keep.txt").exists());
        assert!(!dest.join("local/node_modules").exists());
    }

    #[test]
    fn candidates_add_numeric_suffixes() {
        let base = Path::new("/r/scratch/feat/x");
        let first: Vec<_> = candidates(base).take(3).collect();
        assert_eq!(
            first,
            vec![
                PathBuf::from("/r/scratch/feat/x"),
                PathBuf::from("/r/scratch/feat/x-2"),
                PathBuf::from("/r/scratch/feat/x-3"),
            ]
        );
    }

    #[test]
    fn plan_splits_existing_and_missing() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("a"), "a").expect("write");
        let status = WorktreeStatus {
            entries: vec![
                StatusEntry {
                    path: "a".into(),
                    kind: ChangeKind::Changed { renamed_from: None },
                },
                StatusEntry {
                    path: "gone".into(),
                    kind: ChangeKind::Deleted,
                },
            ],
        };
        let plan = ScratchPlan::new(tmp.path(), &status, vec![]).expect("plan");
        assert_eq!(plan.files, vec![PathBuf::from("a")]);
        assert_eq!(
            plan.not_copied,
            vec![NotCopied {
                path: "gone".into(),
                kind: ChangeKind::Deleted
            }]
        );
    }

    #[test]
    fn save_claims_fresh_dir_and_copies_tree() {
        let tmp = tempfile::tempdir().expect("tmp");
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(wt.join("d/e")).expect("mkdir");
        std::fs::write(wt.join("d/e/f.txt"), "f").expect("write");
        let base = tmp.path().join("scratch/b");
        std::fs::create_dir_all(&base).expect("pre-existing");
        let plan = ScratchPlan {
            files: vec![PathBuf::from("d/e/f.txt")],
            ignored: vec![],
            not_copied: vec![],
        };
        assert_eq!(
            preview_dest(&base).expect("preview"),
            tmp.path().join("scratch/b-2")
        );
        let dest = save(&plan, &wt, &base, Exclude(&[])).expect("save");
        assert_eq!(dest, tmp.path().join("scratch/b-2"));
        assert_eq!(
            std::fs::read_to_string(dest.join("d/e/f.txt")).expect("read"),
            "f"
        );
    }
}
