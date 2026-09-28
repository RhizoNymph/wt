//! Parsing of `git status --porcelain=v1 -z`.

use std::path::PathBuf;

/// How a path differs from HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// Not tracked by git.
    Untracked,
    /// Tracked and removed from the working tree (nothing on disk to copy).
    Deleted,
    /// Unmerged path from an in-progress merge/rebase.
    Conflicted,
    /// Added, modified, renamed, copied or type-changed; the file exists on disk.
    Changed { renamed_from: Option<PathBuf> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// Path relative to the worktree root.
    pub path: PathBuf,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeStatus {
    pub entries: Vec<StatusEntry>,
}

const CONFLICT_CODES: &[&str] = &["DD", "AU", "UD", "UA", "DU", "AA", "UU"];

impl WorktreeStatus {
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn has_untracked(&self) -> bool {
        self.entries.iter().any(|e| e.kind == ChangeKind::Untracked)
    }

    pub fn has_tracked_changes(&self) -> bool {
        self.entries.iter().any(|e| e.kind != ChangeKind::Untracked)
    }

    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        let text = String::from_utf8_lossy(raw);
        let mut fields = text.split('\0').filter(|f| !f.is_empty());
        let mut entries = Vec::new();
        while let Some(field) = fields.next() {
            let (code, path) = match (field.get(..2), field.get(3..)) {
                (Some(code), Some(path)) if field.as_bytes().get(2) == Some(&b' ') => (code, path),
                _ => return Err(format!("malformed status entry {field:?}")),
            };
            let kind = if code == "??" {
                ChangeKind::Untracked
            } else if CONFLICT_CODES.contains(&code) {
                ChangeKind::Conflicted
            } else if code.contains('R') || code.contains('C') {
                let from = fields
                    .next()
                    .ok_or_else(|| format!("rename entry {field:?} missing source path"))?;
                ChangeKind::Changed {
                    renamed_from: Some(PathBuf::from(from)),
                }
            } else if code.contains('D') {
                ChangeKind::Deleted
            } else {
                ChangeKind::Changed { renamed_from: None }
            };
            entries.push(StatusEntry {
                path: PathBuf::from(path),
                kind,
            });
        }
        Ok(Self { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_clean() {
        let s = WorktreeStatus::parse(b"").expect("parse");
        assert!(s.is_clean());
        assert!(!s.has_untracked());
        assert!(!s.has_tracked_changes());
    }

    #[test]
    fn classifies_entries() {
        let raw = b" M src/a.rs\0?? new dir/file.txt\0 D gone.rs\0R  to.rs\0from.rs\0UU both.rs\0A  added.rs\0";
        let s = WorktreeStatus::parse(raw).expect("parse");
        let kinds: Vec<_> = s
            .entries
            .iter()
            .map(|e| (e.path.to_string_lossy().into_owned(), e.kind.clone()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (
                    "src/a.rs".into(),
                    ChangeKind::Changed { renamed_from: None }
                ),
                ("new dir/file.txt".into(), ChangeKind::Untracked),
                ("gone.rs".into(), ChangeKind::Deleted),
                (
                    "to.rs".into(),
                    ChangeKind::Changed {
                        renamed_from: Some("from.rs".into())
                    }
                ),
                ("both.rs".into(), ChangeKind::Conflicted),
                (
                    "added.rs".into(),
                    ChangeKind::Changed { renamed_from: None }
                ),
            ]
        );
        assert!(s.has_untracked());
        assert!(s.has_tracked_changes());
    }

    #[test]
    fn untracked_only() {
        let s = WorktreeStatus::parse(b"?? x\0").expect("parse");
        assert!(s.has_untracked());
        assert!(!s.has_tracked_changes());
    }

    #[test]
    fn rejects_garbage() {
        assert!(WorktreeStatus::parse(b"x\0").is_err());
        assert!(WorktreeStatus::parse(b"R  to\0").is_err());
    }
}
