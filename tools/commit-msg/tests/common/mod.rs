#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub const DATE: &str = "1700000000 +0000";

/// A fresh, empty Git repository under this test target's scratch folder.
pub fn repo(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ok = isolated(Command::new("git"), &dir)
        .args(["init", "-q", "."])
        .status()
        .unwrap()
        .success();
    assert!(ok, "git init");
    dir
}

/// Run in `dir` with no inherited Git state or user and system config, and a
/// fixed human identity.
pub fn isolated(mut command: Command, dir: &Path) -> Command {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    let null = if cfg!(windows) { "nul" } else { "/dev/null" };
    command
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_AUTHOR_NAME", "Jane Doe")
        .env("GIT_AUTHOR_EMAIL", "jane@example.com")
        .env("GIT_AUTHOR_DATE", DATE)
        .env("GIT_COMMITTER_NAME", "Jane Doe")
        .env("GIT_COMMITTER_EMAIL", "jane@example.com")
        .env("GIT_COMMITTER_DATE", DATE)
        .env("GIT_CONFIG_COUNT", "0");
    command
}

pub fn guard(dir: &Path) -> Command {
    isolated(Command::new(env!("CARGO_BIN_EXE_commit-msg-guard")), dir)
}

/// Git config entries passed through the environment, as `git -c` does.
pub fn config(command: &mut Command, pairs: &[(String, String)]) {
    command.env("GIT_CONFIG_COUNT", pairs.len().to_string());
    for (i, (key, value)) in pairs.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{i}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{i}"), value);
    }
}
