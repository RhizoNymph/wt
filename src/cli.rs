//! Command-line interface definitions and the typed policies derived from flags.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::branch::BranchName;
use crate::shell::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "wt",
    version,
    about = "Manage git worktrees laid out by branch name"
)]
pub struct Cli {
    /// Run as if started in this directory.
    #[arg(short = 'C', global = true, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Increase log verbosity (-v debug, -vv trace). `WT_LOG` overrides.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Switch to the worktree for a branch, creating it if needed.
    Checkout(CheckoutArgs),
    /// Remove worktrees whose PRs have been merged.
    Clean(CleanArgs),
    /// Pull remote changes and integrate the base branch into every worktree.
    Sync(SyncArgs),
    /// Print the shell wrapper that lets `wt` change your directory.
    Init(InitArgs),
}

#[derive(Debug, Args)]
pub struct CheckoutArgs {
    /// Branch to check out; `/` in the name becomes a subdirectory.
    pub branch: BranchName,
    /// Start point when creating a brand-new branch (default: the base branch).
    #[arg(long, value_name = "REV")]
    pub from: Option<String>,
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Save dirty files of merged worktrees under <worktree-dir>/scratch, then delete them.
    #[arg(long, conflicts_with = "delete_dirty")]
    pub scratch: bool,
    /// Delete merged worktrees even if they have uncommitted or untracked files.
    #[arg(long)]
    pub delete_dirty: bool,
    /// Report what would happen without changing anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// What `clean` does with a merged worktree that has local changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyPolicy {
    Skip,
    Scratch,
    Delete,
}

impl CleanArgs {
    pub fn dirty_policy(&self) -> DirtyPolicy {
        match (self.scratch, self.delete_dirty) {
            (true, _) => DirtyPolicy::Scratch,
            (false, true) => DirtyPolicy::Delete,
            (false, false) => DirtyPolicy::Skip,
        }
    }
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// Stash dirty changes before syncing and restore them afterwards.
    #[arg(long)]
    pub stash_pop: bool,
    /// Rebase instead of merge when the branch has diverged.
    #[arg(long)]
    pub rebase: bool,
    /// Fail instead of falling back to local refs when the remote is unreachable.
    #[arg(long)]
    pub remote_only: bool,
}

/// How diverged history is combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integration {
    Merge,
    Rebase,
}

/// What to do when fetching from the remote fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchPolicy {
    /// Continue using local branches as the source of truth.
    FallBackToLocal,
    RemoteOnly,
}

impl SyncArgs {
    pub fn integration(&self) -> Integration {
        if self.rebase {
            Integration::Rebase
        } else {
            Integration::Merge
        }
    }

    pub fn fetch_policy(&self) -> FetchPolicy {
        if self.remote_only {
            FetchPolicy::RemoteOnly
        } else {
            FetchPolicy::FallBackToLocal
        }
    }
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(value_enum)]
    pub shell: Shell,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("wt").chain(args.iter().copied()))
    }

    #[test]
    fn clean_policies() {
        let policy = |args: &[&str]| match parse(args).map(|c| c.command) {
            Ok(Command::Clean(a)) => Some(a.dirty_policy()),
            _ => None,
        };
        assert_eq!(policy(&["clean"]), Some(DirtyPolicy::Skip));
        assert_eq!(policy(&["clean", "--scratch"]), Some(DirtyPolicy::Scratch));
        assert_eq!(
            policy(&["clean", "--delete-dirty"]),
            Some(DirtyPolicy::Delete)
        );
        assert!(parse(&["clean", "--scratch", "--delete-dirty"]).is_err());
    }

    #[test]
    fn sync_flags_compose() {
        match parse(&["sync", "--rebase", "--stash-pop", "--remote-only"]).map(|c| c.command) {
            Ok(Command::Sync(a)) => {
                assert_eq!(a.integration(), Integration::Rebase);
                assert_eq!(a.fetch_policy(), FetchPolicy::RemoteOnly);
                assert!(a.stash_pop);
            }
            other => panic!("unexpected parse: {other:?}"),
        }
    }

    #[test]
    fn checkout_validates_branch() {
        assert!(parse(&["checkout", "feat/x"]).is_ok());
        assert!(parse(&["checkout", "bad..name"]).is_err());
    }
}
