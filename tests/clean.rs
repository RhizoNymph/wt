mod common;

use std::path::{Path, PathBuf};

use common::{Fixture, path_str};

/// Install a fake `gh` that prints `json` for any invocation.
fn fake_gh(fx: &Fixture, json: &str) {
    write_script(
        &fx.gh_path(),
        &format!("#!/bin/sh\ncat <<'WT_EOF'\n{json}\nWT_EOF\n"),
    );
}

/// Install a fake `gh` that fails like an unauthenticated CLI.
fn failing_gh(fx: &Fixture) {
    write_script(
        &fx.gh_path(),
        "#!/bin/sh\necho 'To get started with GitHub CLI, please run:  gh auth login' >&2\nexit 1\n",
    );
}

fn write_script(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    let mut perms = std::fs::metadata(path).expect("meta").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

fn pr_json(branch: &str, number: u64, oid: &str) -> String {
    format!(
        r#"{{"headRefName":"{branch}","number":{number},"url":"https://github.com/o/r/pull/{number}","headRefOid":"{oid}"}}"#
    )
}

fn branch_exists(fx: &Fixture, branch: &str) -> bool {
    fx.git_output(
        &fx.primary,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .status
    .success()
}

/// Create a worktree for `branch` with one commit on it; returns (path, tip).
fn worktree_with_commit(fx: &Fixture, branch: &str) -> (PathBuf, String) {
    let path = fx.add_worktree(branch);
    let file = format!("{}.txt", branch.replace('/', "-"));
    let tip = fx.commit(&path, &file, "work\n", &format!("work on {branch}"));
    (path, tip)
}

/// Merge `branch` into main with a merge commit in the primary checkout and push.
fn merge_no_ff(fx: &Fixture, branch: &str) {
    fx.git(
        &fx.primary,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            &format!("merge {branch}"),
            branch,
        ],
    );
    fx.git(&fx.primary, &["push", "-q", "origin", "main"]);
}

/// Squash-merge `branch` into main (new commit, branch tip not an ancestor) and push.
fn squash_merge(fx: &Fixture, branch: &str) {
    fx.git(&fx.primary, &["merge", "-q", "--squash", branch]);
    fx.git(
        &fx.primary,
        &["commit", "-q", "-m", &format!("squash {branch}")],
    );
    fx.git(&fx.primary, &["push", "-q", "origin", "main"]);
}

fn assert_removed(fx: &Fixture, path: &Path, branch: &str) {
    assert!(!path.exists(), "{} should be removed", path.display());
    assert!(!branch_exists(fx, branch), "{branch} should be deleted");
    let listed = fx.git(&fx.primary, &["worktree", "list", "--porcelain"]);
    assert!(
        !listed.contains(path_str(path)),
        "worktree still registered:\n{listed}"
    );
}

fn assert_kept(fx: &Fixture, path: &Path, branch: &str) {
    assert!(path.exists(), "{} should be kept", path.display());
    assert!(branch_exists(fx, branch), "{branch} should be kept");
}

#[test]
fn merged_via_git_fallback_is_removed_with_empty_parent() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();

    assert_removed(&fx, &path, "feat/x");
    assert!(
        !fx.root.join("feat").exists(),
        "empty feat/ should be removed"
    );
    assert!(fx.root.exists(), "root must never be removed");
    assert!(fx.primary.exists());
    assert!(out.stdout.contains("Removed"), "{}", out.stdout);
    assert!(out.stdout.contains("feat/x"), "{}", out.stdout);
    assert!(out.stdout.contains("origin/main"), "{}", out.stdout);
    assert_eq!(out.cd, None);
}

#[test]
fn nonempty_parent_directory_is_kept() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    let (other, _) = worktree_with_commit(&fx, "feat/y");
    merge_no_ff(&fx, "feat/x");

    fx.wt(&fx.primary, &["clean"]).assert_success();

    assert_removed(&fx, &path, "feat/x");
    assert_kept(&fx, &other, "feat/y");
    assert!(fx.root.join("feat").is_dir());
}

#[test]
fn unmerged_branch_is_kept() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();

    assert_kept(&fx, &path, "feat/x");
    assert!(out.stdout.contains("1 not merged"), "{}", out.stdout);
}

#[test]
fn branch_without_commits_is_not_removed() {
    let fx = Fixture::new();
    let path = fx.add_worktree("feat/empty");

    fx.wt(&fx.primary, &["clean"]).assert_success();
    assert_kept(&fx, &path, "feat/empty");

    // Even after main moves on, the untouched branch is an ancestor of base but was
    // never worked on, so it must still be kept.
    fx.push_upstream("main", "later.txt", "later\n", "later work on main");
    fx.git(&fx.primary, &["pull", "-q", "--ff-only"]);
    fx.wt(&fx.primary, &["clean"]).assert_success();
    assert_kept(&fx, &path, "feat/empty");
}

#[test]
fn fast_forward_merge_without_gh_is_kept() {
    // Documented limitation: a fast-forwarded branch is indistinguishable from an
    // untouched one by ancestry alone, so the git fallback errs on keeping it.
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/ff");
    fx.git(&fx.primary, &["merge", "-q", "--ff-only", "feat/ff"]);
    fx.git(&fx.primary, &["push", "-q", "origin", "main"]);

    fx.wt(&fx.primary, &["clean"]).assert_success();
    assert_kept(&fx, &path, "feat/ff");
}

#[test]
fn squash_merge_detected_via_gh() {
    let fx = Fixture::new();
    let (path, tip) = worktree_with_commit(&fx, "feat/x");
    squash_merge(&fx, "feat/x");
    fake_gh(&fx, &format!("[{}]", pr_json("feat/x", 12, &tip)));

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();

    assert_removed(&fx, &path, "feat/x");
    assert!(out.stdout.contains("PR #12"), "{}", out.stdout);
}

#[test]
fn squash_merge_without_gh_is_kept() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    squash_merge(&fx, "feat/x");

    fx.wt(&fx.primary, &["clean"]).assert_success();
    assert_kept(&fx, &path, "feat/x");
}

#[test]
fn gh_merged_but_local_has_new_commits_is_kept() {
    let fx = Fixture::new();
    let (path, tip) = worktree_with_commit(&fx, "feat/x");
    squash_merge(&fx, "feat/x");
    fx.commit(&path, "after.txt", "more\n", "work after merge");
    fake_gh(&fx, &format!("[{}]", pr_json("feat/x", 12, &tip)));

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();
    assert_kept(&fx, &path, "feat/x");
}

#[test]
fn gh_pr_head_ahead_of_local_counts_as_merged() {
    // Someone pushed more commits to the PR after our last fetch: our tip is an
    // ancestor of the merged head, so we hold no unique work.
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    fx.git(&path, &["push", "-q", "origin", "feat/x"]);
    let head = fx.push_upstream("feat/x", "extra.txt", "extra\n", "reviewer fixup");
    fx.git(&fx.primary, &["fetch", "-q", "origin"]);
    fake_gh(&fx, &format!("[{}]", pr_json("feat/x", 7, &head)));

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();
    assert_removed(&fx, &path, "feat/x");
    assert!(out.stdout.contains("PR #7"), "{}", out.stdout);
}

#[test]
fn gh_pr_for_other_branch_name_is_ignored() {
    let fx = Fixture::new();
    let (path, tip) = worktree_with_commit(&fx, "feat/x");
    squash_merge(&fx, "feat/x");
    fake_gh(&fx, &format!("[{}]", pr_json("feat/other", 3, &tip)));

    fx.wt(&fx.primary, &["clean"]).assert_success();
    assert_kept(&fx, &path, "feat/x");
}

#[test]
fn failing_gh_falls_back_to_git() {
    let fx = Fixture::new();
    let (merged, _) = worktree_with_commit(&fx, "feat/merged");
    let (unmerged, _) = worktree_with_commit(&fx, "feat/open");
    merge_no_ff(&fx, "feat/merged");
    failing_gh(&fx);

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();
    assert_removed(&fx, &merged, "feat/merged");
    assert_kept(&fx, &unmerged, "feat/open");
}

#[test]
fn garbage_gh_output_falls_back_to_git() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");
    fake_gh(&fx, "this is not json");

    fx.wt(&fx.primary, &["clean"]).assert_success();
    assert_removed(&fx, &path, "feat/x");
}

#[test]
fn dirty_merged_worktree_is_skipped_by_default() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");
    fx.write(&path, "README.md", "local edit\n");

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();

    assert_kept(&fx, &path, "feat/x");
    assert!(out.stdout.contains("Skipped (dirty)"), "{}", out.stdout);
    assert!(out.stdout.contains("1 modified"), "{}", out.stdout);
    assert!(out.stdout.contains("0 untracked"), "{}", out.stdout);
}

#[test]
fn untracked_only_merged_worktree_is_skipped_by_default() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");
    fx.write(&path, "notes/a.txt", "a\n");
    fx.write(&path, "notes/b.txt", "b\n");

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();

    assert_kept(&fx, &path, "feat/x");
    assert!(out.stdout.contains("0 modified"), "{}", out.stdout);
    assert!(out.stdout.contains("2 untracked"), "{}", out.stdout);
}

#[test]
fn scratch_mirrors_dirty_files_and_removes_worktree() {
    let fx = Fixture::new();
    let path = fx.add_worktree("feat/x");
    fx.commit(&path, "gone.txt", "tracked\n", "add gone");
    fx.commit(&path, "src/lib.rs", "fn a() {}\n", "add lib");
    merge_no_ff(&fx, "feat/x");
    fx.write(&path, "src/lib.rs", "fn b() {}\n");
    fx.write(&path, "notes/deep/idea.md", "idea\n");
    std::fs::remove_file(path.join("gone.txt")).expect("delete tracked file");

    let out = fx.wt(&fx.primary, &["clean", "--scratch"]);
    out.assert_success();

    assert_removed(&fx, &path, "feat/x");
    let scratch = fx.root.join("scratch").join("feat").join("x");
    assert_eq!(
        std::fs::read_to_string(scratch.join("src/lib.rs")).expect("lib"),
        "fn b() {}\n"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.join("notes/deep/idea.md")).expect("idea"),
        "idea\n"
    );
    assert!(!scratch.join("gone.txt").exists());
    assert!(
        !scratch.join("README.md").exists(),
        "clean files are not copied"
    );
    assert!(out.stdout.contains("scratch"), "{}", out.stdout);
    assert!(out.stdout.contains("2 files"), "{}", out.stdout);
    assert!(out.stdout.contains("gone.txt"), "{}", out.stdout);
}

#[test]
fn scratch_never_overwrites_existing_scratch_dir() {
    let fx = Fixture::new();
    let existing = fx.root.join("scratch").join("feat").join("x");
    fx.write(&existing, "keep.txt", "old\n");

    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");
    fx.write(&path, "keep.txt", "new\n");

    let out = fx.wt(&fx.primary, &["clean", "--scratch"]);
    out.assert_success();

    assert_removed(&fx, &path, "feat/x");
    assert_eq!(
        std::fs::read_to_string(existing.join("keep.txt")).expect("old"),
        "old\n"
    );
    let alt = fx.root.join("scratch").join("feat").join("x-2");
    assert_eq!(
        std::fs::read_to_string(alt.join("keep.txt")).expect("new"),
        "new\n"
    );
}

#[test]
fn clean_merged_worktree_with_scratch_flag_is_removed_without_scratch() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");

    fx.wt(&fx.primary, &["clean", "--scratch"]).assert_success();
    assert_removed(&fx, &path, "feat/x");
    assert!(!fx.root.join("scratch").exists());
}

#[test]
fn delete_dirty_removes_dirty_worktree() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");
    fx.write(&path, "README.md", "local edit\n");
    fx.write(&path, "untracked.txt", "u\n");

    let out = fx.wt(&fx.primary, &["clean", "--delete-dirty"]);
    out.assert_success();
    assert_removed(&fx, &path, "feat/x");
    assert!(!fx.root.join("scratch").exists());
}

#[test]
fn scratch_and_delete_dirty_are_mutually_exclusive() {
    let fx = Fixture::new();
    let out = fx.wt(&fx.primary, &["clean", "--scratch", "--delete-dirty"]);
    assert!(!out.success());
}

#[test]
fn dry_run_changes_nothing() {
    let fx = Fixture::new();
    let (clean, _) = worktree_with_commit(&fx, "feat/clean");
    let (dirty, _) = worktree_with_commit(&fx, "feat/dirty");
    merge_no_ff(&fx, "feat/clean");
    merge_no_ff(&fx, "feat/dirty");
    fx.write(&dirty, "notes.txt", "n\n");

    for flags in [
        &["clean", "--dry-run"][..],
        &["clean", "--dry-run", "--scratch"][..],
        &["clean", "--dry-run", "--delete-dirty"][..],
    ] {
        let out = fx.wt(&clean, flags);
        out.assert_success();
        assert!(out.stdout.contains("feat/clean"), "{}", out.stdout);
        assert!(out.stdout.contains("feat/dirty"), "{}", out.stdout);
        assert!(
            out.stdout.to_lowercase().contains("dry run"),
            "{}",
            out.stdout
        );
        assert_eq!(out.cd, None, "dry run must not move the shell");
    }
    assert_kept(&fx, &clean, "feat/clean");
    assert_kept(&fx, &dirty, "feat/dirty");
    assert!(dirty.join("notes.txt").exists());
    assert!(!fx.root.join("scratch").exists());
}

#[test]
fn primary_and_base_worktrees_are_never_removed() {
    let fx = Fixture::new();
    // Primary moves to a feature branch; main gets its own worktree.
    fx.git(&fx.primary, &["checkout", "-q", "-b", "feat/p"]);
    let main_tip = fx.head(&fx.primary);
    let mainline = fx.root.join("mainline");
    fx.git(
        &fx.primary,
        &["worktree", "add", "-q", path_str(&mainline), "main"],
    );
    // gh claims both are merged; neither may be touched.
    fake_gh(
        &fx,
        &format!(
            "[{},{}]",
            pr_json("feat/p", 1, &main_tip),
            pr_json("main", 2, &main_tip)
        ),
    );

    let out = fx.wt(&fx.primary, &["clean", "--delete-dirty"]);
    out.assert_success();
    assert!(fx.primary.join(".git").exists());
    assert!(branch_exists(&fx, "feat/p"));
    assert_kept(&fx, &mainline, "main");
}

#[test]
fn scratch_dir_worktree_is_ignored() {
    let fx = Fixture::new();
    let inside = fx.root.join("scratch").join("tmp");
    fx.git(
        &fx.primary,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "tmp",
            path_str(&inside),
            "origin/main",
        ],
    );
    fx.commit(&inside, "t.txt", "t\n", "tmp work");
    merge_no_ff(&fx, "tmp");

    fx.wt(&fx.primary, &["clean"]).assert_success();
    assert_kept(&fx, &inside, "tmp");
}

#[test]
fn locked_worktree_is_skipped() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/locked");
    merge_no_ff(&fx, "feat/locked");
    fx.git(&fx.primary, &["worktree", "lock", path_str(&path)]);

    let out = fx.wt(&fx.primary, &["clean", "--delete-dirty"]);
    out.assert_success();
    assert_kept(&fx, &path, "feat/locked");
    assert!(out.stdout.contains("locked"), "{}", out.stdout);
}

#[test]
fn detached_worktree_is_skipped() {
    let fx = Fixture::new();
    let path = fx.root.join("det");
    fx.git(
        &fx.primary,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            path_str(&path),
            "origin/main",
        ],
    );

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();
    assert!(path.exists());
    assert!(out.stdout.contains("detached"), "{}", out.stdout);
}

#[test]
fn missing_worktree_directory_is_pruned() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/gone");
    std::fs::remove_dir_all(&path).expect("remove dir");

    let out = fx.wt(&fx.primary, &["clean"]);
    out.assert_success();
    let listed = fx.git(&fx.primary, &["worktree", "list", "--porcelain"]);
    assert!(!listed.contains(path_str(&path)), "{listed}");
    assert!(out.stdout.contains("Pruned"), "{}", out.stdout);
}

#[test]
fn running_inside_removed_worktree_requests_cd_to_primary() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    merge_no_ff(&fx, "feat/x");
    let sub = path.join("sub");
    std::fs::create_dir_all(&sub).expect("mkdir");

    let out = fx.wt(&sub, &["clean"]);
    out.assert_success();
    assert_removed(&fx, &path, "feat/x");
    assert_eq!(out.cd.as_deref(), Some(fx.primary.as_path()));
}

#[test]
fn unreachable_remote_uses_local_refs() {
    let fx = Fixture::new();
    let (path, _) = worktree_with_commit(&fx, "feat/x");
    // Merged locally only; origin/main never sees it.
    fx.git(
        &fx.primary,
        &["merge", "-q", "--no-ff", "-m", "merge feat/x", "feat/x"],
    );
    fx.break_remote();

    let out = fx.wt(&fx.primary, &["clean"]);
    fx.restore_remote();
    out.assert_success();
    assert_removed(&fx, &path, "feat/x");
    assert!(out.stdout.contains("main"), "{}", out.stdout);
}

#[test]
fn failure_on_one_worktree_does_not_stop_others() {
    let fx = Fixture::new();
    let (bad, _) = worktree_with_commit(&fx, "feat/a-bad");
    let (good, _) = worktree_with_commit(&fx, "feat/b-good");
    merge_no_ff(&fx, "feat/a-bad");
    merge_no_ff(&fx, "feat/b-good");
    // Corrupt the bad worktree's .git link so `git status` fails there.
    std::fs::write(bad.join(".git"), "gitdir: /nonexistent\n").expect("corrupt");

    let out = fx.wt(&fx.primary, &["clean"]);
    assert!(!out.success(), "per-worktree error must fail the run");
    assert!(out.stdout.contains("Errors"), "{}", out.stdout);
    assert!(out.stdout.contains("feat/a-bad"), "{}", out.stdout);
    assert_removed(&fx, &good, "feat/b-good");
    assert!(bad.exists());
}
