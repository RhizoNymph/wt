//! Command-line interface definitions and the typed policies derived from flags.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::branch::BranchName;
use crate::shell::Shell;

const SETUP_AND_EXAMPLES: &str = "\
Setup (lets `wt` change your shell's directory):
  bash/zsh:  eval \"$(wt init bash)\"    # add to ~/.bashrc or ~/.zshrc (use `zsh` there)
  fish:      wt init fish | source       # add to ~/.config/fish/config.fish

Examples:
  wt checkout feat/login       Go to <root>/feat/login, creating the worktree if needed
  wt sync --stash-pop          Update every worktree, keeping uncommitted work
  wt clean --dry-run           Show which merged worktrees would be removed

Run `wt <command> --help` for details on a command.";

const CONFIGURATION: &str = "\
Configuration (later sources win):
  ~/.config/wt/config.json     Global defaults (or the path in $WT_CONFIG)
  <primary>/.wt.json           Per-repository settings
  git config wt.dir|wt.base|wt.remote

  { \"worktree_dir\": \"..\", \"base_branch\": \"main\", \"remote\": \"origin\",
    \"scratch_exclude\": [\"target\", \"node_modules\"] }

  worktree_dir is relative to the primary checkout (default \"..\", so worktrees sit
  next to it). The base branch defaults to <remote>/HEAD, then main or master.";

const LONG_ABOUT: &str = "\
Manage git worktrees laid out by branch name.

Each branch gets its own worktree under a root directory configured relative to the
repository's primary checkout; `/` in a branch name becomes a subdirectory, so
`feat/login` lives at <root>/feat/login.";

#[derive(Debug, Parser)]
#[command(
    name = "wt",
    version,
    about = "Manage git worktrees laid out by branch name",
    long_about = LONG_ABOUT,
    arg_required_else_help = true,
    after_help = SETUP_AND_EXAMPLES,
    after_long_help = format!("{SETUP_AND_EXAMPLES}\n\n{CONFIGURATION}"),
)]
pub struct Cli {
    /// Run as if started in this directory.
    #[arg(
        short = 'C',
        global = true,
        value_name = "DIR",
        help_heading = "Global options"
    )]
    pub dir: Option<PathBuf>,
    /// Increase log verbosity (-v debug, -vv trace). `WT_LOG` overrides.
    #[arg(
        short,
        long,
        action = clap::ArgAction::Count,
        global = true,
        help_heading = "Global options"
    )]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Switch to the worktree for a branch, creating it if needed.
    #[command(
        long_about = "\
Switch to the worktree for a branch, creating it if needed.

The worktree lives at <root>/<branch>, with each `/` in the branch name becoming a
subdirectory. If the branch already has a worktree anywhere, wt says it already exists
and takes you there. Otherwise it creates one:
  - from the local branch, if it exists (never reset);
  - tracking <remote>/<branch>, if the branch only exists on the remote;
  - as a new branch from the latest <remote>/<base> (or --from), otherwise.
If the remote can't be reached, local refs are used.

Changing directory needs the shell wrapper (see `wt init`); without it the path is
printed, so `cd \"$(wt checkout <branch>)\"` also works.",
        arg_required_else_help = true,
        after_help = "\
Examples:
  wt checkout feat/login                    Go to <root>/feat/login
  wt checkout main                          Go to wherever main is checked out
  wt checkout feat/b --from feat/a          Start a new branch on top of feat/a"
    )]
    Checkout(CheckoutArgs),
    /// Remove worktrees whose PRs have been merged.
    #[command(
        long_about = "\
Remove worktrees whose PRs have been merged, and delete their local branches.

A branch counts as merged when GitHub (via `gh`) shows a merged PR for it with no
newer local commits, or, without gh, when the base branch already contains its
commits. Branches that were never worked on, the primary checkout, the base branch,
and locked or detached worktrees are never removed.

Merged worktrees with uncommitted or untracked files are skipped and listed in the
report unless --scratch or --delete-dirty is given. With --scratch, those files and
any gitignored files (except build output such as target/ and node_modules/, see
`scratch_exclude`) are copied to <root>/scratch/<branch>/ before removal.",
        after_help = "\
Examples:
  wt clean --dry-run          Show what would be removed
  wt clean                    Remove merged worktrees, skipping dirty ones
  wt clean --scratch          Save dirty and ignored files to <root>/scratch first"
    )]
    Clean(CleanArgs),
    /// Pull remote changes and integrate the base branch into every worktree.
    #[command(
        long_about = "\
Pull remote changes and integrate the base branch into every worktree.

For each worktree under the root, wt first brings in the branch's upstream
(fast-forwarding when possible), then merges the base branch into it if the base
branch has new commits (or rebases with --rebase). The worktree on the base branch
itself only pulls. Conflicts are aborted, leaving that worktree as it was, and
reported; other worktrees still sync. Nothing is ever pushed.

Worktrees with uncommitted changes are skipped unless --stash-pop is given. If the
remote can't be reached, local branches are used instead unless --remote-only.",
        after_help = "\
Examples:
  wt sync                          Merge upstream and base into clean worktrees
  wt sync --rebase --stash-pop     Rebase everything, keeping uncommitted work"
    )]
    Sync(SyncArgs),
    /// Print the shell wrapper that lets `wt` change your directory.
    #[command(
        long_about = "\
Print the shell wrapper that lets `wt` change your directory.

A program can't change its parent shell's directory, so `wt checkout` (and `wt clean`,
when it removes the worktree you're in) hand the path to a small `wt` shell function,
which does the `cd`. Load it from your shell's startup file.",
        arg_required_else_help = true,
        after_help = "\
Setup:
  bash:  echo 'eval \"$(wt init bash)\"' >> ~/.bashrc
  zsh:   echo 'eval \"$(wt init zsh)\"' >> ~/.zshrc
  fish:  echo 'wt init fish | source' >> ~/.config/fish/config.fish"
    )]
    Init(InitArgs),
}

#[derive(Debug, Args)]
pub struct CheckoutArgs {
    /// Branch to check out; `/` in the name becomes a subdirectory.
    pub branch: BranchName,
    /// Where a new branch starts (default: latest base branch).
    ///
    /// Any commit, branch or tag, e.g. `--from feat/a` to stack on an unmerged branch.
    /// Ignored with a warning when the branch already exists locally or on the remote.
    #[arg(long, value_name = "REV")]
    pub from: Option<String>,
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Copy dirty and gitignored files of merged worktrees to <root>/scratch/<branch>/,
    /// then remove them.
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
    /// Stash uncommitted and untracked changes before syncing and restore them after.
    #[arg(long)]
    pub stash_pop: bool,
    /// Rebase instead of merge when combining diverged history.
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
    /// Shell to print the wrapper for.
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
