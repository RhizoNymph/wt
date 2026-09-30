# Progress

## Scope
- Live progress on stderr for `checkout`, `clean` and `sync`: open-ended phases
  (fetching, querying GitHub, creating a worktree) and a counted bar over worktrees,
  with a per-step detail message (`feat/x: pulling origin/feat/x`).
- Plain-line fallback when stderr is not a terminal or `-v` logging is on.
- `-q/--quiet` to hide progress.

## Non-scope
- Final reports and warnings: printed by each command regardless of `--quiet`.
- Progress of git's own transfers (git runs with `--quiet`, output captured).
- `wt init`, which never shows progress.

## Data / control flow
1. `main` builds `Progress::new(Display::detect(cli.quiet, cli.verbose))` after repo
   discovery and passes `&Progress` to the command's `run`.
2. `Display::detect`: `--quiet` → `Hidden`; `-v` or non-terminal stderr → `Lines`;
   otherwise `Bar`.
3. Commands call:
   - `phase(msg)` for open-ended work. Bar: spinner message; Lines: `msg...`.
   - `items(n)` before looping over worktrees (bar switches to `[====>   ] i/n`).
   - `item(label)` / `item_done()` around each worktree. Lines: `[i/n] label`.
   - `detail(msg)` for sub-steps (bar only; lines would be too chatty; `-v` logs cover it).
   - `println(msg)` for warnings mid-run (suspends the bar so the line isn't overwritten).
   - `finish()` before printing the report; `Drop` also clears the bar.
4. Where each command reports:
   - checkout: `fetching <branch> from <remote>`, `fetching <base> from <remote>`,
     `creating worktree at <path>`.
   - clean: `fetching <remote>`, `checking merged pull requests on GitHub`, then items
     per candidate with details (merged check, local changes, saving to scratch, removing).
   - sync: `fetching <remote>`, then items per managed worktree (base first) with
     details (status, stashing, pulling upstream, integrating base, restoring stash).

## Files
| File | Role | Key exports |
|---|---|---|
| `src/progress.rs` | Display detection and the `Progress` handle (indicatif) | `Display`, `Progress` |
| `src/cli.rs` | `-q/--quiet` global flag (conflicts with `-v`) | `Cli::quiet` |
| `src/main.rs` | Builds `Progress`, passes it to commands | — |
| `src/commands/checkout.rs` | Phases for fetches and worktree creation; warnings via `println` | `run`, `checkout` |
| `src/commands/clean/mod.rs` | Phases, items per candidate, details | `run` |
| `src/commands/sync/mod.rs`, `sync/worktree.rs` | Fetch phase, items per worktree, step details | `run`, `sync`, `sync_worktree` |
| `tests/progress.rs` | Lines-mode output, `--quiet`, flag conflict | — |

## Invariants and constraints
- Progress only ever writes to stderr; stdout stays reserved for reports and the cd path.
- The bar is cleared before any report is printed (`finish`, and on drop).
- No ANSI escapes unless stderr is a terminal.
- `-q` and `-v` are mutually exclusive.
- Worktrees are still processed sequentially; progress adds no concurrency.
