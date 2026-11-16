// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! `lamp export`: the indexed albums as CSV or JSON on stdout, or the viewer
//! and catalogue as a static site.
mod common;

use std::path::Path;
use std::process::Command;

use common::{json, obj};
use lamp::pyjson::Value;
use lamp::storage::atomic_write_json;

/// Titles a spreadsheet would misread, or that need quoting, checked
/// against the viewer's own csvCell.
const AWKWARD: [&str; 10] = [
    "=SUM(A1)",
    " +1",
    "@x",
    "-5",
    "\tTab first",
    "say \"hi\"",
    "\u{feff}=x",
    "\u{85}=x",
    "\u{3000}-y",
    "plain, with a comma",
];

/// An album record; `extra` adds fields, `links` among them when it has any.
fn album(id: &str, seller: &str, title: &str, extra: &str) -> String {
    let links = if extra.contains(r#""links""#) {
        ""
    } else {
        r#", "links": []"#
    };
    format!(
        r#"{{"id": "{id}", "seller": "{seller}", "title": {}, "album_url": "https://{seller}.x.yupoo.com/albums/{id}", "thumb": "thumbs/{seller}/{id}.jpg"{links}{extra}}}"#,
        lamp::pyjson::dumps(&Value::from(title), lamp::pyjson::Style::Default)
    )
}

/// Seller `one` selects 3 of its 4 albums; seller `two` has an older index
/// entry without `album_ids`, so both of its albums count.
fn catalogue(root: &Path) {
    let one = [
        album(
            "11",
            "one",
            "Hoodie",
            r#", "price": 128, "updated_at": "2026-11-14T10:00:00+11:00", "first_seen": "2026-11-14T10:00:00+11:00",
               "description": "=cmd", "links": [{"platform": "weidian", "url": "https://weidian.com/item.html?itemID=1"},
               {"platform": "taobao", "url": "https://item.taobao.com/item.htm?id=2"}]"#,
        ),
        album(
            "12",
            "one",
            "Jacket",
            r#", "price": 0, "first_seen": "2026-11-16T00:30:00+11:00""#,
        ),
        album(
            "13",
            "one",
            "Unselected",
            r#", "first_seen": "2026-11-16T09:00:00+11:00""#,
        ),
        album(
            "14",
            "one",
            "Gone",
            r#", "first_seen": "2026-11-15T23:59:00+11:00", "gone_since": "2026-11-16T09:00:00+11:00""#,
        ),
    ];
    let two = [
        album("21", "two", "No dates", ""),
        album(
            "22",
            "two",
            "Later",
            r#", "first_seen": "2026-11-17T08:00:00+11:00""#,
        ),
    ];
    atomic_write_json(
        &root.join("index.json"),
        &json(
            r#"{"schema_version": 1, "sellers": [
            {"id": "one", "shard": "catalog/one.json", "album_ids": ["11", "12", "14"]},
            {"id": "two", "shard": "catalog/two.json"}]}"#,
        ),
    )
    .unwrap();
    for (seller, albums) in [("one", &one[..]), ("two", &two[..])] {
        atomic_write_json(
            &root.join(format!("catalog/{seller}.json")),
            &json(&format!(
                r#"{{"seller": "{seller}", "albums": [{}]}}"#,
                albums.join(",")
            )),
        )
        .unwrap();
    }
}

/// (exit code, stdout bytes, stderr).
fn export(root: &Path, args: &[&str]) -> (i32, Vec<u8>, String) {
    let data = root.join("data");
    let mut full = vec!["--data", data.to_str().unwrap(), "export"];
    full.extend_from_slice(args);
    let output = common::lamp_output(&full, None);
    (
        output.status.code().unwrap_or(-1),
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn csv_is_the_viewers_format_with_the_selected_albums_in_index_order() {
    let dir = common::tmp();
    catalogue(&dir.path().join("data"));
    let (code, stdout, stderr) = export(dir.path(), &[]);
    assert_eq!(code, 0, "{stderr}");
    let expected = [
        r#""seller","album_id","title","currency","price","details_checked","album_url","purchase_links","description","first_seen","gone_since""#,
        r#""one","11","Hoodie","CNY","128","2026-11-14T10:00:00+11:00","https://one.x.yupoo.com/albums/11","https://weidian.com/item.html?itemID=1 https://item.taobao.com/item.htm?id=2","'=cmd","2026-11-14T10:00:00+11:00","""#,
        r#""one","12","Jacket","CNY","","","https://one.x.yupoo.com/albums/12","","","2026-11-16T00:30:00+11:00","""#,
        r#""one","14","Gone","CNY","","","https://one.x.yupoo.com/albums/14","","","2026-11-15T23:59:00+11:00","2026-11-16T09:00:00+11:00""#,
        r#""two","21","No dates","CNY","","","https://two.x.yupoo.com/albums/21","","","","""#,
        r#""two","22","Later","CNY","","","https://two.x.yupoo.com/albums/22","","","2026-11-17T08:00:00+11:00","""#,
    ];
    let mut want = "\u{feff}".to_string();
    for line in expected {
        want.push_str(line);
        want.push_str("\r\n");
    }
    assert_eq!(String::from_utf8(stdout).unwrap(), want);
}

#[test]
fn json_has_the_same_fields_with_nulls_and_a_link_list() {
    let dir = common::tmp();
    catalogue(&dir.path().join("data"));
    let (code, stdout, stderr) = export(dir.path(), &["--format", "json"]);
    assert_eq!(code, 0, "{stderr}");
    let rows = json(&String::from_utf8(stdout).unwrap());
    let rows = rows.as_array().unwrap();
    let ids: Vec<String> = rows.iter().map(|r| obj(r)["album_id"].py_str()).collect();
    assert_eq!(ids, ["11", "12", "14", "21", "22"]);
    let first = obj(&rows[0]);
    let keys: Vec<&str> = first.keys().map(String::as_str).collect();
    assert_eq!(keys, lamp::maintenance::EXPORT_COLUMNS);
    assert!(matches!(first["price"], Value::Int(128)));
    let links: Vec<String> = first["purchase_links"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::py_str)
        .collect();
    assert_eq!(
        links,
        [
            "https://weidian.com/item.html?itemID=1",
            "https://item.taobao.com/item.htm?id=2"
        ]
    );
    assert_eq!(first["description"].py_str(), "=cmd");
    let second = obj(&rows[1]);
    for key in ["price", "details_checked", "description", "gone_since"] {
        assert!(matches!(second[key], Value::Null), "{key}");
    }
}

#[test]
fn since_keeps_albums_first_stored_on_that_local_date_or_later() {
    let dir = common::tmp();
    catalogue(&dir.path().join("data"));
    let (code, stdout, stderr) = export(dir.path(), &["--format", "json", "--since", "2026-11-16"]);
    assert_eq!(code, 0, "{stderr}");
    let rows = json(&String::from_utf8(stdout).unwrap());
    let ids: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| obj(r)["album_id"].py_str())
        .collect();
    // 14 was first stored late on 15 November, 21 has no first_seen and 13
    // is not selected.
    assert_eq!(ids, ["12", "22"]);
}

#[test]
fn since_reads_first_seen_as_a_date_and_skips_one_that_is_not() {
    let dir = common::tmp();
    let data = dir.path().join("data");
    let albums = [
        // Text that sorts after 2026-11-16 but is no date.
        album(
            "1",
            "one",
            "Unpadded",
            r#", "first_seen": "2026-11-6T00:00:00+11:00""#,
        ),
        album("2", "one", "Not a date", r#", "first_seen": "2026-11-99""#),
        album(
            "3",
            "one",
            "On the day",
            r#", "first_seen": "2026-11-16T00:30:00+11:00""#,
        ),
        album(
            "4",
            "one",
            "Day before",
            r#", "first_seen": "2026-11-15T23:59:59+11:00""#,
        ),
        // Other ISO forms the parser reads, each on 16 November but the last.
        album("5", "one", "Space", r#", "first_seen": "2026-11-16 08:00""#),
        album(
            "6",
            "one",
            "Week",
            r#", "first_seen": "2026-W47-1T00:00:00+11:00""#,
        ),
        album(
            "7",
            "one",
            "Midnight",
            r#", "first_seen": "2026-11-15T24:00:00+11:00""#,
        ),
        album("8", "one", "Date only", r#", "first_seen": "2026-11-16""#),
        album(
            "9",
            "one",
            "Week before",
            r#", "first_seen": "2026-W46-7T12:00:00""#,
        ),
    ];
    atomic_write_json(
        &data.join("index.json"),
        &json(r#"{"schema_version": 1, "sellers": [{"id": "one", "shard": "catalog/one.json"}]}"#),
    )
    .unwrap();
    atomic_write_json(
        &data.join("catalog/one.json"),
        &json(&format!(
            r#"{{"seller": "one", "albums": [{}]}}"#,
            albums.join(",")
        )),
    )
    .unwrap();
    let (code, stdout, stderr) = export(dir.path(), &["--format", "json", "--since", "2026-11-16"]);
    assert_eq!(code, 0, "{stderr}");
    let rows = json(&String::from_utf8(stdout).unwrap());
    let ids: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| obj(r)["album_id"].py_str())
        .collect();
    assert_eq!(ids, ["3", "5", "6", "7", "8"]);
}

#[test]
fn a_bad_format_or_date_is_a_usage_error() {
    let dir = common::tmp();
    catalogue(&dir.path().join("data"));
    for args in [
        &["--format", "xml"][..],
        &["--since", "2026-13-01"],
        &["--since", "16/11/2026"],
        &["--since", "2026-11-1"],
        &["--since", "20261116"],
    ] {
        let (code, stdout, stderr) = export(dir.path(), args);
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert!(stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn a_missing_index_or_shard_prints_nothing() {
    let dir = common::tmp();
    let (code, stdout, stderr) = export(dir.path(), &[]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stdout.is_empty() && stderr.starts_with("error: "),
        "{stderr}"
    );
    catalogue(&dir.path().join("data"));
    std::fs::remove_file(dir.path().join("data/catalog/two.json")).unwrap();
    let (code, stdout, stderr) = export(dir.path(), &[]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stdout.is_empty() && stderr.starts_with("error: "),
        "{stderr}"
    );

    catalogue(&dir.path().join("data"));
    let index = dir.path().join("data/index.json");
    let text = std::fs::read_to_string(&index)
        .unwrap()
        .replace(r#""14""#, r#""14", "30""#);
    std::fs::write(&index, text).unwrap();
    let (code, stdout, stderr) = export(dir.path(), &[]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty());
    assert!(
        stderr.contains("catalog/one.json: indexed album missing from shard"),
        "{stderr}"
    );
}

/// The CLI's cells match the viewer's csvCell for the same text.
#[test]
fn csv_cells_match_the_viewers_csv_cell() {
    let node = common::node().expect("node is required: the viewer comparison must run");
    let dir = common::tmp();
    let data = dir.path().join("data");
    let albums: Vec<String> = AWKWARD
        .iter()
        .enumerate()
        .map(|(i, title)| album(&(i + 1).to_string(), "one", title, ""))
        .collect();
    atomic_write_json(
        &data.join("index.json"),
        &json(r#"{"schema_version": 1, "sellers": [{"id": "one", "shard": "catalog/one.json"}]}"#),
    )
    .unwrap();
    atomic_write_json(
        &data.join("catalog/one.json"),
        &json(&format!(
            r#"{{"seller": "one", "albums": [{}]}}"#,
            albums.join(",")
        )),
    )
    .unwrap();
    let (code, stdout, stderr) = export(dir.path(), &[]);
    assert_eq!(code, 0, "{stderr}");
    let csv = String::from_utf8(stdout).unwrap();
    let titles: Vec<String> = AWKWARD.iter().map(|t| t.to_string()).collect();
    let script = format!(
        "const {{csvCell}} = require('./viewer/discovery.js'); \
         process.stdout.write(JSON.stringify({}.map(csvCell)));",
        lamp::pyjson::dumps(
            &Value::Array(titles.into_iter().map(Value::from).collect()),
            lamp::pyjson::Style::Default
        )
    );
    let viewer = Command::new(node)
        .args(["-e", &script])
        .current_dir(common::root())
        .output()
        .unwrap();
    assert!(
        viewer.status.success(),
        "{}",
        String::from_utf8_lossy(&viewer.stderr)
    );
    let expected = json(&String::from_utf8(viewer.stdout).unwrap());
    let lines: Vec<&str> = csv.trim_end_matches("\r\n").split("\r\n").skip(1).collect();
    let cells = expected.as_array().unwrap();
    assert_eq!(lines.len(), AWKWARD.len());
    for (line, cell) in lines.iter().zip(cells) {
        // The title is the third cell; no album ID or seller holds a comma.
        let rest = line.splitn(3, ',').nth(2).unwrap();
        let want = format!("{},\"CNY\",", cell.py_str());
        assert!(rest.starts_with(&want), "{line} against {cell:?}");
    }
}

/// The catalogue with 2 cached covers, an aliases.json and the files a site
/// must leave behind; returns the data directory.
fn site_catalogue(root: &Path) -> std::path::PathBuf {
    let data = root.join("data");
    catalogue(&data);
    common::write(&data.join("thumbs/one/11.jpg"), b"cover 11");
    common::write(&data.join("thumbs/two/21.jpg"), b"cover 21");
    common::write(&data.join("aliases.json"), br#"{"brands": {}}"#);
    common::write(&data.join("runs/one.json"), b"{}");
    common::write(&data.join("snapshots/old.zip"), b"zip");
    data
}

/// Every file under `dir`, as slash-separated relative paths.
fn tree(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap().to_string_lossy();
                found.push(relative.replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_site_holds_the_viewer_and_the_catalogue_it_reads_and_nothing_else() {
    let dir = common::tmp();
    let data = site_catalogue(dir.path());
    let site = dir.path().join("site");
    let (code, stdout, stderr) = export(dir.path(), &["--site", site.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let stdout = String::from_utf8(stdout).unwrap();
    assert!(
        stdout.contains("2 sellers and 2 covers (4 not cached"),
        "{stdout}"
    );
    let mut expected: Vec<String> = lamp::viewer::FILES
        .iter()
        .map(|(name, _)| name.to_string())
        .chain(
            [
                "aliases.json",
                "catalog/one.json",
                "catalog/two.json",
                "index.json",
                "thumbs/one/11.jpg",
                "thumbs/two/21.jpg",
            ]
            .map(|f| format!("data/{f}")),
        )
        .collect();
    expected.sort();
    assert_eq!(tree(&site), expected);
    for name in [
        "index.json",
        "catalog/one.json",
        "thumbs/two/21.jpg",
        "aliases.json",
    ] {
        assert_eq!(
            std::fs::read(site.join("data").join(name)).unwrap(),
            std::fs::read(data.join(name)).unwrap(),
            "{name}"
        );
    }
    for (name, bytes) in lamp::viewer::FILES {
        let written = std::fs::read(site.join(name)).unwrap();
        if *name != "index.html" {
            assert_eq!(&written, bytes, "{name}");
        }
    }
    // A static host sends no policy header, so the page carries the one
    // `lamp serve` sends, before any script or style loads.
    let html = std::fs::read_to_string(site.join("index.html")).unwrap();
    let policy = format!(
        "<meta charset=\"utf-8\">\n<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
        lamp::serve::CSP
    );
    assert!(html.contains(&policy), "{html}");
    assert!(html.find(&policy) < html.find("<script"));
    assert_eq!(
        html.replacen(&policy, "<meta charset=\"utf-8\">", 1)
            .as_bytes(),
        lamp::viewer::get("index.html").unwrap()
    );
}

#[test]
fn a_site_needs_a_new_or_empty_folder_outside_the_data_directory() {
    let dir = common::tmp();
    let data = site_catalogue(dir.path());
    let site = dir.path().join("site");
    common::write(&site.join("keep.txt"), b"mine");
    let (code, stdout, stderr) = export(dir.path(), &["--site", site.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stdout.is_empty() && stderr.contains("folder is not empty"),
        "{stderr}"
    );
    assert_eq!(tree(&site), ["keep.txt"]);

    let inside = data.join("site");
    let (code, _, stderr) = export(dir.path(), &["--site", inside.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("outside the data directory"), "{stderr}");
    assert!(!inside.exists());

    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let args = ["--site", empty.to_str().unwrap(), "--since", "2026-11-16"];
    let (code, _, stderr) = export(dir.path(), &args);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("leave out --since"), "{stderr}");
    let (code, _, stderr) = export(dir.path(), &args[..2]);
    assert_eq!(code, 0, "{stderr}");
    assert!(empty.join("data/index.json").is_file());
}

#[test]
fn a_bad_catalogue_writes_no_site() {
    let dir = common::tmp();
    let data = site_catalogue(dir.path());
    std::fs::remove_file(data.join("catalog/two.json")).unwrap();
    let site = dir.path().join("site");
    let (code, stdout, stderr) = export(dir.path(), &["--site", site.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stdout.is_empty() && stderr.starts_with("error: "),
        "{stderr}"
    );
    assert!(!site.exists());
}
