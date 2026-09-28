//! `wt sync`.

use std::process::ExitCode;

use crate::cli::SyncArgs;
use crate::repo::Repo;

pub fn run(_repo: &Repo, _args: &SyncArgs) -> anyhow::Result<ExitCode> {
    anyhow::bail!("`wt sync` is not implemented yet")
}
