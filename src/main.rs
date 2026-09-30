use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use wt::cli::{Cli, Command};
use wt::commands;
use wt::progress::{Display, Progress};
use wt::repo::Repo;
use wt::shell;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Commands print user-facing warnings in their reports, so logs stay quiet by
/// default (errors only) and `-v` exposes the detail.
fn init_tracing(verbose: u8) {
    let default = match verbose {
        0 => "error",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_env("WT_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    if let Command::Init(args) = &cli.command {
        print!("{}", shell::init_script(args.shell));
        return Ok(ExitCode::SUCCESS);
    }
    let cwd = match cli.dir {
        Some(dir) => dir,
        None => std::env::current_dir().context("reading current directory")?,
    };
    let repo = Repo::discover(&cwd)?;
    tracing::debug!(primary = %repo.primary().display(), root = %repo.root().display(), "repo");
    let progress = Progress::new(Display::detect(cli.quiet, cli.verbose));
    match &cli.command {
        Command::Checkout(args) => commands::checkout::run(&repo, args, &progress),
        Command::Clean(args) => commands::clean::run(&repo, args, &progress),
        Command::Sync(args) => commands::sync::run(&repo, args, &progress),
        Command::Init(_) => Ok(ExitCode::SUCCESS),
    }
}
