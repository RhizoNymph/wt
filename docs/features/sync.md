# Sync

`wt sync [--stash-pop] [--rebase] [--remote-only]`

## Scope
- Fetch the remote once (`git fetch --prune <remote>` in the primary checkout).
- For every managed worktree (under the worktree root, outside `scratch/`, non-bare),
  sequentially:
  1. pull the branch's own upstream (fast-forward, or merge/rebase when diverged);
  2. integrate the base branch (fast-forward, or merge/rebase when diverged).
- Optionally stash dirty changes first and restore them afterwards (`--stash-pop`).
- Abort any merge/rebase that conflicts so the worktree returns to its prior state.
- Print a typed report; exit non-zero if any worktree failed or its stash could not be restored.

## Non-scope
- Pushing (sync never pushes, even after a rebase that makes the branch diverge from its upstream).
- Worktrees outside the configured root, detached worktrees, and bare entries (skipped/ignored).
- Updating local branches that are not checked out in a managed worktree.
- Resolving conflicts; parallel processing of worktrees.

## Flags → typed policies
| Flag | Type | Effect |
|---|---|---|
| `--rebase` | `Integration::{Merge, Rebase}` | How diverged history is combined (upstream and base steps). |
| `--remote-only` | `FetchPolicy::{FallBackToLocal, RemoteOnly}` | Whether a failed fetch aborts the run. |
| `--stash-pop` | `bool` | Dirty worktrees are stashed/restored instead of skipped. |

## Data / control flow
1. `run(repo, args)` → `sync(repo, args)` → prints `SyncReport` → `ExitCode::FAILURE` if
   `report.has_failures()`.
2. **Fetch** (`mod.rs::fetch`): `git fetch --prune --quiet <remote>` in the primary.
   - success → `Mode::Remote`;
   - failure + `RemoteOnly` → `SyncError::Fetch` (returned before anything is touched);
   - failure + `FallBackToLocal` → `tracing::warn!`, `Mode::Local { fetch_error }`; the
     report header says local refs (possibly stale) are used.
3. **Base ref** (`refs::resolve_base`), resolved once before any worktree is modified:
   - remote mode: `refs/remotes/<remote>/<base>` if present, else `refs/heads/<base>`;
   - local mode: if both exist and local is an ancestor of remote-tracking, use
     remote-tracking; otherwise (local newer or diverged) use local `<base>`;
     whichever exists if only one does.
   - neither → `SyncError::NoBaseRef`.
   Refs are passed to git by full name (`RefName.full`), displayed by short name.
4. **Worktrees**: `repo.managed_worktrees()`, stably sorted so worktrees on the base
   branch come first, processed one at a time (`worktree::sync_worktree`).
5. **Per worktree** (`worktree.rs`):
   - `HeadKind::Detached` / `Bare` → `Skipped`; directory missing → `Skipped(Missing)`.
   - Preflight: `git status` + `refs::operation_in_progress` (`MERGE_HEAD`, `rebase-merge`,
     `rebase-apply`, `CHERRY_PICK_HEAD`, `REVERT_HEAD` via `git rev-parse --git-path`) or
     unmerged status entries → `Skipped(InProgress(op))`, regardless of `--stash-pop`.
   - Dirty and no `--stash-pop` → `Skipped(Dirty)`; worktree untouched.
   - Dirty with `--stash-pop` → `StashedChanges::push` (see Stash safety).
   - `run_steps`:
     - **upstream**: `refs::upstream(git, B)` (`B@{upstream}` must resolve; in local mode
       it is whatever was last fetched). None → `UpstreamStep::NoUpstream`. Else
       `integrate::integrate`.
     - **base**: if `B == base branch` → `BaseStep::IsBase` (no self-integration). Else
       `integrate::integrate(base_ref)`.
     - A failure in the upstream step skips the base step.
   - `integrate`: `divergence(HEAD, target)`; `behind == 0` → up to date; `ahead == 0` →
     `git merge --ff-only`; otherwise `git merge --no-edit --ff` or `git rebase`.
     On a failed merge/rebase: if the operation started (`MERGE_HEAD` / rebase dir exists)
     run `--abort` and return `IntegrateError::Conflict { aborted }`; otherwise git refused
     up front and nothing changed → `IntegrateError::Git`.
   - Stash restore: if the step failed and the abort itself failed (worktree still
     mid-operation) → `StashedChanges::keep` (entry kept, SHA reported); otherwise
     `StashedChanges::restore`.
6. **Report** (`report.rs`): header (`Remote mode: fetched origin; base origin/main` or
   `Local mode: could not fetch …; using local refs, which may be stale; base …`), one line
   per worktree `  <branch>  <path>  <outcome>`, and a `N synced, M skipped, K failed` footer.
   Example outcomes: `up to date`, `up to date (no upstream)`,
   `fast-forwarded from origin/feat/x (+3), then merged origin/main (+5); stash restored`,
   `rebased onto origin/main (+2)`, `skipped: dirty (use --stash-pop)`,
   `skipped: merge in progress`, `failed: conflict merging origin/main (aborted)`,
   `…; restoring stash conflicted; changes kept in stash <sha> (git stash apply <sha>)`.

## Stash safety
The stash stack (`refs/stash` and its reflog) is shared by every worktree of a repository,
so position-based operations (`git stash pop`, `stash@{0}`) could grab someone else's entry.
- Push: `git stash push --include-untracked -m "wt-sync <pid>-<nanos>-<counter>"`.
- Identify: `refs/stash` right after the push, accepted only if its subject ends with the
  unique message; otherwise search `git stash list` for it. Not found → `StashError::NotCreated`
  (worktree reported failed, nothing else touched).
- Restore: `git stash apply --index <sha>`; if that fails **and the worktree is still clean**
  (nothing partially applied), retry `git stash apply <sha>`. Any remaining failure →
  `StashOutcome::Conflicted { sha }`: the entry is kept and the SHA printed.
- Drop: find the entry's current selector via `git stash list --format=%gd%x00%H%x00%s`
  matching the SHA and `git stash drop <selector>`. Other entries keep their order.
- `StashedChanges` is `#[must_use]` and consumed by `restore` or `keep`; dropping it while
  still armed logs an error with the SHA.

## Files
| File | Role | Key exports |
|---|---|---|
| `src/commands/sync/mod.rs` | Entry point, fetch, base resolution, orchestration | `run`, `sync`, `SyncError`, re-exports report types |
| `src/commands/sync/worktree.rs` | Per-worktree preflight, stash, steps, restore | `Plan`, `sync_worktree` |
| `src/commands/sync/integrate.rs` | ff/merge/rebase with abort-on-conflict | `integrate`, `IntegrateError` |
| `src/commands/sync/stash.rs` | SHA-addressed stash guard | `StashedChanges`, `StashError` |
| `src/commands/sync/refs.rs` | Base/upstream ref resolution, in-progress detection | `RefName`, `BaseSource`, `resolve_base`, `upstream`, `git_path`, `operation_in_progress` |
| `src/commands/sync/report.rs` | Outcome enums and `Display` | `SyncReport`, `WorktreeReport`, `WorktreeOutcome`, `UpstreamStep`, `BaseStep`, `Integrated`, `Applied`, `SkipReason`, `Operation`, `FailReason`, `ConflictAction`, `StashOutcome`, `Mode` |
| `src/cli.rs` | `SyncArgs`, `Integration`, `FetchPolicy` (core) | — |
| `tests/sync.rs` | Integration tests against the hermetic fixture | — |

## Invariants and constraints
- Fetch happens once, in the primary, before any worktree is modified; `--remote-only`
  failures therefore change nothing.
- Ordering per worktree is always upstream, then base; the base worktree only pulls.
- Worktrees are processed sequentially (shared git dir, index locks, stash stack).
- A conflicting merge/rebase is always aborted; the worktree ends at its pre-sync HEAD
  with its local changes restored. Only if the abort fails is the worktree left
  mid-operation, and then the stash is kept rather than applied on top.
- Stash entries are addressed by SHA only; never `pop`; unrelated entries are untouched.
- Skips (dirty, detached, in progress, missing) are not failures; failed integration and
  unrestored stashes are (non-zero exit).
- Never pushes.
- Note: `git worktree add -b B <path> origin/main` auto-tracks `origin/main` (git's default
  `branch.autoSetupMerge`), so such branches pull `origin/main` as their "upstream" step.
