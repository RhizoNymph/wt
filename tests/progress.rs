//! Progress output. Tests run without a terminal, so progress is printed as plain
//! lines on stderr (the bar is only drawn when stderr is a TTY).
mod common;

use common::Fixture;

#[test]
fn sync_reports_each_step_and_worktree_with_counts() {
    let fx = Fixture::new();
    fx.add_worktree("feat/a");
    fx.add_worktree("feat/b");

    let out = fx.wt_plain(&fx.primary, &["sync"]);
    out.assert_success();
    for needle in [
        "fetching origin",
        "[1/3] main",
        "[2/3] feat/a",
        "[3/3] feat/b",
    ] {
        assert!(
            out.stderr.contains(needle),
            "missing {needle:?}:\n{}",
            out.stderr
        );
    }
    assert!(
        !out.stdout.contains("[1/3]"),
        "progress must stay off stdout"
    );
    assert!(!out.stderr.contains('\u{1b}'), "{}", out.stderr);
}

#[test]
fn clean_reports_phases_and_worktrees() {
    let fx = Fixture::new();
    fx.add_worktree("feat/a");

    let out = fx.wt_plain(&fx.primary, &["clean"]);
    out.assert_success();
    for needle in [
        "fetching origin",
        "checking merged pull requests",
        "[1/1] feat/a",
    ] {
        assert!(
            out.stderr.contains(needle),
            "missing {needle:?}:\n{}",
            out.stderr
        );
    }
    assert!(!out.stdout.contains("[1/1]"));
}

#[test]
fn checkout_reports_fetch_and_creation() {
    let fx = Fixture::new();
    let out = fx.wt_plain(&fx.primary, &["checkout", "feat/x"]);
    out.assert_success();
    for needle in ["fetching feat/x from origin", "creating worktree"] {
        assert!(
            out.stderr.contains(needle),
            "missing {needle:?}:\n{}",
            out.stderr
        );
    }
    assert_eq!(
        out.stdout.trim(),
        fx.root.join("feat").join("x").display().to_string(),
        "stdout must stay just the path"
    );
}

#[test]
fn quiet_suppresses_progress_but_not_reports() {
    let fx = Fixture::new();
    fx.add_worktree("feat/a");

    let out = fx.wt_plain(&fx.primary, &["-q", "sync"]);
    out.assert_success();
    assert!(!out.stderr.contains("fetching"), "{}", out.stderr);
    assert!(!out.stderr.contains("[1/2]"), "{}", out.stderr);
    assert!(
        out.stdout.contains("feat/a"),
        "report still printed:\n{}",
        out.stdout
    );

    let out = fx.wt_plain(&fx.primary, &["checkout", "--quiet", "feat/y"]);
    out.assert_success();
    assert!(!out.stderr.contains("fetching"), "{}", out.stderr);
    assert!(out.stderr.contains("Created worktree"), "{}", out.stderr);
}

#[test]
fn quiet_and_verbose_conflict() {
    let fx = Fixture::new();
    let out = fx.wt_plain(&fx.primary, &["-q", "-v", "sync"]);
    assert!(!out.success());
    assert!(out.stderr.contains("cannot be used with"), "{}", out.stderr);
}
