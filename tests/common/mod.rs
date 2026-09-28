//! Hermetic git fixture for integration tests.
//!
//! Layout inside a temp dir:
//!   remote.git/       bare "origin" (default branch `main`)
//!   repo/main/        primary checkout (clone of origin); worktree root is `repo/`
//!   other/            a second clone, for pushing "someone else's" changes upstream
//!   bin/              put fake executables here (e.g. `bin/gh`, see `gh_path`)
//! Every git/wt invocation gets an isolated HOME, global git config and wt config.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

pub struct Fixture {
    pub tmp: TempDir,
    pub remote: PathBuf,
    pub root: PathBuf,
    pub primary: PathBuf,
    pub other: PathBuf,
}

pub struct WtOutput {
    pub output: Output,
    pub stdout: String,
    pub stderr: String,
    /// Directory the shell wrapper would `cd` into, if any.
    pub cd: Option<PathBuf>,
}

impl WtOutput {
    pub fn success(&self) -> bool {
        self.output.status.success()
    }

    #[track_caller]
    pub fn assert_success(&self) -> &Self {
        assert!(
            self.success(),
            "wt failed ({:?})\nstdout:\n{}\nstderr:\n{}",
            self.output.status,
            self.stdout,
            self.stderr
        );
        self
    }
}

impl Fixture {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let base = tmp.path().canonicalize().expect("canonicalize tempdir");
        for dir in ["home", "xdg", "bin", "repo"] {
            std::fs::create_dir_all(base.join(dir)).expect("create fixture dir");
        }
        std::fs::write(
            base.join("gitconfig"),
            "[user]\n\tname = Test\n\temail = test@example.com\n\
             [init]\n\tdefaultBranch = main\n\
             [commit]\n\tgpgsign = false\n\
             [advice]\n\tdetachedHead = false\n",
        )
        .expect("write gitconfig");
        let fx = Self {
            remote: base.join("remote.git"),
            root: base.join("repo"),
            primary: base.join("repo").join("main"),
            other: base.join("other"),
            tmp,
        };
        fx.git(&base, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        let seed = base.join("seed");
        fx.git(&base, &["clone", "-q", path_str(&fx.remote), "seed"]);
        fx.commit(&seed, "README.md", "hello\n", "initial");
        fx.git(&seed, &["push", "-q", "origin", "main"]);
        fx.git(&fx.root, &["clone", "-q", path_str(&fx.remote), "main"]);
        fx.git(&base, &["clone", "-q", path_str(&fx.remote), "other"]);
        fx
    }

    pub fn base(&self) -> &Path {
        self.tmp.path()
    }

    /// Global wt config file used by every invocation (absent until `write_config`).
    pub fn config_path(&self) -> PathBuf {
        self.base().join("wt-config.json")
    }

    pub fn write_config(&self, json: &str) {
        std::fs::write(self.config_path(), json).expect("write wt config");
    }

    /// Path `WT_GH` points at; write an executable script here to fake the GitHub CLI.
    /// Absent by default, so gh is treated as unavailable.
    pub fn gh_path(&self) -> PathBuf {
        self.base().join("bin").join("gh")
    }

    fn apply_env(&self, cmd: &mut Command) {
        let base = self.base();
        cmd.env("HOME", base.join("home"))
            .env("XDG_CONFIG_HOME", base.join("xdg"))
            .env("GIT_CONFIG_GLOBAL", base.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("WT_CONFIG", self.config_path())
            .env("WT_GH", self.gh_path())
            .env_remove("WT_CD_FILE")
            .env_remove("WT_LOG")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");
    }

    pub fn git_output(&self, dir: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new("git");
        cmd.current_dir(dir).args(args);
        self.apply_env(&mut cmd);
        cmd.output().expect("spawn git")
    }

    /// Run git, panicking with stderr on failure; returns trimmed stdout.
    #[track_caller]
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = self.git_output(dir, args);
        assert!(
            out.status.success(),
            "git {args:?} in {} failed:\n{}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// Write `file` (creating parents), stage everything, commit; returns the new HEAD sha.
    #[track_caller]
    pub fn commit(&self, dir: &Path, file: &str, contents: &str, msg: &str) -> String {
        self.write(dir, file, contents);
        self.git(dir, &["add", "-A"]);
        self.git(dir, &["commit", "-q", "-m", msg]);
        self.git(dir, &["rev-parse", "HEAD"])
    }

    pub fn write(&self, dir: &Path, file: &str, contents: &str) {
        let path = dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(path, contents).expect("write file");
    }

    pub fn head(&self, dir: &Path) -> String {
        self.git(dir, &["rev-parse", "HEAD"])
    }

    /// Push a commit to `branch` on origin from the `other` clone.
    #[track_caller]
    pub fn push_upstream(&self, branch: &str, file: &str, contents: &str, msg: &str) -> String {
        self.git(&self.other, &["fetch", "-q", "origin"]);
        let exists = self
            .git_output(
                &self.other,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("origin/{branch}"),
                ],
            )
            .status
            .success();
        if exists {
            self.git(
                &self.other,
                &["checkout", "-q", "-B", branch, &format!("origin/{branch}")],
            );
        } else {
            self.git(
                &self.other,
                &["checkout", "-q", "-B", branch, "origin/main"],
            );
        }
        let sha = self.commit(&self.other, file, contents, msg);
        self.git(
            &self.other,
            &["push", "-q", "origin", &format!("{branch}:{branch}")],
        );
        sha
    }

    /// Make origin unreachable (fetch/push fail) until `restore_remote`.
    pub fn break_remote(&self) {
        std::fs::rename(&self.remote, self.remote.with_extension("offline")).expect("hide remote");
    }

    pub fn restore_remote(&self) {
        std::fs::rename(self.remote.with_extension("offline"), &self.remote)
            .expect("restore remote");
    }

    /// Run the wt binary as if through the shell wrapper (WT_CD_FILE set).
    pub fn wt(&self, cwd: &Path, args: &[&str]) -> WtOutput {
        let cd_file = self.base().join(format!("cd-{}", std::process::id()));
        let _ = std::fs::remove_file(&cd_file);
        std::fs::write(&cd_file, "").expect("create cd file");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_wt"));
        cmd.current_dir(cwd).args(args);
        self.apply_env(&mut cmd);
        cmd.env("WT_CD_FILE", &cd_file);
        let output = cmd.output().expect("spawn wt");
        let cd = std::fs::read_to_string(&cd_file)
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let _ = std::fs::remove_file(&cd_file);
        WtOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            output,
            cd,
        }
    }

    /// Run the wt binary without the shell wrapper.
    pub fn wt_plain(&self, cwd: &Path, args: &[&str]) -> WtOutput {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_wt"));
        cmd.current_dir(cwd).args(args);
        self.apply_env(&mut cmd);
        let output = cmd.output().expect("spawn wt");
        WtOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            output,
            cd: None,
        }
    }

    /// Create a worktree for a new branch directly with git (bypassing `wt checkout`).
    #[track_caller]
    pub fn add_worktree(&self, branch: &str) -> PathBuf {
        let path = self.root.join(branch);
        self.git(
            &self.primary,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                path_str(&path),
                "origin/main",
            ],
        );
        path
    }
}

pub fn path_str(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}
