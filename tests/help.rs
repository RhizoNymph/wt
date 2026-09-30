mod common;

use common::Fixture;

/// stdout and stderr together: clap prints help-on-missing-args to stderr.
fn all_output(out: &common::WtOutput) -> String {
    format!("{}{}", out.stdout, out.stderr)
}

#[test]
fn bare_wt_prints_overview_with_setup_and_examples() {
    let fx = Fixture::new();
    let text = all_output(&fx.wt_plain(fx.base(), &[]));
    for needle in [
        "Usage: wt",
        "checkout",
        "clean",
        "sync",
        "init",
        "eval \"$(wt init bash)\"",
        "Examples:",
        "wt checkout feat/login",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
    assert!(!text.contains("error:"), "{text}");
}

#[test]
fn checkout_without_branch_prints_its_help_not_an_error() {
    let fx = Fixture::new();
    let text = all_output(&fx.wt_plain(&fx.primary, &["checkout"]));
    assert!(!text.contains("error:"), "{text}");
    for needle in ["Usage: wt checkout", "<BRANCH>", "--from", "Examples:"] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

#[test]
fn checkout_without_branch_does_not_touch_the_repo() {
    let fx = Fixture::new();
    let before = fx.git(&fx.primary, &["worktree", "list", "--porcelain"]);
    let out = fx.wt(&fx.primary, &["checkout"]);
    assert!(!out.success());
    assert!(out.cd.is_none());
    assert_eq!(
        fx.git(&fx.primary, &["worktree", "list", "--porcelain"]),
        before
    );
}

#[test]
fn init_without_shell_prints_its_help_not_an_error() {
    let fx = Fixture::new();
    let text = all_output(&fx.wt_plain(fx.base(), &["init"]));
    assert!(!text.contains("error:"), "{text}");
    for needle in ["Usage: wt init", "bash", "zsh", "fish", "~/.bashrc"] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

#[test]
fn subcommand_help_explains_behavior_and_flags() {
    let fx = Fixture::new();
    let cases: &[(&str, &[&str])] = &[
        (
            "checkout",
            &["already exists", "<root>/feat/login", "--from", "Examples:"],
        ),
        (
            "clean",
            &[
                "merged",
                "--scratch",
                "--delete-dirty",
                "--dry-run",
                "gitignored",
                "Examples:",
            ],
        ),
        (
            "sync",
            &[
                "upstream",
                "base branch",
                "--stash-pop",
                "--rebase",
                "--remote-only",
                "Examples:",
            ],
        ),
    ];
    for (cmd, needles) in cases {
        let out = fx.wt_plain(fx.base(), &[cmd, "--help"]);
        out.assert_success();
        for needle in *needles {
            assert!(
                out.stdout.contains(needle),
                "`wt {cmd} --help` missing {needle:?}:\n{}",
                out.stdout
            );
        }
    }
}

#[test]
fn top_level_help_mentions_configuration() {
    let fx = Fixture::new();
    let out = fx.wt_plain(fx.base(), &["--help"]);
    out.assert_success();
    for needle in ["worktree_dir", ".wt.json", "config.json"] {
        assert!(
            out.stdout.contains(needle),
            "missing {needle:?}:\n{}",
            out.stdout
        );
    }
}
