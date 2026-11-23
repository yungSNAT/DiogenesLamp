//! `.githooks/commit-msg` installed as its header says, driven by `git commit`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const PASS: &str = "Add parser\n";
const CREDIT: &str = "Add parser\n\nCo-authored-by: Claude <noreply@anthropic.com>\n";

/// A repository with the hook copied into its hooks folder and, when
/// `install` is set, the guard binary placed where `cargo install --root`
/// puts it.
fn setup(name: &str, install: bool) -> (PathBuf, PathBuf) {
    let repo = common::repo(name);
    let hooks = repo.join(".git/hooks");
    fs::create_dir_all(&hooks).unwrap();
    let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.githooks/commit-msg");
    fs::copy(shim, hooks.join("commit-msg")).unwrap();
    executable(&hooks.join("commit-msg"));
    let bin = hooks.join("commit-msg-guard/bin");
    let binary = bin.join(format!("commit-msg-guard{}", std::env::consts::EXE_SUFFIX));
    if install {
        fs::create_dir_all(&bin).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_commit-msg-guard"), &binary).unwrap();
        executable(&binary);
    }
    (repo, binary)
}

#[cfg(unix)]
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn executable(_: &Path) {}

fn commit(repo: &Path, message: &str) -> Output {
    let file = repo.join("message.txt");
    fs::write(&file, message).unwrap();
    common::isolated(std::process::Command::new("git"), repo)
        .args(["commit", "-q", "--allow-empty", "-F"])
        .arg(&file)
        .output()
        .unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn installed_guard_accepts_a_plain_message() {
    let (repo, _) = setup("hook-pass", true);
    let out = commit(&repo, PASS);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn installed_guard_refuses_a_credit() {
    let (repo, _) = setup("hook-refuse", true);
    let out = commit(&repo, CREDIT);
    assert!(!out.status.success());
    assert!(
        stderr(&out)
            .contains("line 3: Co-authored-by: Claude <noreply@anthropic.com> (co-author trailer)"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn missing_guard_fails_closed_and_names_the_install() {
    let (repo, _) = setup("hook-missing", false);
    for message in [PASS, CREDIT] {
        let out = commit(&repo, message);
        assert!(!out.status.success());
        let text = stderr(&out);
        assert!(
            text.contains("commit-msg-guard is not installed; cannot check attribution."),
            "{text}"
        );
        assert!(
            text.contains("Install it as the header of .githooks/commit-msg says"),
            "{text}"
        );
    }
}

#[cfg(unix)]
#[test]
fn guard_without_execute_permission_fails_closed() {
    use std::os::unix::fs::PermissionsExt;
    let (repo, binary) = setup("hook-not-executable", true);
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o644)).unwrap();
    let out = commit(&repo, PASS);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("is not installed"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn hook_runs_nothing_from_the_working_tree() {
    // A branch could carry its own tools/commit-msg or .githooks; the
    // installed hook must ignore both.
    let (repo, _) = setup("hook-tree", false);
    fs::create_dir_all(repo.join(".githooks")).unwrap();
    fs::write(repo.join(".githooks/commit-msg"), "#!/bin/sh\nexit 0\n").unwrap();
    let tree_bin = repo.join("tools/commit-msg/target/release");
    fs::create_dir_all(&tree_bin).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_commit-msg-guard"),
        tree_bin.join(format!("commit-msg-guard{}", std::env::consts::EXE_SUFFIX)),
    )
    .unwrap();
    let out = commit(&repo, PASS);
    assert!(
        !out.status.success(),
        "the hook used a binary from the working tree"
    );
}
