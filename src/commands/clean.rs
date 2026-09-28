//! `wt clean`.

use std::process::ExitCode;

use crate::cli::CleanArgs;
use crate::repo::Repo;

pub fn run(_repo: &Repo, _args: &CleanArgs) -> anyhow::Result<ExitCode> {
    anyhow::bail!("`wt clean` is not implemented yet")
}
