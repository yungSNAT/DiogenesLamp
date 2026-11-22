// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Ported from tests/test_cli.py and tests/test_smoke.py.
mod common;

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

use common::{lamp, read, write};
use lamp::cli::report_line;
use lamp::scrape::SellerSummary;

#[test]
fn package_version() {
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.1.3");
}

/// The release smoke test runs `lamp --version` and needs a first line that
/// ends in the release version.
#[test]
fn version_flag_prints_the_package_version() {
    for args in [&["--version"][..], &["--vers"], &["--version", "serve"]] {
        let (code, out, err) = lamp(args, None);
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (0, concat!("lamp ", env!("CARGO_PKG_VERSION"), "\n"), ""),
            "{args:?}"
        );
    }
    let (code, out, err) = lamp(&["--version=1"], None);
    assert_eq!(code, 2);
    assert!(out.is_empty());
    assert!(err.ends_with("lamp: error: argument --version: ignored explicit argument '1'\n"));
}

/// A double-clicked `lamp.exe` runs with no arguments; it gets the next
/// steps, not a usage error. Any argument still goes to the parser.
#[test]
fn no_arguments_print_the_next_steps() {
    let (code, out, err) = lamp(&[], None);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.starts_with(concat!("lamp ", env!("CARGO_PKG_VERSION"))));
    for step in [
        "lamp demo",
        "lamp import",
        "lamp scrape",
        "lamp serve",
        "lamp --help",
    ] {
        assert!(out.contains(step), "{step} missing from {out:?}");
    }
    let (code, out, err) = lamp(&["bogus"], None);
    assert_eq!(code, 2);
    assert!(out.is_empty() && err.contains("lamp: error: argument command: invalid choice"));
}

#[test]
fn import_creates_sources() {
    let dir = common::tmp();
    write(
        &dir.path().join("links.csv"),
        "1\thttps://sellerx.x.yupoo.com/\n2\thttps://weidian.com/?userid=7\n3\tN/A\n",
    );
    let sources = dir.path().join("sources.toml");
    let (code, out, _) = lamp(
        &[
            "--sources",
            &sources.to_string_lossy(),
            "import",
            &dir.path().join("links.csv").to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 0);
    let text = read(&sources);
    assert!(text.contains("[yupoo.sellerx]") && text.contains("[shops.weidian-7]"));
    assert!(out.contains("3 rows") && out.contains("1 invalid"));
    assert!(out.contains("yupoo sellers: 1 new, 0 known"));
    assert!(out.contains("shops: 1 new, 0 known"));
}

#[test]
fn import_reports_missing_and_undecodable_dumps() {
    let dir = common::tmp();
    let s = dir.path().join("s.toml").to_string_lossy().into_owned();
    let (code, _, err) = lamp(
        &[
            "--sources",
            &s,
            "import",
            &dir.path().join("nope.csv").to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(err.contains("not found"));
    let dump = dir.path().join("dump.csv");
    write(&dump, b"https://ok.x.yupoo.com/\n\x81\x9d not utf-8");
    let (code, _, err) = lamp(&["--sources", &s, "import", &dump.to_string_lossy()], None);
    assert_eq!(code, 1);
    assert!(
        err.contains("could not read")
            && err.contains("utf-8")
            && err.contains("re-save it as UTF-8")
    );
    assert!(!dir.path().join("s.toml").exists());
}

#[test]
fn import_accepts_utf16_excel_exports() {
    let dir = common::tmp();
    let mut blob = vec![0xff, 0xfe];
    for unit in "1\thttps://sellerx.x.yupoo.com/\n".encode_utf16() {
        blob.extend_from_slice(&unit.to_le_bytes());
    }
    let dump = dir.path().join("dump.csv");
    write(&dump, blob);
    let s = dir.path().join("s.toml");
    let (code, _, _) = lamp(
        &[
            "--sources",
            &s.to_string_lossy(),
            "import",
            &dump.to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 0);
    assert!(read(&s).contains("[yupoo.sellerx]"));
}

fn sources_file(dir: &std::path::Path) -> String {
    let path = dir.join("sources.toml");
    write(
        &path,
        "[yupoo.sellerx]\nurl = \"https://sellerx.x.yupoo.com\"\nname = \"sellerx\"\n",
    );
    path.to_string_lossy().into_owned()
}

#[test]
fn scrape_requires_sources_and_a_known_seller() {
    let dir = common::tmp();
    let data = dir.path().join("d").to_string_lossy().into_owned();
    let (code, _, err) = lamp(
        &[
            "--sources",
            &dir.path().join("none.toml").to_string_lossy(),
            "--data",
            &data,
            "scrape",
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(err.contains("no sources"));
    let (code, _, err) = lamp(
        &[
            "--sources",
            &sources_file(dir.path()),
            "--data",
            &data,
            "scrape",
            "--seller",
            "nosuchseller",
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(err.contains("unknown seller"));
}

#[test]
fn scrape_report_lines_name_each_diagnostic() {
    let summary = SellerSummary {
        scraped: 4,
        skipped: 2,
        failed: 1,
        failed_ids: vec!["9".into()],
        thumbs_blocked: true,
        dated: 0,
        skipped_old: 7,
        thumbs_refetched: 3,
        ..SellerSummary::new("sellerx")
    };
    let line = report_line(&summary, 1, 1, false);
    assert!(line.starts_with("[1/1] sellerx: 4 scraped, 2 skipped, 1 failed"));
    for part in [
        "7 too old",
        "3 covers fetched again",
        "failed albums: 9",
        "thumbs blocked",
        "no publish dates found",
    ] {
        assert!(line.contains(part), "{part}");
    }
    let errored = SellerSummary {
        error: "blocked: HTTP 429".into(),
        ..SellerSummary::new("sellerx")
    };
    assert!(report_line(&errored, 1, 1, false).contains("ERROR: blocked: HTTP 429"));
    let many = SellerSummary {
        failed_ids: (1..=7).map(|i| i.to_string()).collect(),
        ..SellerSummary::new("s")
    };
    assert!(report_line(&many, 2, 3, true).contains("(failed albums: 1, 2, 3, 4, 5, ...)"));
    let changed = SellerSummary {
        scraped: 7,
        new_ids: (11..=17).map(|i| i.to_string()).collect(),
        returned_ids: vec!["9".into()],
        recounted_ids: vec!["8".into(), "6".into()],
        partly_scanned_ids: vec!["12".into()],
        ..SellerSummary::new("s")
    };
    assert_eq!(
        report_line(&changed, 1, 1, false),
        "[1/1] s: 7 scraped, 0 skipped, 0 failed, 7 new, 1 returned, 2 recounted, \
         1 partly scanned (new albums: 11, 12, 13, 14, 15, ...) (returned albums: 9) \
         (recounted albums: 8, 6) (partly scanned albums: 12) \
         (no publish dates found - Yupoo markup change?)"
    );
    // A run with nothing new or returned keeps its line as before.
    assert!(!report_line(&summary, 1, 1, false).contains("new"));
}

#[test]
fn serve_warns_when_there_is_no_index_and_needs_no_source_tree() {
    let dir = common::tmp();
    // A port that was free when chosen can be taken before lamp binds it, by
    // another process or another lamp. Lamp then exits without its
    // "serving on" line, so wait for that line before connecting, and try
    // another port if it never comes.
    let (mut child, port, serving, rest) = (1..=5)
        .find_map(|_| {
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let mut child = Command::new(env!("CARGO_BIN_EXE_lamp"))
                .args([
                    "--data",
                    &dir.path().to_string_lossy(),
                    "serve",
                    "--no-browser",
                    "--port",
                    &port.to_string(),
                ])
                .current_dir(dir.path())
                // Piped stdout is buffered until exit, as Python's was, and
                // the server is killed, so the line must be flushed at once.
                .env_remove("PYTHONUNBUFFERED")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
            let (tx, rx) = std::sync::mpsc::channel();
            let rest = std::thread::spawn(move || {
                use std::io::BufRead;
                let mut line = String::new();
                let _ = stdout.read_line(&mut line);
                let _ = tx.send(line);
                let mut rest = String::new();
                let _ = stdout.read_to_string(&mut rest);
                rest
            });
            let line = rx.recv_timeout(Duration::from_secs(10)).unwrap_or_default();
            if line.starts_with("serving on http://127.0.0.1:") {
                return Some((child, port, line, rest));
            }
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            eprintln!(
                "port {port}: no serving line ({line:?}); {}; stderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            None
        })
        .expect("lamp serve did not start on any of 5 ports");
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut data = Vec::new();
    let sent = {
        use std::io::Write;
        stream
            .write_all(b"GET / HTTP/1.0\r\n\r\n")
            .and_then(|()| stream.read_to_end(&mut data))
    };
    let exited = child.try_wait();
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // A failure names the server's state and output, so a rare CI failure
    // can be diagnosed from its log.
    let report = format!(
        "port {port}; server exited before kill: {exited:?}; {} bytes read: {:?}\n\
         server stdout: {serving}{}\nserver stderr: {stderr}",
        data.len(),
        String::from_utf8_lossy(&data),
        rest.join().unwrap()
    );
    assert!(sent.is_ok(), "request failed: {sent:?}; {report}");
    assert!(data.starts_with(b"HTTP/1.0 200"), "{report}");
    assert!(stderr.contains("no index.json"), "{report}");
}

#[test]
fn nonsense_numeric_args_are_usage_errors() {
    for args in [
        &["scrape", "--workers", "0"][..],
        &["scrape", "--rps", "0"],
        &["scrape", "--rps", "-4"],
        &["scrape", "--rps", "nan"],
        &["scrape", "--rps", "inf"],
        &["serve", "--port", "0"],
        &["serve", "--port", "99999"],
    ] {
        let (code, out, err) = lamp(args, None);
        assert_eq!(code, 2, "{args:?}");
        assert!(out.is_empty() && err.contains("error"));
    }
}

#[test]
fn help_and_usage_follow_argparse() {
    let (code, out, _) = lamp(&["-h"], None);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: lamp [-h] [--sources SOURCES] [--data DATA] [--version]\n"));
    assert!(out.contains("  -h, --help            show this help message and exit\n"));
    assert!(out.contains("  --version             show program's version number and exit\n"));
    assert!(out.contains("    demo                serve the viewer over synthetic data in a"));
    let (code, _, err) = lamp(&["bogus"], None);
    assert_eq!(code, 2);
    assert!(err.contains(
        "lamp: error: argument command: invalid choice: 'bogus' (choose from 'import', 'scrape'"
    ));
    let (code, _, err) = lamp(&["scrape", "--re", "1"], None);
    assert_eq!(code, 2);
    assert!(err.ends_with("lamp scrape: error: ambiguous option: --re could match --refresh-details-days, --refresh-limit, --retry-failed\n"));
}

/// Excel's other Unicode exports: UTF-16 big-endian and UTF-8 with a BOM.
#[test]
fn import_accepts_utf16_big_endian_and_utf8_with_a_bom() {
    // The link starts the file, so a byte order mark left in would spoil it.
    let row = "https://sellerx.x.yupoo.com/\n";
    let mut big_endian = vec![0xfe, 0xff];
    for unit in row.encode_utf16() {
        big_endian.extend_from_slice(&unit.to_be_bytes());
    }
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(row.as_bytes());
    for blob in [big_endian, bom] {
        let dir = common::tmp();
        let dump = dir.path().join("dump.csv");
        write(&dump, blob);
        let s = dir.path().join("s.toml");
        let (code, out, err) = lamp(
            &[
                "--sources",
                &s.to_string_lossy(),
                "import",
                &dump.to_string_lossy(),
            ],
            None,
        );
        assert_eq!(code, 0, "{err}");
        assert!(
            out.starts_with(
                "1 rows read, 0 invalid
yupoo sellers: 1 new"
            ),
            "{out}"
        );
        assert!(read(&s).contains("[yupoo.sellerx]"));
    }
}

#[test]
fn import_reports_a_dump_it_cannot_read() {
    let dir = common::tmp();
    let s = dir.path().join("s.toml");
    let (code, _, err) = lamp(
        &[
            "--sources",
            &s.to_string_lossy(),
            "import",
            &dir.path().to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(err.starts_with("error: "), "{err}");
    assert!(!s.exists());
}

/// A sources file lamp cannot read is reported, never replaced.
#[test]
fn a_corrupt_sources_file_is_reported_and_left_unchanged() {
    let dir = common::tmp();
    let s = dir.path().join("s.toml");
    write(&s, "[yupoo.sellerx\n");
    let dump = dir.path().join("dump.csv");
    write(&dump, "1\thttps://sellery.x.yupoo.com/\n");
    let sources = s.to_string_lossy().into_owned();
    for args in [
        vec!["--sources", &sources, "import", dump.to_str().unwrap()],
        vec!["--sources", &sources, "scrape"],
    ] {
        let (code, _, err) = lamp(&args, Some(dir.path()));
        assert_eq!(code, 1, "{args:?}");
        assert!(err.starts_with("error: invalid sources file"), "{err}");
        assert_eq!(read(&s), "[yupoo.sellerx\n");
    }
}

#[test]
fn report_lines_count_refreshed_and_deferred_details() {
    let summary = SellerSummary {
        scraped: 1,
        dated: 1,
        refreshed: 3,
        deferred: 2,
        ..SellerSummary::new("s")
    };
    assert_eq!(
        report_line(&summary, 1, 1, false),
        "[1/1] s: 1 scraped, 0 skipped, 0 failed (3 details refreshed, 2 deferred by budget)"
    );
    let none = SellerSummary {
        scraped: 1,
        dated: 1,
        ..SellerSummary::new("s")
    };
    assert!(!report_line(&none, 1, 1, false).contains("refreshed"));
}

/// Ctrl+C stops `lamp serve` with exit code 0 and frees its port. Signalled
/// with `kill -INT`, so Unix only.
#[cfg(unix)]
#[test]
fn serve_stops_cleanly_on_ctrl_c() {
    use std::io::BufRead;
    let dir = common::tmp();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lamp"))
        .args([
            "--data",
            &dir.path().to_string_lossy(),
            "serve",
            "--no-browser",
            "--port",
            &port,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(
        line,
        format!("serving on http://127.0.0.1:{port}/ (Ctrl+C to stop)\n")
    );
    let signalled = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signalled.success());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no index.json"));
    assert!(std::net::TcpListener::bind(("127.0.0.1", port.parse::<u16>().unwrap())).is_ok());
}

/// `lamp scrape`'s usage, as CPython 3.15.0's argparse wraps it at 80 columns.
const SCRAPE_USAGE: &str = r#"usage: lamp scrape [-h] [--seller SELLER] [--full] [--refresh-details-days N]
                   [--refresh-limit N] [--retry-failed] [--workers WORKERS]
                   [--rps RPS] [--max-age-days N] [--dry-run]
                   [--notify COMMAND]
"#;

/// `lamp scrape -h` from CPython 3.15.0's argparse with the same parser.
const SCRAPE_HELP: &str = r#"usage: lamp scrape [-h] [--seller SELLER] [--full] [--refresh-details-days N]
                   [--refresh-limit N] [--retry-failed] [--workers WORKERS]
                   [--rps RPS] [--max-age-days N] [--dry-run]
                   [--notify COMMAND]

options:
  -h, --help            show this help message and exit
  --seller SELLER       scrape only this seller id
  --full                re-fetch unchanged albums
  --refresh-details-days N
                        also refresh unchanged details last checked N days ago
  --refresh-limit N     maximum extra unchanged-detail requests across this
                        run
  --retry-failed        retry only album IDs recorded as failed in earlier
                        runs
  --workers WORKERS
  --rps RPS             global request rate cap
  --max-age-days N      skip albums posted more than N days ago (0 = no age
                        limit); stock that old is usually gone
  --dry-run             preview scrape work without fetching album pages or
                        saving files
  --notify COMMAND      after a run that stores or changes albums, run COMMAND
                        in the system shell with them as JSON on its stdin
                        (not after --dry-run or Ctrl+C)
"#;

/// The scrape usage wraps, and its help lays out long option names and help
/// text, as argparse does.
#[test]
fn scrape_help_and_usage_errors_match_argparse() {
    let (code, out, _) = lamp(&["scrape", "-h"], None);
    assert_eq!((code, out.as_str()), (0, SCRAPE_HELP));
    for (args, message) in [
        (
            ["scrape", "--workers", "0"],
            "argument --workers: must be 1 or greater",
        ),
        // `-.5` reads as a number, not an option, so it reaches --rps.
        (
            ["scrape", "--rps", "-.5"],
            "argument --rps: must be a finite number greater than 0",
        ),
    ] {
        let (code, out, err) = lamp(&args, None);
        assert_eq!(code, 2, "{args:?}");
        assert!(out.is_empty());
        assert_eq!(
            err,
            format!("{SCRAPE_USAGE}lamp scrape: error: {message}\n"),
            "{args:?}"
        );
    }
}

#[test]
fn arguments_that_look_like_options_follow_argparse() {
    let (code, _, err) = lamp(&["scrape", "extra"], None);
    assert_eq!(code, 2);
    assert!(
        err.ends_with("\nlamp: error: unrecognized arguments: extra\n"),
        "{err}"
    );
    // A negative number is a value, so the scrape starts and finds no sources.
    let dir = common::tmp();
    let (code, _, err) = lamp(
        &["--sources", "none.toml", "scrape", "--max-age-days", "-5"],
        Some(dir.path()),
    );
    assert_eq!(
        (code, err.as_str()),
        (1, "error: no sources - run 'lamp import' first\n")
    );
    // After `--`, and on its own, `-` starts a file name.
    for (args, name) in [
        (
            &["--sources", "s.toml", "import", "--", "-x.csv"][..],
            "-x.csv",
        ),
        (&["--sources", "s.toml", "import", "-"], "-"),
    ] {
        let (code, _, err) = lamp(args, Some(dir.path()));
        assert_eq!(
            (code, err),
            (1, format!("error: dump file not found: {name}\n")),
            "{args:?}"
        );
    }
}

/// A seller at 127.0.0.1: the fetcher refuses a non-global address before
/// sending anything, so a scrape runs without the network.
fn offline_sources(dir: &std::path::Path, skip: bool) -> String {
    let path = dir.join("offline.toml");
    write(
        &path,
        format!(
            "[yupoo.sellerx]\nurl = \"https://127.0.0.1\"\nname = \"sellerx\"\nskip = {skip}\n"
        ),
    );
    path.to_string_lossy().into_owned()
}

#[test]
fn scrape_of_one_seller_reports_its_error_or_a_held_catalogue() {
    let dir = common::tmp();
    let data = dir.path().join("d");
    let data_text = data.to_string_lossy().into_owned();
    let sources = offline_sources(dir.path(), false);
    let args = [
        "--sources",
        sources.as_str(),
        "--data",
        data_text.as_str(),
        "scrape",
        "--seller",
        "sellerx",
        "--rps",
        "2",
    ];
    std::fs::create_dir_all(&data).unwrap();
    let lock = lamp::locking::CatalogueLock::acquire(&data).unwrap();
    let (code, out, err) = lamp(&args, None);
    assert_eq!(code, 1);
    assert!(out.is_empty(), "{out}");
    assert!(
        err.starts_with("error: catalogue is already in use"),
        "{err}"
    );
    drop(lock);
    let (code, out, err) = lamp(&args, None);
    assert_eq!(code, 1, "{err}");
    assert!(out.contains("non-global address"), "{out}");
    assert!(out.contains("1 sellers, 1 with errors"), "{out}");
    assert!(!out.contains("skipped ("), "{out}");
}

#[test]
fn a_dry_run_with_every_seller_skipped_fetches_nothing() {
    let dir = common::tmp();
    let data = dir.path().join("d").to_string_lossy().into_owned();
    let sources = offline_sources(dir.path(), true);
    let (code, out, err) = lamp(
        &[
            "--sources",
            &sources,
            "--data",
            &data,
            "scrape",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("0 sellers, 0 with errors, 1 skipped (sellerx) [DRY RUN - no files written]"),
        "{out}"
    );
    assert!(!dir.path().join("d").exists());
}

#[test]
fn serve_on_a_busy_port_fails() {
    let dir = common::tmp();
    let busy = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = busy.local_addr().unwrap().port().to_string();
    let data = dir.path().to_string_lossy().into_owned();
    let (code, _, err) = lamp(
        &["--data", &data, "serve", "--no-browser", "--port", &port],
        None,
    );
    assert_eq!(code, 1);
    assert!(
        err.contains(&format!("error: could not listen on 127.0.0.1:{port}")),
        "{err}"
    );
}
