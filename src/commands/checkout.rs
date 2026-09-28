//! `wt checkout`.

use std::process::ExitCode;

use crate::cli::CheckoutArgs;
use crate::repo::Repo;

pub fn run(_repo: &Repo, _args: &CheckoutArgs) -> anyhow::Result<ExitCode> {
    anyhow::bail!("`wt checkout` is not implemented yet")
}
