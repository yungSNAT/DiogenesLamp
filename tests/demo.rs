// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! `lamp demo`: synthetic data in its own temporary directory, never beside a
//! named `--data` or in the working directory.
mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A scratch root with the system temporary directory, a working directory
/// and the parent of a named `--data`, each created empty.
struct Dirs {
    _root: tempfile::TempDir,
    temp: PathBuf,
    work: PathBuf,
    named: PathBuf,
}

fn dirs() -> Dirs {
    let root = common::tmp();
    let [temp, work, named] = ["temp", "work", "named"].map(|name| {
        let dir = root.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        dir
    });
    Dirs {
        _root: root,
        temp,
        work,
        named,
    }
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// `lamp ... demo --no-browser --port <port>` with `dirs.temp` as the system
/// temporary directory and a named `--data` and `--sources` under `dirs.named`.
fn demo(dirs: &Dirs, port: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lamp"));
    command
        .args([
            "--data",
            &dirs.named.join("data").to_string_lossy(),
            "--sources",
            &dirs.named.join("sources.toml").to_string_lossy(),
            "demo",
            "--no-browser",
            "--port",
            port,
        ])
        .current_dir(&dirs.work)
        .env("TMPDIR", &dirs.temp)
        .env("TMP", &dirs.temp)
        .env("TEMP", &dirs.temp)
        .env("PYTHONUNBUFFERED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn free_port() -> String {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string()
}

/// GET `path` once the port accepts. The server binds before it writes the
/// data and answers only once it is written, so this waits for both.
fn get(child: &mut Child, port: &str, path: &str) -> String {
    let started = Instant::now();
    loop {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())) {
            stream
                .set_read_timeout(Some(Duration::from_secs(300)))
                .unwrap();
            stream
                .write_all(format!("GET {path} HTTP/1.0\r\n\r\n").as_bytes())
                .unwrap();
            let mut data = Vec::new();
            stream.read_to_end(&mut data).unwrap();
            return String::from_utf8_lossy(&data).into_owned();
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "lamp demo exited before serving"
        );
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "lamp demo did not bind its port"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn demo_serves_synthetic_data_and_writes_nowhere_near_named_data() {
    let dirs = dirs();
    let port = free_port();
    let mut child = demo(&dirs, &port).spawn().unwrap();
    let index = get(&mut child, &port, "/data/index.json");
    let made = entries(&dirs.temp);
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(index.starts_with("HTTP/1.0 200"), "{index}");
    assert!(index.contains("\"demoseller4\""), "{index}");
    assert_eq!(made.len(), 1, "{made:?}");
    assert!(made[0].starts_with("lamp-demo-"), "{made:?}");
    let demo_dir = dirs.temp.join(&made[0]);
    assert!(demo_dir.join("index.json").is_file());
    assert_eq!(entries(&demo_dir.join("catalog")).len(), 5);
    assert!(
        entries(&dirs.named).is_empty(),
        "demo wrote beside the named --data"
    );
    assert!(
        entries(&dirs.work).is_empty(),
        "demo wrote in the working directory"
    );
    assert!(
        stdout.contains("demo catalogue: 5 sellers x 2000 albums (6665 covers) in "),
        "{stdout}"
    );
    assert!(stdout.contains("serving on http://127.0.0.1:"), "{stdout}");
}

#[test]
fn demo_on_a_busy_port_writes_nothing() {
    let dirs = dirs();
    let busy = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = busy.local_addr().unwrap().port().to_string();
    let output = demo(&dirs, &port).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!("error: could not listen on 127.0.0.1:{port}")),
        "{stderr}"
    );
    for dir in [&dirs.temp, &dirs.work, &dirs.named] {
        assert!(entries(dir).is_empty(), "{} was written", dir.display());
    }
}

/// Ctrl+C stops the server and removes the demo data. Signalled with
/// `kill -INT`, so Unix only.
#[cfg(unix)]
#[test]
fn demo_removes_its_data_when_interrupted() {
    let dirs = dirs();
    let port = free_port();
    let mut child = demo(&dirs, &port).spawn().unwrap();
    let index = get(&mut child, &port, "/data/index.json");
    assert!(index.starts_with("HTTP/1.0 200"), "{index}");
    assert_eq!(entries(&dirs.temp).len(), 1);
    let signalled = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signalled.success());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(entries(&dirs.temp).is_empty(), "demo data left behind");
}
