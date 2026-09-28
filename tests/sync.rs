mod common;

use std::path::{Path, PathBuf};

use common::{Fixture, path_str};

/// Create a worktree for `branch` and publish it with upstream tracking.
fn tracked_worktree(fx: &Fixture, branch: &str) -> PathBuf {
    let wt = fx.add_worktree(branch);
    fx.git(&wt, &["push", "-q", "-u", "origin", branch]);
    wt
}

fn stash_list(fx: &Fixture) -> Vec<String> {
    let out = fx.git(&fx.primary, &["stash", "list", "--format=%H %s"]);
    out.lines().map(str::to_owned).collect()
}

fn merge_count(fx: &Fixture, dir: &Path, range: &str) -> usize {
    fx.git(dir, &["rev-list", "--merges", range])
        .lines()
        .count()
}

fn parents(fx: &Fixture, dir: &Path, rev: &str) -> Vec<String> {
    fx.git(dir, &["rev-list", "--parents", "-n", "1", rev])
        .split_whitespace()
        .skip(1)
        .map(str::to_owned)
        .collect()
}

fn is_ancestor(fx: &Fixture, dir: &Path, ancestor: &str, descendant: &str) -> bool {
    fx.git_output(dir, &["merge-base", "--is-ancestor", ancestor, descendant])
        .status
        .success()
}

fn status(fx: &Fixture, dir: &Path) -> String {
    let out = fx.git_output(dir, &["status", "--porcelain=v1", "--untracked-files=all"]);
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim_end().to_owned()
}

fn git_path_exists(fx: &Fixture, dir: &Path, name: &str) -> bool {
    let p = fx.git(dir, &["rev-parse", "--git-path", name]);
    dir.join(p).exists()
}

fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).expect("read file")
}

#[test]
fn primary_fast_forwards_to_new_origin_main() {
    let fx = Fixture::new();
    let sha = fx.push_upstream("main", "up.txt", "up\n", "upstream change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert_eq!(fx.head(&fx.primary), sha);
    assert!(out.stdout.contains("fast-forwarded"), "{}", out.stdout);
    assert!(out.stdout.contains("main"), "{}", out.stdout);
}

#[test]
fn feature_with_upstream_receives_upstream_commits() {
    let fx = Fixture::new();
    let wt = tracked_worktree(&fx, "feat/x");
    let sha = fx.push_upstream("feat/x", "x.txt", "x\n", "teammate change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert_eq!(fx.head(&wt), sha);
    assert!(out.stdout.contains("origin/feat/x"), "{}", out.stdout);
}

#[test]
fn feature_gets_base_merged_by_default() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    let local = fx.commit(&wt, "feature.txt", "feature\n", "feature work");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    let head = fx.head(&wt);
    assert_eq!(parents(&fx, &wt, &head), vec![local, main_sha]);
    assert_eq!(read(&wt, "base.txt"), "base\n");
    assert_eq!(read(&wt, "feature.txt"), "feature\n");
    assert!(out.stdout.contains("merged origin/main"), "{}", out.stdout);
}

#[test]
fn branch_without_upstream_still_gets_base() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    // Branching from origin/main auto-tracks it; remove that to test the no-upstream path.
    fx.git(&wt, &["branch", "--unset-upstream"]);
    let local = fx.commit(&wt, "feature.txt", "feature\n", "feature work");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert_eq!(parents(&fx, &wt, &fx.head(&wt)), vec![local, main_sha]);
    // Missing upstream is noted, not an error.
    assert!(out.stdout.contains("no upstream"), "{}", out.stdout);
}

#[test]
fn feature_behind_base_only_is_fast_forwarded() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    fx.wt_plain(&fx.primary, &["sync"]).assert_success();
    assert_eq!(fx.head(&wt), main_sha);
}

#[test]
fn rebase_flag_produces_linear_history() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    let local = fx.commit(&wt, "feature.txt", "feature\n", "feature work");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync", "--rebase"]);
    out.assert_success();
    let head = fx.head(&wt);
    assert_ne!(head, local);
    assert!(is_ancestor(&fx, &wt, &main_sha, &head));
    assert_eq!(merge_count(&fx, &wt, &format!("{main_sha}..HEAD")), 0);
    assert_eq!(parents(&fx, &wt, &head), vec![main_sha]);
    assert_eq!(read(&wt, "feature.txt"), "feature\n");
    assert!(
        out.stdout.contains("rebased onto origin/main"),
        "{}",
        out.stdout
    );
}

#[test]
fn diverged_upstream_is_merged_by_default() {
    let fx = Fixture::new();
    let wt = tracked_worktree(&fx, "feat/x");
    let local = fx.commit(&wt, "mine.txt", "mine\n", "local work");
    let theirs = fx.push_upstream("feat/x", "theirs.txt", "theirs\n", "remote work");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    let head = fx.head(&wt);
    assert_eq!(parents(&fx, &wt, &head), vec![local, theirs]);
    assert_eq!(read(&wt, "mine.txt"), "mine\n");
    assert_eq!(read(&wt, "theirs.txt"), "theirs\n");
    assert!(
        out.stdout.contains("merged origin/feat/x"),
        "{}",
        out.stdout
    );
}

#[test]
fn diverged_upstream_is_rebased_with_flag() {
    let fx = Fixture::new();
    let wt = tracked_worktree(&fx, "feat/x");
    fx.commit(&wt, "mine.txt", "mine\n", "local work");
    let theirs = fx.push_upstream("feat/x", "theirs.txt", "theirs\n", "remote work");
    let out = fx.wt_plain(&fx.primary, &["sync", "--rebase"]);
    out.assert_success();
    let head = fx.head(&wt);
    assert_eq!(parents(&fx, &wt, &head), vec![theirs.clone()]);
    assert_eq!(merge_count(&fx, &wt, "origin/main..HEAD"), 0);
    assert_eq!(read(&wt, "mine.txt"), "mine\n");
    assert!(
        out.stdout.contains("rebased onto origin/feat/x"),
        "{}",
        out.stdout
    );
}

#[test]
fn upstream_is_pulled_before_base_is_integrated() {
    let fx = Fixture::new();
    let wt = tracked_worktree(&fx, "feat/x");
    let theirs = fx.push_upstream("feat/x", "theirs.txt", "theirs\n", "remote work");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    let head = fx.head(&wt);
    // Fast-forwarded to the upstream first, then merged the base on top.
    assert_eq!(parents(&fx, &wt, &head), vec![theirs, main_sha]);
}

#[test]
fn dirty_worktree_is_skipped_without_stash_pop() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    let before = fx.head(&wt);
    fx.write(&wt, "README.md", "local edit\n");
    fx.write(&wt, "scratch.txt", "untracked\n");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert_eq!(fx.head(&wt), before);
    assert_eq!(read(&wt, "README.md"), "local edit\n");
    assert_eq!(read(&wt, "scratch.txt"), "untracked\n");
    assert!(!wt.join("base.txt").exists());
    assert!(out.stdout.contains("skipped: dirty"), "{}", out.stdout);
    assert!(out.stdout.contains("--stash-pop"), "{}", out.stdout);
    // Other worktrees are still synced.
    assert_eq!(fx.head(&fx.primary), main_sha);
}

#[test]
fn stash_pop_preserves_changes_and_leaves_other_stashes_alone() {
    let fx = Fixture::new();
    // A pre-existing, unrelated stash entry in the shared stash stack.
    fx.write(&fx.primary, "README.md", "unrelated wip\n");
    fx.git(&fx.primary, &["stash", "push", "-q", "-m", "unrelated"]);
    fx.write(&fx.primary, "README.md", "older wip\n");
    fx.git(&fx.primary, &["stash", "push", "-q", "-m", "older"]);
    let stashes_before = stash_list(&fx);
    assert_eq!(stashes_before.len(), 2);

    let wt = fx.add_worktree("feat/x");
    fx.write(&wt, "README.md", "local edit\n");
    fx.write(&wt, "new/untracked.txt", "untracked\n");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");

    let out = fx.wt_plain(&fx.primary, &["sync", "--stash-pop"]);
    out.assert_success();
    assert_eq!(fx.head(&wt), main_sha);
    assert_eq!(read(&wt, "README.md"), "local edit\n");
    assert_eq!(read(&wt, "new/untracked.txt"), "untracked\n");
    assert_eq!(read(&wt, "base.txt"), "base\n");
    let st = status(&fx, &wt);
    assert!(st.contains(" M README.md"), "{st}");
    assert!(st.contains("?? new/untracked.txt"), "{st}");
    assert_eq!(stash_list(&fx), stashes_before);
    assert!(out.stdout.contains("stash restored"), "{}", out.stdout);
}

#[test]
fn stash_pop_preserves_staged_changes() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    fx.write(&wt, "staged.txt", "staged\n");
    fx.git(&wt, &["add", "staged.txt"]);
    fx.push_upstream("main", "base.txt", "base\n", "base change");
    fx.wt_plain(&fx.primary, &["sync", "--stash-pop"])
        .assert_success();
    let st = status(&fx, &wt);
    assert!(st.contains("A  staged.txt"), "{st}");
    assert!(stash_list(&fx).is_empty());
}

#[test]
fn stash_pop_with_rebase() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    fx.commit(&wt, "feature.txt", "feature\n", "feature work");
    fx.write(&wt, "README.md", "local edit\n");
    fx.write(&wt, "untracked.txt", "untracked\n");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync", "--stash-pop", "--rebase"]);
    out.assert_success();
    let head = fx.head(&wt);
    assert_eq!(parents(&fx, &wt, &head), vec![main_sha]);
    assert_eq!(read(&wt, "README.md"), "local edit\n");
    assert_eq!(read(&wt, "untracked.txt"), "untracked\n");
    assert_eq!(read(&wt, "feature.txt"), "feature\n");
    assert!(stash_list(&fx).is_empty());
}

#[test]
fn merge_conflict_is_aborted_and_other_worktrees_still_sync() {
    let fx = Fixture::new();
    let conflicted = fx.add_worktree("feat/a");
    let local = fx.commit(&conflicted, "README.md", "mine\n", "edit readme");
    fx.write(&conflicted, "wip.txt", "wip\n");
    let clean = fx.add_worktree("feat/b");
    let main_sha = fx.push_upstream("main", "README.md", "theirs\n", "edit readme upstream");

    let out = fx.wt_plain(&fx.primary, &["sync", "--stash-pop"]);
    assert!(!out.success(), "{}", out.stdout);
    assert_eq!(fx.head(&conflicted), local);
    assert!(!git_path_exists(&fx, &conflicted, "MERGE_HEAD"));
    assert_eq!(read(&conflicted, "README.md"), "mine\n");
    assert_eq!(read(&conflicted, "wip.txt"), "wip\n");
    assert_eq!(status(&fx, &conflicted), "?? wip.txt");
    assert!(stash_list(&fx).is_empty());
    assert!(out.stdout.contains("failed: conflict"), "{}", out.stdout);
    assert!(out.stdout.contains("aborted"), "{}", out.stdout);

    assert_eq!(fx.head(&clean), main_sha);
    assert_eq!(fx.head(&fx.primary), main_sha);
}

#[test]
fn rebase_conflict_is_aborted() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/a");
    let local = fx.commit(&wt, "README.md", "mine\n", "edit readme");
    fx.push_upstream("main", "README.md", "theirs\n", "edit readme upstream");
    let out = fx.wt_plain(&fx.primary, &["sync", "--rebase"]);
    assert!(!out.success(), "{}", out.stdout);
    assert_eq!(fx.head(&wt), local);
    assert!(!git_path_exists(&fx, &wt, "rebase-merge"));
    assert!(!git_path_exists(&fx, &wt, "rebase-apply"));
    assert_eq!(fx.git(&wt, &["symbolic-ref", "--short", "HEAD"]), "feat/a");
    assert_eq!(status(&fx, &wt), "");
    assert!(out.stdout.contains("failed: conflict"), "{}", out.stdout);
}

#[test]
fn stash_restore_conflict_keeps_stash_and_reports_it() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    fx.write(&wt, "README.md", "local edit\n");
    let main_sha = fx.push_upstream("main", "README.md", "upstream edit\n", "edit readme");
    let out = fx.wt_plain(&fx.primary, &["sync", "--stash-pop"]);
    assert!(!out.success(), "{}", out.stdout);
    assert_eq!(fx.head(&wt), main_sha);
    let stashes = stash_list(&fx);
    assert_eq!(stashes.len(), 1, "{stashes:?}");
    let sha = stashes[0].split_whitespace().next().expect("sha");
    assert!(out.stdout.contains(sha), "{}", out.stdout);
}

#[test]
fn worktree_with_operation_in_progress_is_skipped() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    fx.commit(&wt, "README.md", "mine\n", "edit readme");
    fx.git(&wt, &["branch", "other", "origin/main"]);
    let other = fx.root.join("other-tmp");
    fx.git(
        &fx.primary,
        &["worktree", "add", "-q", path_str(&other), "other"],
    );
    fx.commit(&other, "README.md", "theirs\n", "conflicting");
    fx.git(&fx.primary, &["worktree", "remove", path_str(&other)]);
    let merge = fx.git_output(&wt, &["merge", "--no-edit", "other"]);
    assert!(!merge.status.success());
    let head = fx.head(&wt);
    fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync", "--stash-pop"]);
    out.assert_success();
    assert_eq!(fx.head(&wt), head);
    assert!(git_path_exists(&fx, &wt, "MERGE_HEAD"));
    assert!(out.stdout.contains("skipped"), "{}", out.stdout);
    assert!(stash_list(&fx).is_empty());
}

#[test]
fn unreachable_remote_falls_back_to_local_base() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    fx.commit(&wt, "feature.txt", "feature\n", "feature work");
    let local_main = fx.commit(&fx.primary, "local.txt", "local\n", "local base change");
    fx.break_remote();
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert!(is_ancestor(&fx, &wt, &local_main, "HEAD"));
    assert_eq!(read(&wt, "local.txt"), "local\n");
    assert_eq!(fx.head(&fx.primary), local_main);
    assert!(
        out.stdout.to_lowercase().contains("local"),
        "{}",
        out.stdout
    );
}

#[test]
fn unreachable_remote_uses_newer_remote_tracking_base() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    // Fetch without updating local main, then go offline.
    fx.git(&fx.primary, &["fetch", "-q", "origin"]);
    fx.break_remote();
    fx.wt_plain(&fx.primary, &["sync"]).assert_success();
    assert_eq!(fx.head(&wt), main_sha);
    assert_eq!(fx.head(&fx.primary), main_sha);
}

#[test]
fn remote_only_with_unreachable_remote_fails_without_changes() {
    let fx = Fixture::new();
    let wt = fx.add_worktree("feat/x");
    let wt_head = fx.head(&wt);
    let local_main = fx.commit(&fx.primary, "local.txt", "local\n", "local base change");
    fx.write(&wt, "README.md", "dirty\n");
    fx.break_remote();
    let out = fx.wt_plain(&fx.primary, &["sync", "--remote-only", "--stash-pop"]);
    assert!(!out.success(), "{}", out.stdout);
    assert!(!out.stderr.is_empty());
    assert_eq!(fx.head(&wt), wt_head);
    assert_eq!(fx.head(&fx.primary), local_main);
    assert_eq!(read(&wt, "README.md"), "dirty\n");
    assert!(stash_list(&fx).is_empty());
}

#[test]
fn detached_worktree_is_skipped() {
    let fx = Fixture::new();
    let det = fx.root.join("detached");
    fx.git(
        &fx.primary,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            path_str(&det),
            "origin/main",
        ],
    );
    let before = fx.head(&det);
    fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert_eq!(fx.head(&det), before);
    assert!(out.stdout.contains("detached"), "{}", out.stdout);
}

#[test]
fn worktree_outside_root_is_not_touched() {
    let fx = Fixture::new();
    let outside = fx.base().join("elsewhere");
    fx.git(
        &fx.primary,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/out",
            path_str(&outside),
            "origin/main",
        ],
    );
    let before = fx.head(&outside);
    let main_sha = fx.push_upstream("main", "base.txt", "base\n", "base change");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert_eq!(fx.head(&outside), before);
    assert!(!out.stdout.contains("feat/out"), "{}", out.stdout);
    assert_eq!(fx.head(&fx.primary), main_sha);
}

#[test]
fn sync_never_pushes() {
    let fx = Fixture::new();
    let wt = tracked_worktree(&fx, "feat/x");
    let pushed = fx.head(&wt);
    fx.commit(&wt, "feature.txt", "feature\n", "unpushed");
    fx.push_upstream("main", "base.txt", "base\n", "base change");
    fx.wt_plain(&fx.primary, &["sync"]).assert_success();
    let remote_head = fx.git(&fx.remote, &["rev-parse", "refs/heads/feat/x"]);
    assert_eq!(remote_head, pushed);
}

#[test]
fn up_to_date_worktrees_report_so() {
    let fx = Fixture::new();
    tracked_worktree(&fx, "feat/x");
    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    assert!(out.stdout.contains("up to date"), "{}", out.stdout);
    assert!(out.stdout.contains("feat/x"), "{}", out.stdout);
}
