mod common;

use common::Fixture;

#[test]
fn init_prints_wrapper_for_each_shell() {
    let fx = Fixture::new();
    for shell in ["bash", "zsh", "fish"] {
        let out = fx.wt_plain(fx.base(), &["init", shell]);
        out.assert_success();
        assert!(out.stdout.contains("WT_CD_FILE"), "{shell}: {}", out.stdout);
    }
}

#[test]
fn outside_repo_is_an_error() {
    let fx = Fixture::new();
    let out = fx.wt_plain(fx.base(), &["sync"]);
    assert!(!out.success());
    assert!(
        out.stderr.contains("not inside a git repository"),
        "{}",
        out.stderr
    );
}

#[test]
fn bash_wrapper_changes_directory() {
    let fx = Fixture::new();
    let script = fx.wt_plain(fx.base(), &["init", "bash"]).stdout;
    // Stand-in binary that just requests a cd, to exercise the wrapper itself.
    let fake = fx.base().join("bin").join("wt");
    let target = fx.base().join("target-dir");
    std::fs::create_dir_all(&target).expect("mkdir");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s' '{}' > \"$WT_CD_FILE\"\n",
            target.display()
        ),
    )
    .expect("write fake wt");
    let mut perms = std::fs::metadata(&fake).expect("meta").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&fake, perms).expect("chmod");
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(format!("{script}\nwt checkout x && pwd"))
        .env(
            "PATH",
            format!(
                "{}:{}",
                fake.parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .expect("run bash");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        target.display().to_string()
    );
}
