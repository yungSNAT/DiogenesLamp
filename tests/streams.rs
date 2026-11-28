// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

//! A standard stream that cannot be written fails as it did for the Python
//! release, measured against it on a closed pipe: stdout still pending at exit
//! is exit status 120, a maintenance command's handler reports a failed print,
//! an uncaught one exits 1, a diagnostic that cannot be written loses the
//! original code, and help ignores its own write errors.
mod common;

use std::path::Path;
use std::process::{Command, Stdio};

/// The OSError Python raised for a write to a pipe with no reader.
const CLOSED_PIPE: &str = if cfg!(windows) {
    "OSError: [Errno 22] Invalid argument"
} else {
    "BrokenPipeError: [Errno 32] Broken pipe"
};

/// A pipe write end whose read end is closed everywhere.
///
/// On Unix, a sibling test thread's concurrent `Command::spawn` (`fork` or
/// `clone`) can inherit `reader` before `drop(reader)` runs and hold it open
/// until `execve` closes `O_CLOEXEC` descriptors, so the child under test
/// writes to a live pipe instead of failing with `BrokenPipe`. A 1-byte probe
/// fails with `BrokenPipe` only once no reader remains anywhere; an anonymous
/// pipe with no reader can never gain another.
fn closed_pipe() -> Stdio {
    closed_pipe_from(|| {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        writer
    })
    .into()
}

fn closed_pipe_from(mut make: impl FnMut() -> std::io::PipeWriter) -> std::io::PipeWriter {
    use std::io::{ErrorKind, Write};
    for _ in 0..1000 {
        let mut writer = make();
        match writer.write_all(&[0]) {
            Err(e) if e.kind() == ErrorKind::BrokenPipe => return writer,
            Ok(()) => std::thread::yield_now(),
            Err(e) => panic!("closed pipe probe failed unexpectedly: {e}"),
        }
    }
    panic!("closed pipe still had a reader after 1000 attempts");
}

/// Exit code and the text on the stream that still works.
fn run(cwd: &Path, args: &[&str], broken_stdout: bool, unbuffered: bool) -> (i32, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lamp"));
    command
        .args(args)
        .current_dir(cwd)
        .env("COLUMNS", "80")
        .env_remove("PYTHONUNBUFFERED");
    if unbuffered {
        command.env("PYTHONUNBUFFERED", "1");
    }
    let output = if broken_stdout {
        command.stdout(closed_pipe()).stderr(Stdio::piped())
    } else {
        command.stdout(Stdio::piped()).stderr(closed_pipe())
    }
    .output()
    .unwrap();
    let other = if broken_stdout {
        output.stderr
    } else {
        output.stdout
    };
    (
        output.status.code().unwrap(),
        String::from_utf8(other).unwrap().replace("\r\n", "\n"),
    )
}

/// A catalogue directory, a dump to import and a seller whose fetch is
/// refused before any request is sent.
fn workspace() -> tempfile::TempDir {
    let dir = common::tmp();
    common::write(&dir.path().join("data").join("readme"), "");
    common::write(
        &dir.path().join("links.txt"),
        "https://exampleseller.x.yupoo.com/albums\n",
    );
    common::write(
        &dir.path().join("offline.toml"),
        "[yupoo.bad]\nurl = \"https://127.0.0.1:9\"\nname = \"bad\"\n",
    );
    dir
}

#[test]
fn pending_stdout_fails_at_exit_with_status_120() {
    let dir = workspace();
    let flush = format!("Exception ignored while flushing sys.stdout:\n{CLOSED_PIPE}\n");
    for args in [&["--help"][..], &["doctor"], &["import", "links.txt"]] {
        assert_eq!(
            run(dir.path(), args, true, false),
            (120, flush.clone()),
            "{args:?}"
        );
    }
}

#[test]
fn unbuffered_stdout_fails_at_the_print() {
    let dir = workspace();
    let message = CLOSED_PIPE.split_once(": ").unwrap().1;
    assert_eq!(run(dir.path(), &["--help"], true, true), (0, String::new()));
    assert_eq!(
        run(dir.path(), &["doctor"], true, true),
        (1, format!("error: {message}\n"))
    );
    assert_eq!(
        run(dir.path(), &["import", "links.txt"], true, true),
        (1, format!("{CLOSED_PIPE}\n"))
    );
}

#[test]
fn a_scrape_report_line_fails_at_once() {
    let dir = workspace();
    let scrape = ["--sources", "offline.toml", "scrape"];
    assert_eq!(
        run(dir.path(), &scrape, true, false),
        (
            120,
            format!("{CLOSED_PIPE}\nException ignored while flushing sys.stdout:\n{CLOSED_PIPE}\n")
        )
    );
    assert_eq!(
        run(dir.path(), &scrape, true, true),
        (1, format!("{CLOSED_PIPE}\n"))
    );
    // The run stopped before the index was written.
    assert!(!dir.path().join("data").join("index.json").exists());
}

/// A pipe whose read end is still held (as by a sibling test's concurrent
/// spawn on Unix) is discarded rather than handed to a child expecting EPIPE.
#[test]
fn closed_pipe_discards_a_pipe_with_a_live_reader() {
    use std::io::{ErrorKind, Write};
    let (_held_reader, live_writer) = std::io::pipe().unwrap();
    let mut leaked = Some(live_writer);
    let mut calls = 0;
    let mut writer = closed_pipe_from(|| {
        calls += 1;
        leaked.take().unwrap_or_else(|| {
            let (reader, writer) = std::io::pipe().unwrap();
            drop(reader);
            writer
        })
    });
    assert!(calls >= 2);
    assert_eq!(
        writer.write_all(&[0]).unwrap_err().kind(),
        ErrorKind::BrokenPipe
    );
}

/// A stdout opened without write access fails as a bad file descriptor on
/// every platform: on Windows the C runtime under Python's `write()` reports
/// ERROR_ACCESS_DENIED as EBADF, unlike the closed pipe's EINVAL.
#[test]
fn a_read_only_stdout_is_a_bad_file_descriptor() {
    let dir = workspace();
    let path = dir.path().join("read-only.txt");
    std::fs::write(&path, "").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lamp"))
        .arg("--help")
        .current_dir(dir.path())
        .env("COLUMNS", "80")
        .env_remove("PYTHONUNBUFFERED")
        .stdout(std::fs::File::open(&path).unwrap())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(120));
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .replace("\r\n", "\n"),
        "Exception ignored while flushing sys.stdout:\nOSError: [Errno 9] Bad file descriptor\n"
    );
}

#[test]
fn an_unwritable_diagnostic_loses_the_original_code() {
    let dir = workspace();
    let missing = ["import", "missing.txt"];
    assert_eq!(
        run(dir.path(), &missing, false, false),
        (120, String::new())
    );
    assert_eq!(run(dir.path(), &missing, false, true), (1, String::new()));
    assert_eq!(
        run(dir.path(), &["--bogus"], false, false),
        (120, String::new())
    );
    assert_eq!(
        run(dir.path(), &["--bogus"], false, true),
        (2, String::new())
    );
}

/// With stdout closed, Python's sys.stdout was None and print wrote nothing.
#[cfg(unix)]
#[test]
fn a_closed_stdout_discards_output() {
    let dir = workspace();
    let output = Command::new("sh")
        .args(["-c", "exec \"$0\" >&-", env!("CARGO_BIN_EXE_lamp")])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .env_remove("PYTHONUNBUFFERED")
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "");
}

/// On a terminal stdout is line-buffered, so the next steps arrive whole.
#[cfg(target_os = "linux")]
#[test]
fn a_terminal_stdout_gets_the_next_steps() {
    // util-linux script(1) runs lamp on a pseudo-terminal.
    let output = Command::new("script")
        .args(["-q", "-e", "-c", "exec \"$LAMP\"", "/dev/null"])
        .env("LAMP", env!("CARGO_BIN_EXE_lamp"))
        .env_remove("PYTHONUNBUFFERED")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("lamp --help lists every command."),
        "{text:?}"
    );
}
