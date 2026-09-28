```yaml
Overview:
  description: >
    `wt` is a Rust CLI for managing git worktrees laid out by branch name under a
    configured directory relative to the repository's primary checkout. A branch
    `feat/x` lives at `<root>/feat/x`. It creates/switches to worktrees (checkout),
    removes worktrees whose PRs merged (clean), and brings every worktree up to date
    with its upstream and the base branch (sync).
  subsystems:
    cli: clap definitions (src/cli.rs); flags are converted to typed policies
      (DirtyPolicy, Integration, FetchPolicy) before reaching commands.
    git: typed wrapper over the git CLI (src/git/) - command runner, porcelain
      parsers for `worktree list` and `status`, ref/ancestry/divergence helpers.
    repo: repository context (src/repo.rs) - primary checkout, worktree root,
      branch->path mapping, managed-worktree discovery, base-branch resolution.
    config: layered configuration (src/config.rs) - defaults < global json <
      repo `.wt.json` < `git config wt.*`.
    shell: shell integration (src/shell.rs) - `wt init <shell>` wrapper function and
      the WT_CD_FILE cd-directive channel.
    commands: one module per subcommand (src/commands/) consuming repo + git.
  data_flow: >
    main parses Cli -> (init short-circuits) -> Repo::discover(cwd) runs
    `git worktree list` to find the primary checkout, loads Config from it and
    resolves the worktree root -> the command module queries Repo/Git, performs git
    operations, and prints a report to stdout. Directory changes are requested via
    shell::request_cd, which writes the path to $WT_CD_FILE for the shell wrapper
    (or prints it to stdout without the wrapper). Commands return an ExitCode;
    errors surface as anyhow chains over typed thiserror errors.
Features Index:
  core:
    description: Shared git wrapper, config, repo discovery, branch-name types, shell wrapper, test fixture
    entry_points: [src/main.rs, src/lib.rs, "wt init <bash|zsh|fish>"]
    depends_on: []
    doc: docs/features/core.md
  checkout:
    description: Switch to (or create) the worktree for a branch
    entry_points: [src/commands/checkout.rs, "wt checkout <branch>"]
    depends_on: [core]
    doc: docs/features/checkout.md
  clean:
    description: Remove worktrees whose PRs merged; skip, scratch-save or delete dirty ones
    entry_points: [src/commands/clean.rs, "wt clean [--scratch|--delete-dirty]"]
    depends_on: [core]
    doc: docs/features/clean.md
  sync:
    description: Pull upstream and integrate the base branch into every worktree
    entry_points: [src/commands/sync/mod.rs, "wt sync [--stash-pop] [--rebase] [--remote-only]"]
    depends_on: [core]
    doc: docs/features/sync.md
```
