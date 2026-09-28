//! Deciding whether a branch has been merged.
//!
//! Safety rule: a branch only counts as merged when its tip contains no work that is
//! missing from the base branch or from a merged pull request's head.
//!
//! 1. Pull request evidence (GitHub CLI): a merged PR has `headRefName == branch` and
//!    the local tip equals the PR head or is an ancestor of it. Squash/rebase merges
//!    are recognised this way; new local commits after the merge disqualify the branch.
//! 2. Ancestry evidence (git only): the tip is an ancestor of `<remote>/<base>` or of
//!    local `<base>`, AND the tip is not on that ref's first-parent chain. A tip on the
//!    first-parent chain is indistinguishable from a branch that was created from base
//!    and never worked on, so it is conservatively treated as not merged (this also
//!    means fast-forward merges are only recognised via pull request evidence).

use std::fmt;

use super::error::GhError;
use super::github::PrIndex;
use crate::branch::BranchName;
use crate::error::GitError;
use crate::git::Git;

/// Why a branch is considered merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeEvidence {
    PullRequest {
        number: u64,
        url: String,
    },
    /// Reachable from `base_ref` through a merge (not its first-parent chain).
    AncestorOfBase {
        base_ref: String,
    },
}

impl fmt::Display for MergeEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PullRequest { number, .. } => write!(f, "PR #{number}"),
            Self::AncestorOfBase { base_ref } => write!(f, "merged into {base_ref}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    Merged(MergeEvidence),
    NotMerged,
}

/// Where merge evidence comes from for this run.
#[derive(Debug)]
pub enum Detection {
    /// Merged PRs from the GitHub CLI, then git ancestry per branch.
    GitHub(PrIndex),
    /// GitHub CLI unavailable; git ancestry only.
    GitOnly(GhError),
}

/// A base ref that exists locally: full name for git, short name for display.
#[derive(Debug, Clone)]
struct BaseRef {
    full: String,
    short: String,
}

pub struct MergeDetector<'a> {
    git: &'a Git,
    detection: Detection,
    base_refs: Vec<BaseRef>,
}

impl<'a> MergeDetector<'a> {
    /// `git` must run in a checkout of the repository (normally the primary).
    pub fn new(
        git: &'a Git,
        detection: Detection,
        remote: &str,
        base: &BranchName,
    ) -> Result<Self, GitError> {
        let mut base_refs = Vec::new();
        for (full, short) in [
            (
                format!("refs/remotes/{remote}/{base}"),
                format!("{remote}/{base}"),
            ),
            (base.full_ref(), base.to_string()),
        ] {
            if git.ref_exists(&full)? {
                base_refs.push(BaseRef { full, short });
            }
        }
        tracing::debug!(
            base_refs = ?base_refs.iter().map(|b| b.short.as_str()).collect::<Vec<_>>(),
            "merge detection base refs"
        );
        Ok(Self {
            git,
            detection,
            base_refs,
        })
    }

    pub fn detection(&self) -> &Detection {
        &self.detection
    }

    pub fn evaluate(&self, branch: &BranchName, tip: &str) -> Result<MergeStatus, GitError> {
        if let Detection::GitHub(index) = &self.detection
            && let Some(evidence) = self.pull_request_evidence(index, branch, tip)?
        {
            return Ok(MergeStatus::Merged(evidence));
        }
        for base in &self.base_refs {
            if self.merged_into(tip, &base.full)? {
                return Ok(MergeStatus::Merged(MergeEvidence::AncestorOfBase {
                    base_ref: base.short.clone(),
                }));
            }
        }
        Ok(MergeStatus::NotMerged)
    }

    fn pull_request_evidence(
        &self,
        index: &PrIndex,
        branch: &BranchName,
        tip: &str,
    ) -> Result<Option<MergeEvidence>, GitError> {
        for pr in index.for_branch(branch) {
            let covered = pr.head_ref_oid == tip
                || (self.git.rev_parse(&pr.head_ref_oid)?.is_some()
                    && self.git.is_ancestor(tip, &pr.head_ref_oid)?);
            tracing::debug!(branch = %branch, pr = pr.number, pr_head = %pr.head_ref_oid, tip, covered, "merged PR candidate");
            if covered {
                return Ok(Some(MergeEvidence::PullRequest {
                    number: pr.number,
                    url: pr.url.clone(),
                }));
            }
        }
        Ok(None)
    }

    /// Tip reachable from `base` but not on its first-parent chain.
    fn merged_into(&self, tip: &str, base: &str) -> Result<bool, GitError> {
        if !self.git.is_ancestor(tip, base)? {
            return Ok(false);
        }
        Ok(!self.on_first_parent_chain(tip, base)?)
    }

    /// Assumes `tip` is an ancestor of `base`. Walking `base`'s first parents while
    /// excluding everything reachable from `tip` stops at the first commit whose first
    /// parent is reachable from `tip`; that parent is `tip` itself iff `tip` lies on
    /// the chain.
    fn on_first_parent_chain(&self, tip: &str, base: &str) -> Result<bool, GitError> {
        let range = format!("{tip}..{base}");
        let walk = self
            .git
            .run(["rev-list", "--first-parent", range.as_str()])?;
        match walk.lines().last() {
            // base == tip.
            None => Ok(true),
            Some(last) => Ok(self.git.rev_parse(&format!("{last}^1"))?.as_deref() == Some(tip)),
        }
    }
}
