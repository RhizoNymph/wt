//! Layered configuration: defaults < global file < repo file < git config.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::branch::BranchName;
use crate::error::ConfigError;
use crate::git::Git;

/// Env var overriding the global config file location.
pub const CONFIG_ENV: &str = "WT_CONFIG";
/// Per-repo config file, looked up in the primary checkout.
pub const REPO_CONFIG_FILE: &str = ".wt.json";
pub const DEFAULT_WORKTREE_DIR: &str = "..";
pub const DEFAULT_REMOTE: &str = "origin";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Worktree root; relative paths resolve against the primary checkout.
    pub worktree_dir: PathBuf,
    /// Explicit base branch; `None` means auto-detect from `<remote>/HEAD`.
    pub base_branch: Option<BranchName>,
    pub remote: String,
}

/// One source of configuration; every field optional so layers can be merged.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    worktree_dir: Option<PathBuf>,
    base_branch: Option<String>,
    remote: Option<String>,
}

impl Layer {
    fn read(path: &Path) -> Result<Option<Self>, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| ConfigError::Parse {
                path: path.to_owned(),
                source,
            })
    }

    fn from_git(git: &Git) -> Result<Self, ConfigError> {
        Ok(Self {
            worktree_dir: git.config_get("wt.dir")?.map(PathBuf::from),
            base_branch: git.config_get("wt.base")?,
            remote: git.config_get("wt.remote")?,
        })
    }

    /// Overlay `other` on top of `self`; set fields in `other` win.
    fn overlay(self, other: Self) -> Self {
        Self {
            worktree_dir: other.worktree_dir.or(self.worktree_dir),
            base_branch: other.base_branch.or(self.base_branch),
            remote: other.remote.or(self.remote),
        }
    }
}

impl Config {
    /// Path of the global config file: `$WT_CONFIG`, else `<config_dir>/wt/config.json`.
    pub fn global_path() -> Option<PathBuf> {
        std::env::var_os(CONFIG_ENV)
            .map(PathBuf::from)
            .or_else(|| dirs::config_dir().map(|d| d.join("wt").join("config.json")))
    }

    /// Load config for the repo whose primary checkout is `primary`.
    pub fn load(primary: &Path) -> Result<Self, ConfigError> {
        let mut layer = Layer::default();
        if let Some(global) = Self::global_path()
            && let Some(l) = Layer::read(&global)?
        {
            layer = layer.overlay(l);
        }
        if let Some(l) = Layer::read(&primary.join(REPO_CONFIG_FILE))? {
            layer = layer.overlay(l);
        }
        layer = layer.overlay(Layer::from_git(&Git::new(primary))?);
        Ok(Self {
            worktree_dir: layer
                .worktree_dir
                .unwrap_or_else(|| PathBuf::from(DEFAULT_WORKTREE_DIR)),
            base_branch: layer
                .base_branch
                .as_deref()
                .map(BranchName::parse)
                .transpose()?,
            remote: layer.remote.unwrap_or_else(|| DEFAULT_REMOTE.to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_prefers_later_layer() {
        let a = Layer {
            worktree_dir: Some("a".into()),
            base_branch: Some("main".into()),
            remote: None,
        };
        let b = Layer {
            worktree_dir: Some("b".into()),
            base_branch: None,
            remote: Some("up".into()),
        };
        let merged = a.overlay(b);
        assert_eq!(merged.worktree_dir, Some(PathBuf::from("b")));
        assert_eq!(merged.base_branch.as_deref(), Some("main"));
        assert_eq!(merged.remote.as_deref(), Some("up"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("c.json");
        std::fs::write(&path, r#"{"worktree_dir": "x", "bogus": 1}"#).expect("write");
        assert!(matches!(Layer::read(&path), Err(ConfigError::Parse { .. })));
    }

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            Layer::read(&dir.path().join("nope.json")),
            Ok(None)
        ));
    }
}
