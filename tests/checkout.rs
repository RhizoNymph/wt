mod common;

use std::path::{Path, PathBuf};

use common::{Fixture, path_str};

/// Branch checked out in `dir`, via git.
fn branch_at(fx: &Fixture, dir: &Path) -> String {
    fx.git(dir, &["symbolic-ref", "--short", "HEAD"])
}

fn upstream_of(fx: &Fixture, dir: &Path, branch: &str) -> Option<String> {
    let out = fx.git_output(
        dir,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            &format!("{branch}@{{upstream}}"),
        ],
    );
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn worktree_paths(fx: &Fixture) -> Vec<PathBuf> {
    fx.git(&fx.primary, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

#[test]
fn new_branch_is_created_from_remote_base_and_cds_there() {
    let fx = Fixture::new();
    // Advance origin/main so we can tell origin/main apart from a stale local main.
    let upstream = fx.push_upstream("main", "up.txt", "up\n", "upstream change");
    let out = fx.wt(&fx.primary, &["checkout", "feat/x"]);
    out.assert_success();
    let path = fx.root.join("feat").join("x");
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert!(path.join("README.md").exists());
    assert_eq!(branch_at(&fx, &path), "feat/x");
    assert_eq!(
        fx.head(&path),
        upstream,
        "starts at freshly fetched origin/main"
    );
    assert_eq!(
        upstream_of(&fx, &path, "feat/x"),
        None,
        "new branch has no upstream"
    );
    assert!(
        out.stderr.contains("Created worktree for feat/x"),
        "{}",
        out.stderr
    );
    assert!(out.stdout.is_empty(), "stdout: {}", out.stdout);
}

#[test]
fn nested_slashes_create_subdirectories() {
    let fx = Fixture::new();
    let out = fx.wt(&fx.primary, &["checkout", "fix/deep/nested-name"]);
    out.assert_success();
    let path = fx.root.join("fix").join("deep").join("nested-name");
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert_eq!(branch_at(&fx, &path), "fix/deep/nested-name");
}

#[test]
fn second_checkout_reuses_existing_worktree() {
    let fx = Fixture::new();
    let first = fx.wt(&fx.primary, &["checkout", "feat/x"]);
    first.assert_success();
    let before = worktree_paths(&fx);
    let path = fx.root.join("feat").join("x");
    fx.write(&path, "wip.txt", "uncommitted\n");

    let second = fx.wt(&fx.primary, &["checkout", "feat/x"]);
    second.assert_success();
    assert_eq!(second.cd.as_deref(), Some(path.as_path()));
    assert!(
        second.stderr.contains("already exists"),
        "{}",
        second.stderr
    );
    assert_eq!(worktree_paths(&fx), before);
    assert!(
        path.join("wip.txt").exists(),
        "existing worktree left untouched"
    );
}

#[test]
fn checkout_of_primary_branch_goes_to_primary() {
    let fx = Fixture::new();
    let out = fx.wt(&fx.primary, &["checkout", "main"]);
    out.assert_success();
    assert_eq!(out.cd.as_deref(), Some(fx.primary.as_path()));
    assert!(out.stderr.contains("already exists"), "{}", out.stderr);
    assert!(!fx.root.join("main").join("main").exists());
}

#[test]
fn remote_only_branch_is_created_tracking_remote() {
    let fx = Fixture::new();
    let sha = fx.push_upstream("feat/r", "r.txt", "remote\n", "remote work");
    let out = fx.wt(&fx.primary, &["checkout", "feat/r"]);
    out.assert_success();
    let path = fx.root.join("feat").join("r");
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert_eq!(fx.head(&path), sha);
    assert_eq!(
        upstream_of(&fx, &path, "feat/r").as_deref(),
        Some("origin/feat/r")
    );
    assert!(
        out.stderr.contains("tracking origin/feat/r"),
        "{}",
        out.stderr
    );
}

#[test]
fn existing_local_branch_is_checked_out_not_reset() {
    let fx = Fixture::new();
    fx.git(&fx.primary, &["branch", "feat/local"]);
    fx.git(&fx.primary, &["checkout", "-q", "feat/local"]);
    let sha = fx.commit(&fx.primary, "local.txt", "local\n", "local work");
    fx.git(&fx.primary, &["checkout", "-q", "main"]);
    // Make origin/main differ so a reset would be observable.
    fx.push_upstream("main", "up.txt", "up\n", "upstream change");

    let out = fx.wt(&fx.primary, &["checkout", "feat/local"]);
    out.assert_success();
    let path = fx.root.join("feat").join("local");
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert_eq!(fx.head(&path), sha);
    assert!(path.join("local.txt").exists());
}

#[test]
fn from_flag_sets_start_point() {
    let fx = Fixture::new();
    let first = fx.head(&fx.primary);
    fx.commit(&fx.primary, "second.txt", "2\n", "second");
    let out = fx.wt(&fx.primary, &["checkout", "feat/from", "--from", &first]);
    out.assert_success();
    let path = fx.root.join("feat").join("from");
    assert_eq!(fx.head(&path), first);
    assert_eq!(upstream_of(&fx, &path, "feat/from"), None);
}

#[test]
fn unknown_from_rev_is_rejected() {
    let fx = Fixture::new();
    let out = fx.wt(
        &fx.primary,
        &["checkout", "feat/bad", "--from", "no-such-rev"],
    );
    assert!(!out.success());
    assert!(out.stderr.contains("no-such-rev"), "{}", out.stderr);
    assert!(!fx.root.join("feat").join("bad").exists());
    assert_eq!(out.cd, None);
}

#[test]
fn from_with_local_base_branch() {
    let fx = Fixture::new();
    let out = fx.wt(&fx.primary, &["checkout", "feat/lb", "--from", "main"]);
    out.assert_success();
    let path = fx.root.join("feat").join("lb");
    assert_eq!(fx.head(&path), fx.head(&fx.primary));
    assert_eq!(upstream_of(&fx, &path, "feat/lb"), None, "--no-track");
}

#[test]
fn unreachable_remote_falls_back_to_local_refs() {
    let fx = Fixture::new();
    let base = fx.head(&fx.primary);
    fx.break_remote();
    let out = fx.wt(&fx.primary, &["checkout", "feat/offline"]);
    fx.restore_remote();
    out.assert_success();
    let path = fx.root.join("feat").join("offline");
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert_eq!(fx.head(&path), base);
    assert!(out.stderr.to_lowercase().contains("warn"), "{}", out.stderr);
}

#[test]
fn unreachable_remote_still_uses_known_remote_tracking_branch() {
    let fx = Fixture::new();
    let sha = fx.push_upstream("feat/r", "r.txt", "remote\n", "remote work");
    fx.git(&fx.primary, &["fetch", "-q", "origin"]);
    fx.break_remote();
    let out = fx.wt(&fx.primary, &["checkout", "feat/r"]);
    fx.restore_remote();
    out.assert_success();
    let path = fx.root.join("feat").join("r");
    assert_eq!(fx.head(&path), sha);
    assert_eq!(
        upstream_of(&fx, &path, "feat/r").as_deref(),
        Some("origin/feat/r")
    );
}

#[test]
fn scratch_prefix_is_rejected() {
    let fx = Fixture::new();
    let out = fx.wt(&fx.primary, &["checkout", "scratch/x"]);
    assert!(!out.success());
    assert!(out.stderr.contains("reserved"), "{}", out.stderr);
    assert!(!fx.root.join("scratch").exists());
    assert_eq!(out.cd, None);
    let branches = fx.git(&fx.primary, &["branch", "--list", "scratch/x"]);
    assert!(branches.is_empty(), "no branch created: {branches}");
}

#[test]
fn occupied_non_worktree_path_is_rejected() {
    let fx = Fixture::new();
    let path = fx.root.join("feat").join("busy");
    fx.write(&path, "stray.txt", "do not clobber\n");
    let out = fx.wt(&fx.primary, &["checkout", "feat/busy"]);
    assert!(!out.success());
    assert!(out.stderr.contains(path_str(&path)), "{}", out.stderr);
    assert_eq!(out.cd, None);
    assert_eq!(
        std::fs::read_to_string(path.join("stray.txt")).expect("stray file kept"),
        "do not clobber\n"
    );
    let branches = fx.git(&fx.primary, &["branch", "--list", "feat/busy"]);
    assert!(branches.is_empty(), "no branch created: {branches}");
}

#[test]
fn path_occupied_by_other_worktree_is_rejected() {
    let fx = Fixture::new();
    let path = fx.root.join("feat").join("taken");
    fx.git(
        &fx.primary,
        &["worktree", "add", "-q", "--detach", path_str(&path), "HEAD"],
    );
    let out = fx.wt(&fx.primary, &["checkout", "feat/taken"]);
    assert!(!out.success());
    assert!(out.stderr.contains(path_str(&path)), "{}", out.stderr);
    assert_eq!(out.cd, None);
}

#[test]
fn empty_directory_at_path_is_reused() {
    let fx = Fixture::new();
    let path = fx.root.join("feat").join("empty");
    std::fs::create_dir_all(&path).expect("mkdir");
    let out = fx.wt(&fx.primary, &["checkout", "feat/empty"]);
    out.assert_success();
    assert_eq!(branch_at(&fx, &path), "feat/empty");
}

#[test]
fn works_from_inside_another_worktree() {
    let fx = Fixture::new();
    let other = fx.add_worktree("feat/other");
    let nested = other.join("sub");
    std::fs::create_dir_all(&nested).expect("mkdir");
    let out = fx.wt(&nested, &["checkout", "feat/y"]);
    out.assert_success();
    let path = fx.root.join("feat").join("y");
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert_eq!(branch_at(&fx, &path), "feat/y");

    // And switching back to an existing one from there.
    let back = fx.wt(&path, &["checkout", "feat/other"]);
    back.assert_success();
    assert_eq!(back.cd.as_deref(), Some(other.as_path()));
}

#[test]
fn without_wrapper_stdout_is_exactly_the_path() {
    let fx = Fixture::new();
    let out = fx.wt_plain(&fx.primary, &["checkout", "feat/plain"]);
    out.assert_success();
    let path = fx.root.join("feat").join("plain");
    assert_eq!(out.stdout, format!("{}\n", path.display()));

    let again = fx.wt_plain(&fx.primary, &["checkout", "feat/plain"]);
    again.assert_success();
    assert_eq!(again.stdout, format!("{}\n", path.display()));
}

#[test]
fn configured_worktree_dir_is_respected() {
    let fx = Fixture::new();
    fx.write_config(r#"{"worktree_dir": "../wts"}"#);
    let out = fx.wt(&fx.primary, &["checkout", "feat/cfg"]);
    out.assert_success();
    let path = fx.root.join("wts").join("feat").join("cfg");
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert_eq!(branch_at(&fx, &path), "feat/cfg");
}

#[test]
fn stale_worktree_registration_is_pruned_and_recreated() {
    let fx = Fixture::new();
    let path = fx.add_worktree("feat/stale");
    std::fs::remove_dir_all(&path).expect("delete worktree dir");
    let out = fx.wt(&fx.primary, &["checkout", "feat/stale"]);
    out.assert_success();
    assert_eq!(out.cd.as_deref(), Some(path.as_path()));
    assert_eq!(branch_at(&fx, &path), "feat/stale");
}
