//! The `wt clean` report printed to stdout.

use std::fmt;
use std::path::PathBuf;

use super::candidates::Skipped;
use super::error::CleanError;
use super::merged::MergeEvidence;
use super::scratch::NotCopied;
use crate::branch::BranchName;
use crate::git::{ChangeKind, WorktreeStatus};

/// Modified (any tracked change) and untracked file counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirtyCounts {
    pub modified: usize,
    pub untracked: usize,
}

impl DirtyCounts {
    pub fn of(status: &WorktreeStatus) -> Self {
        let untracked = status
            .entries
            .iter()
            .filter(|e| e.kind == ChangeKind::Untracked)
            .count();
        Self {
            modified: status.entries.len() - untracked,
            untracked,
        }
    }
}

impl fmt::Display for DirtyCounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} modified, {} untracked",
            self.modified, self.untracked
        )
    }
}

/// How merge evidence was gathered this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectionMode {
    GitHub { merged_prs: usize },
    GitOnly { reason: String },
}

/// Result of the up-front `git fetch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Fetched { remote: String },
    Failed { remote: String, error: String },
}

#[derive(Debug)]
pub struct Removed {
    pub branch: BranchName,
    pub path: PathBuf,
    pub evidence: MergeEvidence,
    /// Local changes discarded by `--delete-dirty`; `None` when the worktree was clean.
    pub discarded: Option<DirtyCounts>,
}

#[derive(Debug)]
pub struct Scratched {
    pub branch: BranchName,
    pub path: PathBuf,
    pub evidence: MergeEvidence,
    /// `None` when nothing existed on disk to copy.
    pub dest: Option<PathBuf>,
    pub copied: usize,
    pub not_copied: Vec<NotCopied>,
}

#[derive(Debug)]
pub struct SkippedDirty {
    pub branch: BranchName,
    pub path: PathBuf,
    pub evidence: MergeEvidence,
    pub counts: DirtyCounts,
}

#[derive(Debug)]
pub struct Failure {
    pub branch: Option<BranchName>,
    pub path: PathBuf,
    pub error: CleanError,
}

/// What happened to one clean candidate.
#[derive(Debug)]
pub enum Outcome {
    Removed(Removed),
    Scratched(Scratched),
    SkippedDirty(SkippedDirty),
    NotMerged,
}

#[derive(Debug)]
pub struct CleanReport {
    pub dry_run: bool,
    pub detection: DetectionMode,
    pub fetch: FetchOutcome,
    pub removed: Vec<Removed>,
    pub scratched: Vec<Scratched>,
    pub skipped_dirty: Vec<SkippedDirty>,
    pub skipped: Vec<Skipped>,
    pub not_merged: usize,
    pub pruned: Vec<PathBuf>,
    pub errors: Vec<Failure>,
}

impl CleanReport {
    pub fn new(dry_run: bool, detection: DetectionMode, fetch: FetchOutcome) -> Self {
        Self {
            dry_run,
            detection,
            fetch,
            removed: Vec::new(),
            scratched: Vec::new(),
            skipped_dirty: Vec::new(),
            skipped: Vec::new(),
            not_merged: 0,
            pruned: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn record(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Removed(r) => self.removed.push(r),
            Outcome::Scratched(s) => self.scratched.push(s),
            Outcome::SkippedDirty(s) => self.skipped_dirty.push(s),
            Outcome::NotMerged => self.not_merged += 1,
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Worktree directories this run deleted (empty for a dry run).
    pub fn removed_paths(&self) -> impl Iterator<Item = &PathBuf> {
        let active = !self.dry_run;
        self.removed
            .iter()
            .map(|r| &r.path)
            .chain(self.scratched.iter().map(|s| &s.path))
            .filter(move |_| active)
    }

    fn title<'a>(&self, done: &'a str, planned: &'a str) -> &'a str {
        if self.dry_run { planned } else { done }
    }
}

fn not_copied_label(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Deleted => "deleted",
        ChangeKind::Conflicted => "conflicted, missing",
        ChangeKind::Untracked | ChangeKind::Changed { .. } => "missing",
    }
}

impl fmt::Display for CleanReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dry_run {
            writeln!(f, "Dry run: no changes were made.")?;
        }
        match &self.detection {
            DetectionMode::GitHub { merged_prs } => writeln!(
                f,
                "Merge detection: GitHub pull requests ({merged_prs} merged), then git ancestry"
            )?,
            DetectionMode::GitOnly { reason } => {
                writeln!(f, "Merge detection: git ancestry only ({reason})")?
            }
        }
        if let FetchOutcome::Failed { remote, error } = &self.fetch {
            writeln!(f, "Fetch from {remote} failed, using local refs: {error}")?;
        }

        if !self.removed.is_empty() {
            writeln!(f, "\n{}:", self.title("Removed", "Would remove"))?;
            for r in &self.removed {
                write!(f, "  {}  {}  [{}]", r.branch, r.path.display(), r.evidence)?;
                if let Some(counts) = r.discarded {
                    write!(f, "  (discarded {counts})")?;
                }
                writeln!(f)?;
            }
        }

        if !self.scratched.is_empty() {
            writeln!(
                f,
                "\n{}:",
                self.title("Saved to scratch", "Would save to scratch")
            )?;
            for s in &self.scratched {
                let dest = s
                    .dest
                    .as_ref()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|| "(nothing to copy)".into());
                writeln!(
                    f,
                    "  {}  {} -> {}  ({} files)  [{}]",
                    s.branch,
                    s.path.display(),
                    dest,
                    s.copied,
                    s.evidence
                )?;
                for nc in &s.not_copied {
                    writeln!(
                        f,
                        "    not copied ({}): {}",
                        not_copied_label(&nc.kind),
                        nc.path.display()
                    )?;
                }
            }
        }

        if !self.skipped_dirty.is_empty() {
            writeln!(f, "\nSkipped (dirty):")?;
            for s in &self.skipped_dirty {
                writeln!(
                    f,
                    "  {}  {}  ({})  [{}]",
                    s.branch,
                    s.path.display(),
                    s.counts,
                    s.evidence
                )?;
            }
            writeln!(
                f,
                "  rerun with --scratch to save their changes, or --delete-dirty to discard them"
            )?;
        }

        if !self.skipped.is_empty() || self.not_merged > 0 {
            writeln!(f, "\nSkipped:")?;
            for s in &self.skipped {
                match &s.branch {
                    Some(b) => writeln!(f, "  {}  {}  ({})", b, s.path.display(), s.reason)?,
                    None => writeln!(f, "  {}  ({})", s.path.display(), s.reason)?,
                }
            }
            if self.not_merged > 0 {
                writeln!(f, "  {} not merged", self.not_merged)?;
            }
        }

        if !self.pruned.is_empty() {
            writeln!(
                f,
                "\n{}:",
                self.title(
                    "Pruned stale worktree entries",
                    "Would prune stale worktree entries"
                )
            )?;
            for p in &self.pruned {
                writeln!(f, "  {}", p.display())?;
            }
        }

        if !self.errors.is_empty() {
            writeln!(f, "\nErrors:")?;
            for e in &self.errors {
                let branch = e.branch.as_ref().map(|b| b.as_str()).unwrap_or("-");
                writeln!(f, "  {}  {}: {}", branch, e.path.display(), e.error)?;
            }
        }

        let nothing = self.removed.is_empty()
            && self.scratched.is_empty()
            && self.skipped_dirty.is_empty()
            && self.pruned.is_empty()
            && self.errors.is_empty();
        if nothing {
            writeln!(f, "\nNothing to clean.")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::StatusEntry;

    fn branch(name: &str) -> BranchName {
        BranchName::parse(name).expect("branch")
    }

    #[test]
    fn counts_modified_and_untracked() {
        let status = WorktreeStatus {
            entries: vec![
                StatusEntry {
                    path: "a".into(),
                    kind: ChangeKind::Untracked,
                },
                StatusEntry {
                    path: "b".into(),
                    kind: ChangeKind::Deleted,
                },
                StatusEntry {
                    path: "c".into(),
                    kind: ChangeKind::Changed { renamed_from: None },
                },
            ],
        };
        assert_eq!(
            DirtyCounts::of(&status),
            DirtyCounts {
                modified: 2,
                untracked: 1
            }
        );
    }

    #[test]
    fn renders_sections() {
        let mut report = CleanReport::new(
            false,
            DetectionMode::GitHub { merged_prs: 3 },
            FetchOutcome::Fetched {
                remote: "origin".into(),
            },
        );
        report.record(Outcome::Removed(Removed {
            branch: branch("feat/x"),
            path: "/r/feat/x".into(),
            evidence: MergeEvidence::PullRequest {
                number: 12,
                url: "u".into(),
            },
            discarded: None,
        }));
        report.record(Outcome::SkippedDirty(SkippedDirty {
            branch: branch("feat/y"),
            path: "/r/feat/y".into(),
            evidence: MergeEvidence::AncestorOfBase {
                base_ref: "origin/main".into(),
            },
            counts: DirtyCounts {
                modified: 1,
                untracked: 2,
            },
        }));
        report.record(Outcome::NotMerged);
        let text = report.to_string();
        assert!(
            text.contains("Removed:\n  feat/x  /r/feat/x  [PR #12]"),
            "{text}"
        );
        assert!(text.contains("(1 modified, 2 untracked)"), "{text}");
        assert!(text.contains("1 not merged"), "{text}");
        assert!(!text.contains("Nothing to clean"), "{text}");
        assert_eq!(report.removed_paths().count(), 1);
    }

    #[test]
    fn dry_run_titles_and_no_removed_paths() {
        let mut report = CleanReport::new(
            true,
            DetectionMode::GitOnly {
                reason: "gh missing".into(),
            },
            FetchOutcome::Failed {
                remote: "origin".into(),
                error: "offline".into(),
            },
        );
        report.record(Outcome::Removed(Removed {
            branch: branch("feat/x"),
            path: "/r/feat/x".into(),
            evidence: MergeEvidence::AncestorOfBase {
                base_ref: "main".into(),
            },
            discarded: None,
        }));
        let text = report.to_string();
        assert!(text.starts_with("Dry run"), "{text}");
        assert!(text.contains("Would remove:"), "{text}");
        assert!(text.contains("git ancestry only (gh missing)"), "{text}");
        assert!(text.contains("using local refs: offline"), "{text}");
        assert_eq!(report.removed_paths().count(), 0);
    }
}
