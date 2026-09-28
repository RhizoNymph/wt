//! Merged pull requests from the GitHub CLI.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use super::error::GhError;
use crate::branch::BranchName;

/// Environment variable naming the GitHub CLI executable (default `gh`).
pub const GH_ENV: &str = "WT_GH";
const DEFAULT_GH: &str = "gh";
/// Most recent merged PRs considered. Older ones fall through to git ancestry.
pub const PR_LIMIT: u32 = 1000;

/// One merged pull request as reported by `gh pr list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedPr {
    pub head_ref_name: String,
    pub number: u64,
    pub url: String,
    /// Commit at the PR head when it merged.
    pub head_ref_oid: String,
}

/// Merged pull requests grouped by head branch name.
#[derive(Debug, Default)]
pub struct PrIndex {
    by_branch: HashMap<String, Vec<MergedPr>>,
    total: usize,
}

impl PrIndex {
    pub fn new(prs: Vec<MergedPr>) -> Self {
        let total = prs.len();
        let mut by_branch: HashMap<String, Vec<MergedPr>> = HashMap::new();
        for pr in prs {
            by_branch
                .entry(pr.head_ref_name.clone())
                .or_default()
                .push(pr);
        }
        Self { by_branch, total }
    }

    pub fn for_branch(&self, branch: &BranchName) -> &[MergedPr] {
        self.by_branch
            .get(branch.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Number of merged pull requests loaded.
    pub fn merged_count(&self) -> usize {
        self.total
    }
}

pub fn parse_prs(raw: &[u8], program: &str) -> Result<Vec<MergedPr>, GhError> {
    serde_json::from_slice(raw).map_err(|source| GhError::Parse {
        program: program.to_owned(),
        source,
    })
}

fn gh_program() -> OsString {
    std::env::var_os(GH_ENV)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| DEFAULT_GH.into())
}

/// Ask the GitHub CLI (run in `dir`) for the repository's merged pull requests.
pub fn merged_prs(dir: &Path) -> Result<PrIndex, GhError> {
    let program = gh_program();
    let name = program.to_string_lossy().into_owned();
    let limit = PR_LIMIT.to_string();
    let output = Command::new(&program)
        .current_dir(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .args(["pr", "list", "--state", "merged", "--limit", &limit])
        .args(["--json", "headRefName,number,url,headRefOid"])
        .output()
        .map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => GhError::NotInstalled {
                program: name.clone(),
            },
            _ => GhError::Spawn {
                program: name.clone(),
                source,
            },
        })?;
    if !output.status.success() {
        return Err(GhError::Failed {
            program: name,
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(PrIndex::new(parse_prs(&output.stdout, &name)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_indexes_by_branch() {
        let raw = br#"[
            {"headRefName":"feat/x","number":1,"url":"u1","headRefOid":"aaa"},
            {"headRefName":"feat/x","number":5,"url":"u5","headRefOid":"bbb"},
            {"headRefName":"fix/y","number":2,"url":"u2","headRefOid":"ccc"}
        ]"#;
        let index = PrIndex::new(parse_prs(raw, "gh").expect("parse"));
        assert_eq!(index.merged_count(), 3);
        let x = BranchName::parse("feat/x").expect("branch");
        let numbers: Vec<_> = index.for_branch(&x).iter().map(|p| p.number).collect();
        assert_eq!(numbers, vec![1, 5]);
        let none = BranchName::parse("feat/none").expect("branch");
        assert!(index.for_branch(&none).is_empty());
    }

    #[test]
    fn rejects_non_json() {
        assert!(matches!(
            parse_prs(b"nope", "gh"),
            Err(GhError::Parse { .. })
        ));
        assert!(matches!(
            parse_prs(br#"[{"number":1}]"#, "gh"),
            Err(GhError::Parse { .. })
        ));
    }
}
