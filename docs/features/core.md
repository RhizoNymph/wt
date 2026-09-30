# Core

## Scope
- Git subprocess runner and parsers (`git worktree list --porcelain -z`, `git status --porcelain=v1 -z`).
- Validated `BranchName` type and branch → path mapping.
- Layered configuration and repository discovery.
- Shell integration (`wt init`) and the cd-directive channel.
- Shared integration-test fixture.

## Non-scope
- Subcommand behavior (see `checkout.md`, `clean.md`, `sync.md`).
- Talking to GitHub (owned by `clean`).

## Data / control flow
1. `main` parses `Cli`. `wt init <shell>` prints `shell::init_script` and exits.
2. `Repo::discover(cwd)`: `git rev-parse --git-dir` confirms a repo; the first entry of
   `git worktree list` is the primary checkout (canonicalized).
3. `Config::load(primary)` overlays: defaults → `$WT_CONFIG` or `<config_dir>/wt/config.json`
   → `<primary>/.wt.json` → `git config wt.dir|wt.base|wt.remote`.
4. Worktree root = `normalize(primary / worktree_dir)` (lexical; may not exist yet).
5. Commands use `Repo::path_for(branch)` (`root / branch-components`),
   `Repo::managed_worktrees()` (non-bare worktrees under root, outside `root/scratch`),
   and `Repo::base_branch()` (config → `refs/remotes/<remote>/HEAD` → local `main`/`master`).
6. To move the user, commands call `shell::request_cd(path)`: writes to `$WT_CD_FILE`
   when the wrapper is active, else prints the path on stdout.

## Files
| File | Role | Key exports |
|---|---|---|
| `src/main.rs` | Entry: tracing init, dispatch, error → exit code | — |
| `src/lib.rs` | Module tree | — |
| `src/cli.rs` | clap types, help text (about/long_about/examples) and flag → policy conversion | `Cli`, `Command`, `*Args`, `DirtyPolicy`, `Integration`, `FetchPolicy` |
| `src/error.rs` | Typed errors | `GitError`, `BranchNameError`, `ConfigError`, `RepoError` |
| `src/branch.rs` | Branch name validation, path mapping | `BranchName` |
| `src/git/mod.rs` | Git runner and ref helpers (incl. `ignored_paths`) | `Git`, `Divergence` |
| `src/git/worktree.rs` | Worktree list parser | `Worktree`, `HeadKind`, `parse_porcelain` |
| `src/git/status.rs` | Status parser | `WorktreeStatus`, `StatusEntry`, `ChangeKind` |
| `src/config.rs` | Layered config | `Config`, `CONFIG_ENV`, `REPO_CONFIG_FILE`, `DEFAULT_SCRATCH_EXCLUDE` |
| `src/repo.rs` | Repo context and discovery | `Repo`, `SCRATCH_DIR`, `normalize` |
| `src/shell.rs` | Wrapper scripts, cd directive | `Shell`, `init_script`, `request_cd`, `CdDelivery`, `CD_FILE_ENV` |
| `tests/common/mod.rs` | Hermetic fixture (bare origin, primary at `repo/main`, `other` clone, fake `bin/gh`) | `Fixture`, `WtOutput` |

## Config file format
```json
{ "worktree_dir": "..", "base_branch": "main", "remote": "origin",
  "scratch_exclude": ["target", "node_modules"] }
```
All fields optional; unknown fields are rejected. `scratch_exclude` (file layers only,
replaces the default list) names path components `clean --scratch` never copies from
gitignored content; see `clean.md`.

## Invariants and constraints
- Help: bare `wt`, `wt checkout` and `wt init` print help (with setup/examples) instead
  of a missing-argument error (`arg_required_else_help`); `-C`/`-v` are listed under
  "Global options". Help text must stay in sync with command behavior (`tests/help.rs`).
- `BranchName` enforces git ref-format rules, so `rel_path()` never contains `.`/`..`
  components or an absolute prefix: worktree paths cannot escape the root.
- Branches whose first component is `scratch` are rejected by `path_for` (reserved for `clean --scratch`).
- Logging: default level is `error` because commands report warnings (fetch fallback,
  skipped worktrees, kept stashes) in their own stdout reports; `-v`/`-vv`/`WT_LOG`
  expose tracing logs. ANSI colors only when stderr is a terminal.
- Every git call sets `LC_ALL=C` and `GIT_TERMINAL_PROMPT=0`; network failures never hang on prompts.
- `Git::check` treats exit 1 as `false` and any other non-zero exit as an error.
- The git stash stack is shared by all worktrees of a repo; code must never use bare
  `stash pop` (see `sync.md`).
- Tests run with isolated `HOME`, `GIT_CONFIG_GLOBAL`, `WT_CONFIG`, and `WT_GH`.
