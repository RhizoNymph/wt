# Checkout

`wt checkout <branch> [--from <rev>]` switches to the worktree for `branch`, creating it
at `<root>/<branch with / as subdirectories>` when none exists.

## Scope
- Finding an existing worktree for a branch (anywhere, including the primary checkout) and cd'ing to it.
- Creating a worktree from an existing local branch, a remote-only branch (tracking it),
  or a brand-new branch (from `--from`, else `<remote>/<base>`, else local `<base>`).
- Refreshing the relevant remote ref first, tolerating an unreachable remote.
- Refusing to create a worktree over a non-empty path or another worktree.
- Pruning a stale registration for the branch whose directory was deleted.

## Non-scope
- Removing worktrees (`clean`) or updating them (`sync`).
- Moving an existing worktree to its expected path (a worktree elsewhere is reused as-is).
- Resetting or fast-forwarding an existing local branch; it is checked out unchanged.
- Fetching the whole remote; only the target branch (and the base branch for new branches).

## Data / control flow
`run(repo, args)` calls `checkout(repo, branch, from) -> Result<Outcome, CheckoutError>`, prints
the outcome to stderr and calls `shell::request_cd(outcome.path())`.

`checkout`:
1. `repo.path_for(branch)` computes the target path; `scratch/...` fails with `RepoError::ReservedPath`
   before any git mutation.
2. `repo.worktrees()` lists all worktrees. If one has `branch` checked out:
   - its directory exists → `Outcome::Existing { path }` (that worktree's path, even if it is
     the primary or somewhere other than the expected path);
   - its directory is gone → `git worktree prune`, re-list, and continue to creation.
3. `occupant(path, worktrees)`: a registered worktree at `path` → `Occupant::Worktree { head }`;
   a file or non-empty directory → `Occupant::Files`; missing or empty directory → free.
   Occupied → `CheckoutError::PathOccupied`.
4. `resolve_source`:
   - `refs/heads/<branch>` exists → `Source::Local` (no fetch).
   - else `git fetch --quiet --no-tags <remote> <branch>` (classified by `fetch_branch` as
     `Fetched`, `Missing` — stderr contains "couldn't find remote ref" — or `Unreachable`, which
     prints a warning and continues). If `refs/remotes/<remote>/<branch>` now exists →
     `Source::Remote { upstream: "<remote>/<branch>" }`.
   - else with `--from` → validate via `rev_parse` (`CheckoutError::UnknownStartPoint` if not a
     commit) → `Source::New { start: rev }`.
   - else resolve `repo.base_branch()`, fetch it (skipped if the first fetch found the remote
     unreachable), and use `<remote>/<base>` if that ref exists, else local `<base>`.
   `--from` is ignored with a warning for the `Local` and `Remote` cases.
5. Create parent directories, then `git worktree add --quiet` with
   - `Local`: `<path> <branch>`
   - `Remote`: `--track -b <branch> <path> <remote>/<branch>`
   - `New`: `--no-track -b <branch> <path> <start>`
   → `Outcome::Created { path, source }`.

Output: stderr gets `Worktree for <b> already exists at <p>` or
`Created worktree for <b> at <p> (existing local branch | tracking <upstream> | new branch from <start>)`.
stdout is empty with the shell wrapper, else exactly `<path>\n` (from `request_cd`).

## Files
| File | Role | Key exports |
|---|---|---|
| `src/commands/checkout.rs` | Command implementation and unit tests | `run`, `checkout`, `Outcome`, `Source`, `Occupant`, `CheckoutError` |
| `src/cli.rs` | `CheckoutArgs { branch: BranchName, from: Option<String> }` | `CheckoutArgs` |
| `src/repo.rs` | `path_for`, `worktrees`, `base_branch`, `remote`, `git` | `Repo` |
| `src/git/mod.rs` | `local_branch_exists`, `remote_branch_exists`, `rev_parse`, `output`/`run` | `Git` |
| `src/shell.rs` | cd directive delivery | `request_cd` |
| `tests/checkout.rs` | Integration tests against the hermetic fixture | — |

## Invariants and constraints
- Never clobbers: a non-empty target path or a path registered to another worktree is an error,
  and nothing (branch, directory, worktree) is created in that case.
- An existing worktree is never modified; an existing local branch is never reset.
- A new branch never gets an upstream (`--no-track`); a remote-only branch always tracks
  `<remote>/<branch>` (`--track`).
- Only spawn failures of `git fetch` are fatal; network/remote failures degrade to local refs.
- `--from` is validated before any directory or worktree is created.
- All git calls run in the primary checkout, so behaviour is identical from any worktree.
- User messages go to stderr; stdout carries only the cd path (and only without the wrapper).
