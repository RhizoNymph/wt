//! Shell integration: wrapper functions and the cd directive channel.
//!
//! A child process cannot change its parent shell's directory, so `wt init <shell>`
//! prints a `wt` function that points `WT_CD_FILE` at a temp file, runs the binary,
//! and `cd`s into whatever path the binary wrote there.

use std::path::Path;

use clap::ValueEnum;

pub const CD_FILE_ENV: &str = "WT_CD_FILE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

const POSIX_WRAPPER: &str = r#"wt() {
  local __wt_cd __wt_status
  __wt_cd="$(mktemp "${TMPDIR:-/tmp}/wt-cd.XXXXXX")" || return 1
  WT_CD_FILE="$__wt_cd" command wt "$@"
  __wt_status=$?
  if [ -s "$__wt_cd" ]; then
    cd -- "$(cat -- "$__wt_cd")" || __wt_status=$?
  fi
  rm -f -- "$__wt_cd"
  return $__wt_status
}
"#;

const FISH_WRAPPER: &str = r#"function wt
    set -l __wt_cd (mktemp (set -q TMPDIR; and echo $TMPDIR; or echo /tmp)/wt-cd.XXXXXX); or return 1
    WT_CD_FILE=$__wt_cd command wt $argv
    set -l __wt_status $status
    if test -s $__wt_cd
        cd (cat $__wt_cd); or set __wt_status $status
    end
    rm -f $__wt_cd
    return $__wt_status
end
"#;

pub fn init_script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash | Shell::Zsh => POSIX_WRAPPER,
        Shell::Fish => FISH_WRAPPER,
    }
}

/// How a cd request was delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdDelivery {
    /// Written to `WT_CD_FILE`; the shell wrapper will change directory.
    Wrapper,
    /// No wrapper active; the path was printed to stdout for `cd "$(wt ...)"`.
    Stdout,
}

/// Ask the invoking shell to change directory to `path`.
pub fn request_cd(path: &Path) -> std::io::Result<CdDelivery> {
    match std::env::var_os(CD_FILE_ENV) {
        Some(file) if !file.is_empty() => {
            std::fs::write(file, path.as_os_str().as_encoded_bytes())?;
            Ok(CdDelivery::Wrapper)
        }
        _ => {
            println!("{}", path.display());
            Ok(CdDelivery::Stdout)
        }
    }
}
