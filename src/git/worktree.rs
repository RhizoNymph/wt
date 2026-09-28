//! Parsing of `git worktree list --porcelain -z`.

use std::path::PathBuf;

use crate::branch::BranchName;

/// What a worktree has checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadKind {
    Branch(BranchName),
    Detached,
    /// The bare repository entry; it has no working tree.
    Bare,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// Commit at HEAD; `None` for bare entries and unborn branches.
    pub head: Option<String>,
    pub kind: HeadKind,
    pub locked: bool,
    pub prunable: bool,
}

impl Worktree {
    pub fn branch(&self) -> Option<&BranchName> {
        match &self.kind {
            HeadKind::Branch(b) => Some(b),
            HeadKind::Detached | HeadKind::Bare => None,
        }
    }
}

const NULL_OID: &str = "0000000000000000000000000000000000000000";

/// Parse NUL-separated porcelain output. Records end with an empty field.
pub fn parse_porcelain(raw: &[u8]) -> Result<Vec<Worktree>, String> {
    let text = String::from_utf8_lossy(raw);
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in text.split('\0') {
        if field.is_empty() {
            if let Some(wt) = current.take() {
                worktrees.push(wt);
            }
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((field, ""));
        if key == "worktree" {
            if let Some(wt) = current.take() {
                worktrees.push(wt);
            }
            current = Some(Worktree {
                path: PathBuf::from(value),
                head: None,
                kind: HeadKind::Detached,
                locked: false,
                prunable: false,
            });
            continue;
        }
        let wt = current
            .as_mut()
            .ok_or_else(|| format!("field {field:?} before any `worktree` line"))?;
        match key {
            "HEAD" => wt.head = (value != NULL_OID).then(|| value.to_owned()),
            "branch" => {
                let branch = BranchName::from_full_ref(value)
                    .ok_or_else(|| format!("unparseable branch ref {value:?}"))?;
                wt.kind = HeadKind::Branch(branch);
            }
            "detached" => wt.kind = HeadKind::Detached,
            "bare" => wt.kind = HeadKind::Bare,
            "locked" => wt.locked = true,
            "prunable" => wt.prunable = true,
            _ => tracing::debug!(field, "ignoring unknown worktree field"),
        }
    }
    if let Some(wt) = current.take() {
        worktrees.push(wt);
    }
    Ok(worktrees)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_entries() {
        let raw = b"worktree /r/main\0HEAD abc\0branch refs/heads/main\0\0\
worktree /r/feat/x\0HEAD def\0branch refs/heads/feat/x\0locked\0\0\
worktree /r/det\0HEAD 123\0detached\0prunable gitdir file points to non-existent location\0\0";
        let wts = parse_porcelain(raw).expect("parse");
        assert_eq!(wts.len(), 3);
        assert_eq!(wts[0].path, PathBuf::from("/r/main"));
        assert_eq!(wts[0].branch().map(BranchName::as_str), Some("main"));
        assert_eq!(wts[1].branch().map(BranchName::as_str), Some("feat/x"));
        assert!(wts[1].locked);
        assert_eq!(wts[2].kind, HeadKind::Detached);
        assert!(wts[2].prunable);
        assert_eq!(wts[2].head.as_deref(), Some("123"));
    }

    #[test]
    fn parses_bare() {
        let wts = parse_porcelain(b"worktree /r.git\0bare\0\0").expect("parse");
        assert_eq!(wts[0].kind, HeadKind::Bare);
        assert_eq!(wts[0].head, None);
    }

    #[test]
    fn rejects_orphan_field() {
        assert!(parse_porcelain(b"HEAD abc\0\0").is_err());
    }
}
