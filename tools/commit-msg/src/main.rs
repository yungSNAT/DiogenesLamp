//! Refuse AI authorship credit before a commit is written.
//!
//! `.githooks/commit-msg` runs this binary from the hooks directory, where the
//! install step in CONTRIBUTING.md puts it; it never runs from the working
//! tree. Usage: `commit-msg-guard <message file>`, or `--version`.

use std::io::Write;
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = match args.as_slice() {
        [flag] if flag == "--version" => {
            println!(
                "commit-msg-guard {} (source {})",
                env!("CARGO_PKG_VERSION"),
                SOURCE.unwrap_or("unknown")
            );
            return ExitCode::SUCCESS;
        }
        [path] => path,
        _ => {
            eprintln!("usage: commit-msg-guard <message file> | --version");
            return ExitCode::from(2);
        }
    };
    let message = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("commit-msg: cannot read {path}: {err}");
            return ExitCode::FAILURE;
        }
    };
    // Each configured comment value is passed through rather than resolved:
    // the scan treats every one as a marker instead of picking between them.
    let author = git(&["var", "GIT_AUTHOR_IDENT"], false);
    let committer = git(&["var", "GIT_COMMITTER_IDENT"], false);
    let comment_string = git(&["config", "--get", "core.commentString"], true);
    let comment_char = git(&["config", "--get", "core.commentChar"], true);
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
    let _ = std::io::stderr().write_all(check::report(&failures).as_bytes());
    ExitCode::FAILURE
}
