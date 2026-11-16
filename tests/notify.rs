// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! `lamp scrape --notify`: the run's new and changed albums, as JSON on the
//! stdin of the user's command.
mod common;

use std::io::Read;
use std::path::Path;

use common::{json, obj};
use lamp::cli::notify;
use lamp::maintenance::{EXPORT_COLUMNS, run_rows};
use lamp::pyjson::{self, Map, Style, Value};
use lamp::storage::atomic_write_json;

const RUN: &str = "2026-11-17T03:47:00+11:00";
const EARLIER: &str = "2026-11-16T03:47:00+11:00";

/// Not a test of lamp. The notify tests run this test binary as their
/// command, with `NOTIFY_COPY_TO` set, and it copies its stdin to that file.
#[test]
fn copy_stdin_helper() {
    if let Some(path) = std::env::var_os("NOTIFY_COPY_TO") {
        let mut input = Vec::new();
        std::io::stdin().read_to_end(&mut input).unwrap();
        std::fs::write(path, input).unwrap();
    }
}

/// A shell command that runs `copy_stdin_helper`, copying its stdin to `to`.
fn copy_to(to: &Path) -> String {
    let exe = std::env::current_exe().unwrap();
    let (exe, to) = (exe.display(), to.display());
    if cfg!(windows) {
        format!(r#"set "NOTIFY_COPY_TO={to}" && "{exe}" copy_stdin_helper --exact --quiet > NUL"#)
    } else {
        format!("NOTIFY_COPY_TO='{to}' '{exe}' copy_stdin_helper --exact --quiet > /dev/null")
    }
}

fn album(id: &str, title: &str, extra: &str) -> String {
    format!(
        r#"{{"id": "{id}", "seller": "one", "title": {}, "album_url": "https://one.x.yupoo.com/albums/{id}", "thumb": "", "links": []{extra}}}"#,
        pyjson::dumps(&Value::from(title), Style::Default)
    )
}

/// Seller `one` indexes albums 2, 3 and 4; album 1 is a duplicate the index
/// leaves out. Seller `two` has no `album_ids` and no shard: the index could
/// not read it.
fn catalogue(root: &Path, title: &str) {
    let albums = [
        album(
            "1",
            title,
            &format!(r#", "price": 99, "first_seen": "{RUN}""#),
        ),
        album(
            "2",
            "Price rise",
            &format!(
                r#", "price": 120, "first_seen": "{EARLIER}", "history": [
                {{"at": "{EARLIER}", "field": "purchase links", "before": [], "after": []}},
                {{"at": "{RUN}", "field": "price", "before": 100, "after": 120}}]"#
            ),
        ),
        album(
            "3",
            "Unchanged",
            &format!(
                r#", "first_seen": "{EARLIER}", "history": [
                {{"at": "{EARLIER}", "field": "price", "before": 1, "after": 2}}]"#
            ),
        ),
        album(
            "4",
            "Gone",
            &format!(
                r#", "first_seen": "{EARLIER}", "gone_since": "{RUN}", "history": [
                {{"at": "{RUN}", "field": "availability", "before": "listed", "after": "gone"}}]"#
            ),
        ),
    ];
    atomic_write_json(
        &root.join("index.json"),
        &json(
            r#"{"schema_version": 1, "sellers": [
            {"id": "one", "shard": "catalog/one.json", "album_ids": ["2", "3", "4"]},
            {"id": "two", "shard": "catalog/two.json"}]}"#,
        ),
    )
    .unwrap();
    atomic_write_json(
        &root.join("catalog/one.json"),
        &json(&format!(
            r#"{{"seller": "one", "albums": [{}]}}"#,
            albums.join(",")
        )),
    )
    .unwrap();
}

fn ids(rows: &[Map]) -> Vec<String> {
    rows.iter().map(|r| r["album_id"].py_str()).collect()
}

#[test]
fn rows_are_the_albums_the_run_stored_or_changed_with_its_changes() {
    let dir = common::tmp();
    catalogue(dir.path(), "New hoodie");
    let rows = run_rows(dir.path(), RUN).unwrap();
    assert_eq!(ids(&rows), ["1", "2", "4"]);
    let keys: Vec<&str> = rows[0].keys().map(String::as_str).collect();
    let mut want = EXPORT_COLUMNS.to_vec();
    want.extend(["new", "changes"]);
    assert_eq!(keys, want);
    let shown = |row: &Map, key: &str| pyjson::dumps(&row[key], Style::Default);
    assert_eq!(shown(&rows[0], "new"), "true");
    assert_eq!(shown(&rows[0], "changes"), "[]");
    assert_eq!(shown(&rows[0], "price"), "99");
    assert_eq!(shown(&rows[1], "new"), "false");
    assert_eq!(
        shown(&rows[1], "changes"),
        r#"[{"field": "price", "before": 100, "after": 120}]"#
    );
    assert_eq!(
        shown(&rows[2], "changes"),
        r#"[{"field": "availability", "before": "listed", "after": "gone"}]"#
    );
    assert_eq!(rows[2]["gone_since"].as_str(), Some(RUN));
    // Another run's timestamp matches nothing.
    assert!(
        run_rows(dir.path(), "2026-11-18T03:47:00+11:00")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_command_gets_the_rows_on_stdin_and_never_in_its_command_line() {
    let dir = common::tmp();
    let marker = dir.path().join("pwned");
    let m = marker.display();
    let title =
        format!(r#"x"; touch '{m}'; $(touch '{m}') `touch '{m}'` & echo>"{m}" | %PATH% ^& "#);
    catalogue(dir.path(), &title);
    let rows = run_rows(dir.path(), RUN).unwrap();
    let want = pyjson::dumps(
        &Value::Array(rows.iter().cloned().map(Value::Object).collect()),
        Style::Indent(2),
    ) + "\n";
    let copy = dir.path().join("stdin.json");
    notify(&copy_to(&copy), rows).unwrap();
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), want);
    assert_eq!(
        obj(&json(&want).as_array().unwrap()[0])["title"].as_str(),
        Some(title.as_str())
    );
    assert!(!marker.exists(), "scraped text ran as a command");
}

#[test]
fn no_rows_run_no_command() {
    let dir = common::tmp();
    let copy = dir.path().join("stdin.json");
    notify(&copy_to(&copy), Vec::new()).unwrap();
    assert!(!copy.exists());
}

#[test]
fn a_failing_command_is_an_error_with_its_status() {
    let dir = common::tmp();
    catalogue(dir.path(), "x");
    let rows = run_rows(dir.path(), RUN).unwrap();
    let error = notify("exit 3", rows).unwrap_err();
    assert!(
        error.starts_with("the command failed (") && error.contains('3'),
        "{error}"
    );
}

/// A command may ignore its input; more than a pipe buffer of rows must not
/// turn that into an error.
#[test]
fn a_command_that_reads_nothing_succeeds() {
    let dir = common::tmp();
    catalogue(dir.path(), &"long title ".repeat(100));
    let rows = run_rows(dir.path(), RUN).unwrap();
    let many: Vec<Map> = (0..300).flat_map(|_| rows.clone()).collect();
    notify("exit 0", many).unwrap();
}

/// End to end without fetching: the only seller is skipped, so the run
/// stores nothing and the command does not run.
#[test]
fn scrape_runs_no_command_when_nothing_changed() {
    let dir = common::tmp();
    let sources = dir.path().join("sources.toml");
    common::write(
        &sources,
        "[yupoo.one]\nurl = \"https://one.x.yupoo.com\"\nname = \"one\"\nskip = true\n",
    );
    let data = dir.path().join("data");
    catalogue(&data, "x");
    let copy = dir.path().join("stdin.json");
    let command = copy_to(&copy);
    let (code, out, err) = common::lamp(
        &[
            "--sources",
            sources.to_str().unwrap(),
            "--data",
            data.to_str().unwrap(),
            "scrape",
            "--notify",
            &command,
        ],
        None,
    );
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(!copy.exists());
}
