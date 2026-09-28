# Clean

`wt clean [--scratch | --delete-dirty] [--dry-run]` removes worktrees (and their local
branches) whose work has been merged.

## Scope
- Choosing candidate worktrees under the worktree root and reporting the ones skipped
  (locked, detached, no commits).
- Pruning stale worktree registrations whose directory no longer exists.
- Merge detection: GitHub pull requests via the `gh` CLI, then git ancestry.
- Handling dirty merged worktrees per `DirtyPolicy`: skip (default), save changed files
  to `<root>/scratch/` (`--scratch`), or discard (`--delete-dirty`).
- Removing now-empty parent directories below the root.
- Moving the user's shell out of a removed worktree.
- A typed report (`CleanReport`) printed to stdout; non-zero exit on per-worktree errors.

## Non-scope
- Deleting remote branches or closing PRs.
- Worktrees outside the worktree root, the primary checkout, the base branch's worktree,
  and anything under `<root>/scratch` (never candidates).
- Cleaning up old scratch directories.
- Preserving gitignored files outside `--scratch`: default and `--delete-dirty` runs
  let `git worktree remove` delete them (git semantics), and ignored files never make a
  worktree count as dirty.

## Data / control flow
1. `run` captures the canonical current directory (it may vanish later), resolves the
   base branch (`Repo::base_branch`).
2. `git fetch --quiet <remote>` once in the primary checkout. Failure is logged at warn,
   recorded as `FetchOutcome::Failed`, and the run continues with local refs. Dry runs
   fetch too (only remote-tracking refs change).
3. `github::merged_prs` runs `$WT_GH` (default `gh`) in the primary checkout:
   `pr list --state merged --limit 1000 --json headRefName,number,url,headRefOid`,
   building a `PrIndex` keyed by head branch name. On any `GhError` the run uses
   `Detection::GitOnly` (not installed → info log; failure/unparseable → warn log).
4. `MergeDetector::new` records which base refs exist: `refs/remotes/<remote>/<base>`
   and `refs/heads/<base>`.
5. `candidates::classify` over `Repo::managed_worktrees()`:
   primary and base-branch worktree → dropped silently; locked → `Skipped(Locked)`;
   prunable → `prunable`; detached → `Skipped(Detached)`; unborn → `Skipped(Unborn)`;
   otherwise → `Target { branch, path, tip }`. Sorted by path.
6. If anything is prunable: `git worktree prune` (not in dry run); listed under "Pruned".
7. For each target, `process` (errors are caught per target, recorded as `Failure`,
   and processing continues):
   1. `MergeDetector::evaluate` → `MergeStatus::{Merged(MergeEvidence), NotMerged}`.
      Not merged → counted.
   2. `Git::new(path).status()`; with `--scratch` also `scratch::ignored_to_save`
      (`git ls-files --others --ignored --exclude-standard --directory`, minus paths with
      a component in `config.scratch_exclude`).
   3. Clean (and, with `--scratch`, no ignored files left after exclusion) → `git worktree remove <path>` (no force: git itself refuses if the tree
      became dirty in the meantime) → `git branch -D <branch>` → remove empty parents.
   4. Dirty + `Skip` → `SkippedDirty` with modified/untracked counts.
   5. Dirty + `Delete` → `git worktree remove --force`, `branch -D`, parents.
   6. Dirty or has saveable ignored files + `Scratch` → `ScratchPlan::new` (every status
      path that exists on disk is copied; others are listed as not copied with their
      `ChangeKind`; ignored paths are copied too, recursing into ignored directories
      while skipping excluded names), `scratch::save`
      claims a fresh directory and copies the files preserving relative paths, and only
      after every copy succeeds is the worktree force-removed and the branch deleted.
      If nothing exists to copy, no scratch directory is created.
   In a dry run every mutating step is skipped; scratch destinations are previewed.
8. Print the report. If not a dry run and the captured cwd was inside a removed
   worktree (and no longer exists), `shell::request_cd(primary)`.
9. Exit `FAILURE` if the report has errors, else `SUCCESS`.

## Merge detection rule (safety)
A branch counts as merged only if its tip holds no commit missing from either the base
branch or a merged PR's head:

- **Pull request** (`MergeEvidence::PullRequest`): some merged PR has
  `headRefName == branch` and the PR's `headRefOid` equals the local tip, or the local
  tip is an ancestor of `headRefOid` (the head object must exist locally). Covers squash
  and rebase merges. Local commits made after the merge disqualify the branch.
- **Ancestry** (`MergeEvidence::AncestorOfBase`), checked against `<remote>/<base>` then
  local `<base>`: the tip is an ancestor of the base ref **and not on its first-parent
  chain**. A tip on the first-parent chain (including tip == base) is indistinguishable
  from a branch created from base and never worked on, so it is treated as not merged.
  Consequence: a fast-forward merge is recognised only through PR evidence. Squash and
  rebase merges without `gh` are not recognised (the branch is kept).

First-parent check: `git rev-list --first-parent <tip>..<base>`; the last commit listed
has a first parent reachable from `<tip>`, which equals `<tip>` iff the tip is on the
chain. An empty list means tip == base.

## Scratch layout
`<root>/scratch/<branch components>/<relative file path>`, e.g. `feat/x`'s
`notes/a.md` → `<root>/scratch/feat/x/notes/a.md`. An existing scratch directory is
never written into: the first free name among `x`, `x-2`, … `x-1000` is claimed with an
atomic `create_dir`; if all are taken the worktree fails (kept) with
`CleanError::ScratchExhausted`. Symlinks are recreated as symlinks; untracked
directories reported whole (e.g. nested repositories) are copied recursively.

Gitignored files (`.env`, local config, notes) are saved alongside dirty files. Names in
`scratch_exclude` (config; default `target`, `node_modules`, `.venv`, `venv`,
`__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.gradle`, `.next`) are
skipped at any depth inside ignored content, so rebuildable build/dependency output is
not copied. The exclusion never applies to dirty (non-ignored) files.

## Files
| File | Role | Key exports |
|---|---|---|
| `src/commands/clean/mod.rs` | Orchestration: fetch, detection source, classify, per-target processing, report, cd request | `run`, re-exports `CleanReport`, `CleanError`, `GhError`, `MergeEvidence`, `MergeStatus` |
| `src/commands/clean/candidates.rs` | Worktree classification | `Target`, `Skipped`, `SkipReason`, `Classified`, `classify` |
| `src/commands/clean/github.rs` | `gh pr list` invocation and parsing | `GH_ENV`, `PR_LIMIT`, `MergedPr`, `PrIndex`, `merged_prs`, `parse_prs` |
| `src/commands/clean/merged.rs` | Merge evidence and the safety rule | `MergeEvidence`, `MergeStatus`, `Detection`, `MergeDetector` |
| `src/commands/clean/scratch.rs` | Scratch planning, destination choice, copying | `ScratchPlan`, `NotCopied`, `Exclude`, `ignored_to_save`, `base_dest`, `preview_dest`, `save` |
| `src/commands/clean/remove.rs` | Worktree + branch removal, empty-parent cleanup | `Force`, `remove`, `remove_empty_parents` |
| `src/commands/clean/report.rs` | Typed report and its `Display` | `CleanReport`, `Outcome`, `Removed`, `Scratched`, `SkippedDirty`, `Failure`, `DirtyCounts`, `DetectionMode`, `FetchOutcome` |
| `src/commands/clean/error.rs` | Typed errors | `CleanError`, `GhError` |
| `tests/clean.rs` | Integration tests (fake `gh` via `WT_GH`) | — |

## Invariants and constraints
- Never removes the primary checkout, the base branch's worktree, locked worktrees,
  detached worktrees, or anything outside `<root>` or inside `<root>/scratch`.
- Never removes a worktree whose branch tip has commits absent from both the base refs
  and every matching merged PR head (see the rule above).
- Clean worktrees are removed without `--force`, so a worktree that became dirty after
  the status check is refused by git rather than lost.
- `--scratch` removes a worktree only after all copies succeed; existing scratch
  directories are never overwritten.
- `remove_empty_parents` stops at the first non-empty directory and never removes the
  root.
- `--dry-run` performs no worktree, branch, filesystem, or prune changes (it does fetch).
- Per-worktree failures do not stop the run; any failure makes the exit code non-zero.
- `--scratch` and `--delete-dirty` are mutually exclusive (clap `conflicts_with`).
- Only the most recent `PR_LIMIT` (1000) merged PRs are consulted; older merges fall
  back to ancestry.
