//! Refuse AI authorship credit before a commit is written.
//!
//! `.githooks/commit-msg` runs this binary from the hooks directory, where the
//! install step in CONTRIBUTING.md puts it; it never runs from the working
//! tree. Usage: `commit-msg-guard <message file>`, or `--version`.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

mod check;
// lamp's Python 3.15 (Unicode 17) str tables, shared so both agree with Python.
#[allow(dead_code)]
#[path = "../../../src/pytables.rs"]
mod pytables;
mod tables;

/// The source commit, set by the install step; `unknown` otherwise.
const SOURCE: Option<&str> = option_env!("COMMIT_MSG_GUARD_SOURCE");

/// stdout of a git command with trailing newlines removed, as the shell's
/// `$(...)` gives it; empty when git fails or is missing.
fn git(args: &[&str], quiet: bool) -> String {
    let mut command = Command::new("git");
    command.args(args).stdin(Stdio::null());
    // git's own complaint (an empty identity, say) reaches the user first.
    command.stderr(if quiet {
        Stdio::null()
    } else {
        Stdio::inherit()
    });
    let out = command.output().map(|o| o.stdout).unwrap_or_default();
    String::from_utf8_lossy(&out)
        .trim_end_matches('\n')
        .to_owned()
}

/// Warn when the rules on the remote default branch differ from the source
/// this guard was built from. It runs git only, never code from the tree, and
/// stays silent when the source or the remote branch is unknown.
fn warn_if_stale() {
    let Some(source) = SOURCE else { return };
    if let Some(remote) = stale_against(Path::new("."), source) {
        eprintln!(
            "commit-msg: warning: the installed guard (source {source}) differs from the rules on {remote}; reinstall it as the header of .githooks/commit-msg says."
        );
    }
}

/// The remote default branch (`origin/HEAD`, else `origin/main` for a remote
/// added by hand) when its rule files differ from `source`.
fn stale_against(dir: &Path, source: &str) -> Option<&'static str> {
    let run = |args: &[&str]| {
        Command::new("git")
            .current_dir(dir)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .and_then(|s| s.code())
    };
    let remote = ["origin/HEAD", "origin/main"].into_iter().find(|r| {
        run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{r}^{{commit}}"),
        ]) == Some(0)
    })?;
    // diff-tree is plumbing: no diff driver or user diff setting applies.
    let differs = [
        &["diff-tree", "--quiet", source, remote, "--"][..],
        &RULE_FILES,
    ]
    .concat();
    (run(&differs) == Some(1)).then_some(remote)
}

/// The files that decide what the guard refuses.
const RULE_FILES: [&str; 3] = [
    "tools/commit-msg",
    "src/pytables.rs",
    ".githooks/commit-msg",
];

fn main() -> ExitCode {
    // A check that cannot finish panics (exit 101), so Git refuses the commit.
    std::panic::set_hook(Box::new(|info| {
        eprintln!(
            "commit-msg: {}",
            info.payload_as_str().unwrap_or("internal error")
        );
    }));
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let path = match args.as_slice() {
        [flag] if flag == "--version" => {
            println!(
                "commit-msg-guard {} (source {})",
                env!("CARGO_PKG_VERSION"),
                SOURCE.unwrap_or("unknown")
            );
            return ExitCode::SUCCESS;
        }
        // The message path may not be UTF-8 (a worktree under such a folder).
        [path] => PathBuf::from(path),
        _ => {
            eprintln!("usage: commit-msg-guard <message file> | --version");
            return ExitCode::from(2);
        }
    };
    let message = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("commit-msg: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    // Each configured comment value is passed through rather than resolved:
    // the scan treats every one as a marker instead of picking between them.
    let author = git(&["var", "GIT_AUTHOR_IDENT"], false);
    let committer = git(&["var", "GIT_COMMITTER_IDENT"], false);
    let comment_string = git(&["config", "--get", "core.commentString"], true);
    let comment_char = git(&["config", "--get", "core.commentChar"], true);
    warn_if_stale();
    let failures = check::failures(
        &message,
        &author,
        &committer,
        &comment_string,
        &comment_char,
    );
    if failures.is_empty() {
        return ExitCode::SUCCESS;
    }
    let report = check::report(&failures);
    // Python's text-mode stderr wrote CRLF on Windows.
    let report = if cfg!(windows) {
        report.replace('\n', "\r\n")
    } else {
        report
    };
    let _ = std::io::stderr().write_all(report.as_bytes());
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::stale_against;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Run git in `dir` with no inherited Git state or user config.
    fn git(dir: &Path, args: &[&str]) -> String {
        let mut command = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        let null = if cfg!(windows) { "nul" } else { "/dev/null" };
        let out = command
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", null)
            .env("GIT_AUTHOR_NAME", "Jane Doe")
            .env("GIT_AUTHOR_EMAIL", "jane@example.com")
            .env("GIT_COMMITTER_NAME", "Jane Doe")
            .env("GIT_COMMITTER_EMAIL", "jane@example.com")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn commit(dir: &Path, file: &str, text: &str) -> String {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", file]);
        git(dir, &["rev-parse", "HEAD"])
    }

    fn repo(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("commit-msg-guard-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "."]);
        dir
    }

    #[test]
    fn a_rule_change_on_the_remote_is_reported() {
        let dir = repo("stale");
        let old = commit(&dir, "tools/commit-msg/src/check.rs", "old\n");
        let new = commit(&dir, "tools/commit-msg/src/check.rs", "new\n");
        // A remote added by hand has origin/main but no origin/HEAD.
        git(&dir, &["update-ref", "refs/remotes/origin/main", &new]);
        assert_eq!(stale_against(&dir, &old), Some("origin/main"));
        assert_eq!(stale_against(&dir, &new), None);
        // A clone's origin/HEAD names the default branch and is preferred.
        let other = commit(&dir, ".githooks/commit-msg", "shim\n");
        git(&dir, &["update-ref", "refs/remotes/origin/trunk", &other]);
        git(
            &dir,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/trunk",
            ],
        );
        assert_eq!(stale_against(&dir, &new), Some("origin/HEAD"));
        // A change outside the rule files is not a stale guard.
        let docs = commit(&dir, "README.md", "docs\n");
        git(&dir, &["update-ref", "refs/remotes/origin/trunk", &docs]);
        assert_eq!(stale_against(&dir, &other), None);
        // An unknown source stays silent.
        assert_eq!(
            stale_against(&dir, "0000000000000000000000000000000000000000"),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
