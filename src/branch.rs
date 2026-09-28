//! Validated branch names and their mapping onto worktree paths.

use std::fmt;
use std::path::PathBuf;

use crate::error::BranchNameError;

/// A branch name that satisfies git's ref-format rules.
///
/// Because every `/`-separated component is non-empty and never `.`/`..`,
/// [`BranchName::rel_path`] is always a safe relative path with no traversal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BranchName(String);

const FORBIDDEN_CHARS: &[char] = &[' ', '~', '^', ':', '?', '*', '[', '\\'];

impl BranchName {
    pub fn parse(name: &str) -> Result<Self, BranchNameError> {
        let err = |reason| {
            Err(BranchNameError {
                name: name.to_owned(),
                reason,
            })
        };
        if name.is_empty() {
            return err("empty");
        }
        if name == "HEAD" || name == "@" {
            return err("reserved name");
        }
        if name.starts_with('-') {
            return err("starts with '-'");
        }
        if name.ends_with('.') {
            return err("ends with '.'");
        }
        if name.contains("..") {
            return err("contains '..'");
        }
        if name.contains("@{") {
            return err("contains '@{'");
        }
        if name
            .chars()
            .any(|c| c.is_ascii_control() || FORBIDDEN_CHARS.contains(&c))
        {
            return err("contains a forbidden character");
        }
        for component in name.split('/') {
            if component.is_empty() {
                return err("has an empty path component");
            }
            if component.starts_with('.') {
                return err("has a component starting with '.'");
            }
            if component.ends_with(".lock") {
                return err("has a component ending with '.lock'");
            }
        }
        Ok(Self(name.to_owned()))
    }

    /// Parse the branch out of a full ref such as `refs/heads/feat/x`.
    pub fn from_full_ref(full: &str) -> Option<Self> {
        full.strip_prefix("refs/heads/")
            .and_then(|name| Self::parse(name).ok())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn full_ref(&self) -> String {
        format!("refs/heads/{}", self.0)
    }

    /// First `/`-separated component (`feat` for `feat/x`).
    pub fn first_component(&self) -> &str {
        self.0.split('/').next().unwrap_or(&self.0)
    }

    /// Relative path for this branch's worktree: each `/` becomes a directory level.
    pub fn rel_path(&self) -> PathBuf {
        self.0.split('/').collect()
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for BranchName {
    type Err = BranchNameError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn accepts_simple_and_nested_names() {
        for name in ["main", "feat/x", "fix/deep/nested-name", "v1.2", "a@b"] {
            assert!(BranchName::parse(name).is_ok(), "{name} should be valid");
        }
    }

    #[test]
    fn rejects_invalid_names() {
        for name in [
            "", "HEAD", "@", "-x", "a.", "a..b", "a@{1}", "a b", "a~1", "a^", "a:b", "a?", "a*",
            "a[", "a\\b", "/a", "a/", "a//b", ".a", "a/.b", "a.lock", "a/b.lock", "a\tb",
        ] {
            assert!(
                BranchName::parse(name).is_err(),
                "{name:?} should be invalid"
            );
        }
    }

    #[test]
    fn rel_path_splits_on_slashes() {
        let b = BranchName::parse("feat/api/v2").expect("valid");
        assert_eq!(b.rel_path(), Path::new("feat").join("api").join("v2"));
        assert_eq!(b.first_component(), "feat");
    }

    #[test]
    fn from_full_ref_strips_prefix() {
        let b = BranchName::from_full_ref("refs/heads/feat/x").expect("branch ref");
        assert_eq!(b.as_str(), "feat/x");
        assert_eq!(b.full_ref(), "refs/heads/feat/x");
        assert!(BranchName::from_full_ref("refs/tags/v1").is_none());
    }
}
