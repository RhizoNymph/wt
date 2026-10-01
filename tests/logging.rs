mod common;

use common::Fixture;

#[test]
fn warnings_are_reported_once_without_log_noise() {
    let fx = Fixture::new();
    fx.break_remote();
    for cmd in [["clean"].as_slice(), ["sync"].as_slice()] {
        let out = fx.wt_plain(&fx.primary, cmd);
        out.assert_success();
        assert!(
            out.stdout.contains("local refs"),
            "report should mention the fallback:\n{}",
            out.stdout
        );
        assert!(
            !out.stderr.contains("WARN"),
            "`wt {cmd:?}` stderr should not repeat warnings as logs:\n{}",
            out.stderr
        );
        assert!(
            !out.stderr.contains('\u{1b}'),
            "no ANSI escapes when stderr is not a terminal:\n{}",
            out.stderr
        );
    }
}

#[test]
fn verbose_flag_shows_logs() {
    let fx = Fixture::new();
    fx.break_remote();
    let out = fx.wt_plain(&fx.primary, &["-v", "sync"]);
    out.assert_success();
    assert!(out.stderr.contains("fetch failed"), "{}", out.stderr);
    assert!(!out.stderr.contains('\u{1b}'), "{}", out.stderr);
}
