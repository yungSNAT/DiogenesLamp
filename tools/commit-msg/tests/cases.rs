//! Byte-for-byte parity with the Python hook this crate replaced.
//!
//! Each folder under tests/cases was recorded by running that hook on
//! `message` with the identity in `env/` (dates fixed at 1700000000 +0000)
//! and the git config in `config/`: `code` is its exit code and `stderr` its
//! output (CRLF written by Windows Python recorded as LF).

mod common;

use std::fs;
use std::path::Path;

#[test]
fn every_case_matches_the_recorded_hook() {
    let repo = common::repo("cases");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert!(names.len() >= 100, "{} cases", names.len());
    let mut wrong = Vec::new();
    for name in &names {
        let dir = root.join(name);
        let mut command = common::guard(&repo);
        for entry in fs::read_dir(dir.join("env")).unwrap() {
            let entry = entry.unwrap();
            command.env(entry.file_name(), fs::read_to_string(entry.path()).unwrap());
        }
        let config = dir.join("config");
        let mut pairs = Vec::new();
        if config.is_dir() {
            for entry in fs::read_dir(&config).unwrap() {
                let entry = entry.unwrap();
                pairs.push((
                    entry.file_name().into_string().unwrap(),
                    fs::read_to_string(entry.path()).unwrap(),
                ));
            }
        }
        common::config(&mut command, &pairs);
        let out = command.arg(dir.join("message")).output().unwrap();
        let code: i32 = fs::read_to_string(dir.join("code"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let stderr = fs::read(dir.join("stderr")).unwrap();
        if out.status.code() != Some(code) || out.stderr != stderr || !out.stdout.is_empty() {
            wrong.push(format!(
                "{name}: code {:?} (want {code})\n--- got\n{}--- want\n{}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&stderr)
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ:\n{}",
        wrong.len(),
        names.len(),
        wrong.join("\n")
    );
}

#[test]
fn version_names_the_source() {
    let out = common::guard(&common::repo("version"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.starts_with(concat!(
            "commit-msg-guard ",
            env!("CARGO_PKG_VERSION"),
            " (source "
        )),
        "{text}"
    );
}

#[test]
fn bad_arguments_and_unreadable_message() {
    let repo = common::repo("args");
    let out = common::guard(&repo).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        out.stderr,
        b"usage: commit-msg-guard <message file> | --version\n"
    );
    let out = common::guard(&repo)
        .arg(repo.join("absent"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("commit-msg: cannot read "));
}
