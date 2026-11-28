// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

//! `lamp demo`: synthetic data in its own temporary directory, never beside a
//! named `--data` or in the working directory.
mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
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

/// A port that was free when chosen and that no other test here was given:
/// the tests run at once, and 2 given one port meet each other's demo.
fn free_port() -> String {
    static GIVEN: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    loop {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut given = GIVEN.lock().unwrap();
        if !given.contains(&port) {
            given.push(port);
            return port.to_string();
        }
    }
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
fn demo_data_passes_doctor() {
    // 200 albums a seller include gone ones (every 97th), which the index must not count.
    let dir = common::tmp();
    lamp::demo::write(dir.path(), 2, 200, 0).unwrap();
    let (code, stdout, _) =
        common::lamp(&["--data", &dir.path().to_string_lossy(), "doctor"], None);
    assert_eq!(code, 0, "{stdout}");
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
    // Hold the port from the first bind: a port freed and bound again can be
    // taken in between as another test's outgoing local port.
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

// A path already taken by a folder fails the write that needs it.
#[test]
fn demo_write_reports_a_file_it_cannot_write() {
    for blocked in [
        &["index.json"][..],
        &["catalog/demoseller0.json"],
        &[
            "thumbs/demoseller0/10000000.png",
            "thumbs/demoseller0/10000001.png",
            "thumbs/demoseller0/10000002.png",
        ],
    ] {
        let dir = tempfile::tempdir().unwrap();
        for path in blocked {
            std::fs::create_dir_all(dir.path().join(path)).unwrap();
        }
        let error = lamp::demo::write(dir.path(), 1, 3, 3)
            .unwrap_err()
            .to_string();
        // The OS refuses to open a directory for writing.
        let expected = if cfg!(windows) {
            "Access is denied."
        } else {
            "Is a directory"
        };
        assert!(error.contains(expected), "{blocked:?}: {error}");
    }
}

/// Without a temporary directory to write in, nothing is served.
#[test]
fn demo_without_a_temporary_directory_fails() {
    let dirs = dirs();
    let missing = dirs.temp.join("missing");
    let output = demo(&dirs, &free_port())
        .env("TMPDIR", &missing)
        .env("TMP", &missing)
        .env("TEMP", &missing)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("error: could not write demo data: "),
        "{stderr}"
    );
    assert!(!missing.exists());
}

/// Whether this process may write where the file mode says it may not.
#[cfg(unix)]
fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

/// A demo folder made read-only by the umask fails the write and is removed.
#[cfg(unix)]
#[test]
fn demo_reports_data_it_cannot_write_and_removes_the_folder() {
    if is_root() {
        return;
    }
    let dirs = dirs();
    let lamp = demo(&dirs, &free_port());
    let output = Command::new("sh")
        .arg("-c")
        .arg("umask 222 && exec \"$0\" \"$@\"")
        .arg(lamp.get_program())
        .args(lamp.get_args())
        .current_dir(&dirs.work)
        .env("TMPDIR", &dirs.temp)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("error: could not write demo data: "),
        "{stderr}"
    );
    assert!(stderr.contains("Permission denied"), "{stderr}");
    assert!(entries(&dirs.temp).is_empty(), "demo data left behind");
}

/// Data the demo cannot remove when stopped is named, not left silently.
#[cfg(unix)]
#[test]
fn demo_names_data_it_could_not_remove() {
    use std::os::unix::fs::PermissionsExt;
    if is_root() {
        return;
    }
    let dirs = dirs();
    let port = free_port();
    let mut child = demo(&dirs, &port).spawn().unwrap();
    let index = get(&mut child, &port, "/data/index.json");
    assert!(index.starts_with("HTTP/1.0 200"), "{index}");
    let catalog = dirs.temp.join(&entries(&dirs.temp)[0]).join("catalog");
    std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o555)).unwrap();
    let signalled = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signalled.success());
    let output = child.wait_with_output().unwrap();
    std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("warning: could not remove "), "{stderr}");
    assert!(catalog.is_dir());
}

/// A stdout that cannot take the demo's first line ends it, data removed.
#[cfg(target_os = "linux")]
#[test]
fn demo_stops_and_removes_its_data_when_stdout_fails() {
    let dirs = dirs();
    let full = std::fs::File::create("/dev/full").unwrap();
    let output = demo(&dirs, &free_port()).stdout(full).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(stderr, "OSError: [Errno 28] No space left on device\n");
    assert!(entries(&dirs.temp).is_empty(), "demo data left behind");
}
