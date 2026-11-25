// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Metadata admission: each malformed index, run summary or shard is refused
//! with the reason `lamp doctor` and `lamp restore` report, and each refused
//! canonical link or catalogue path names why.
mod common;

use common::json;
use lamp::schema::{data_path, public_https_host, validate_metadata};

fn refusal(text: &str, relative: &str) -> String {
    match validate_metadata(&json(text), relative) {
        Ok(()) => panic!("{relative} admitted {text}"),
        Err(e) => e.to_string(),
    }
}

const SELLER: &str = r#"{"id": "one", "shard": "catalog/one.json"}"#;

#[test]
fn index_refusals_name_the_broken_field() {
    let cases = [
        (
            r#"{"selection_version": 2, "sellers": []}"#.to_string(),
            "unsupported selection version",
        ),
        (
            r#"{"sellers": {}}"#.into(),
            "index sellers must be an array",
        ),
        (r#"{"sellers": [1]}"#.into(), "invalid seller ID"),
        (r#"{"sellers": [{"id": 5}]}"#.into(), "invalid seller ID"),
        (
            r#"{"sellers": [{"id": "One", "shard": "catalog/One.json"}]}"#.into(),
            "invalid seller ID",
        ),
        (
            format!(r#"{{"sellers": [{SELLER}, {SELLER}]}}"#),
            "duplicate seller or mismatched shard path",
        ),
        (
            r#"{"sellers": [{"id": "one", "shard": "catalog/two.json"}]}"#.into(),
            "duplicate seller or mismatched shard path",
        ),
        (
            r#"{"sellers": [{"id": "one", "shard": "catalog/one.json", "album_ids": "1"}]}"#.into(),
            "invalid indexed album IDs",
        ),
        (
            r#"{"sellers": [{"id": "one", "shard": "catalog/one.json", "album_ids": ["x"]}]}"#
                .into(),
            "invalid indexed album IDs",
        ),
        (
            r#"{"sellers": [{"id": "one", "shard": "catalog/one.json", "album_ids": ["1", "1"]}]}"#
                .into(),
            "invalid indexed album IDs",
        ),
        (
            r#"{"sellers": [{"id": "one", "shard": "catalog/one.json", "album_ids": ["1"]},
                {"id": "two", "shard": "catalog/two.json", "album_ids": ["1"]}]}"#
                .into(),
            "duplicate indexed album ID across sellers",
        ),
        (
            format!(r#"{{"selection_version": 1, "sellers": [{SELLER}]}}"#),
            "selected index requires album IDs for every seller",
        ),
        (
            r#"{"sellers": [], "shops": {}}"#.into(),
            "index shops must be an array",
        ),
        (
            r#"{"sellers": [], "shops": [1]}"#.into(),
            "invalid shop entry",
        ),
        (
            r#"{"sellers": [], "shops": [{"url": "https://shop.example/", "preferred": "yes"}]}"#
                .into(),
            "invalid shop preferred flag",
        ),
        (
            r#"{"sellers": [], "shops": [{"name": "Missing URL"}]}"#.into(),
            "invalid shop URL",
        ),
        (
            r#"{"sellers": [], "shops": [{"name": 5, "platform": "web", "url": "https://shop.example/"}]}"#
                .into(),
            "invalid shop name or platform",
        ),
        (
            r#"{"sellers": [], "shops": [{"name": "No platform", "url": "https://shop.example/"}]}"#
                .into(),
            "invalid shop name or platform",
        ),
    ];
    for (text, reason) in cases {
        assert_eq!(refusal(&text, "index.json"), reason, "{text}");
    }
    for bad_url in [
        "javascript:alert(1)",
        "data:text/html,<h1>x</h1>",
        "file:///etc/passwd",
        "https://[bad/",
        "https://user:pass@shop.example/",
        "https://user@shop.example/",
        "https://",
        "http:///x",
        // The viewer's URL parser refuses these, so the shop would vanish there.
        "http://shop example/",
        "http://shop.example:99999/",
        "http://shop.example:x/",
        "http://shop^example/",
    ] {
        let text = format!(r#"{{"sellers": [], "shops": [{{"url": "{bad_url}"}}]}}"#);
        assert_eq!(
            refusal(&text, "index.json"),
            "invalid shop URL",
            "{bad_url}"
        );
    }
    let selected = r#"{"selection_version": 1, "sellers": [
        {"id": "one", "shard": "catalog/one.json", "album_ids": ["1", "2"]},
        {"id": "two", "shard": "catalog/two.json", "album_ids": ["3"]}],
        "shops": [
            {"id": "s1", "platform": "weidian", "name": "S1", "url": "https://weidian.com/?userid=1", "preferred": true},
            {"id": "s2", "platform": "web", "name": "S2", "url": "http://shop.example/"},
            {"id": "s3", "platform": "web", "name": "S3", "url": "http://[::1]:8080/x"}]}"#;
    validate_metadata(&json(selected), "index.json").unwrap();
}

#[test]
fn run_summary_refusals_name_the_broken_field() {
    let cases = [
        (r#"{"failed_ids": ["x"]}"#, "invalid failed album IDs"),
        (
            r#"{"short_missing": "1"}"#,
            "invalid short_missing album IDs",
        ),
        (r#"{"last_attempt": 5}"#, "invalid last_attempt timestamp"),
        (
            r#"{"last_listing": null}"#,
            "invalid last_listing timestamp",
        ),
    ];
    for (text, reason) in cases {
        assert_eq!(refusal(text, "runs/one.json"), reason, "{text}");
    }
    let good = r#"{"failed_ids": ["1"], "short_missing": [], "last_success": "2026-11-19T10:00:00+11:00"}"#;
    validate_metadata(&json(good), "runs/one.json").unwrap();
}

/// A shard holding one album with `album` spliced into an otherwise valid
/// record; the record has an empty cover unless `album` names one.
fn shard(album: &str) -> String {
    let thumb = if album.contains(r#""thumb""#) {
        ""
    } else {
        r#", "thumb": """#
    };
    format!(
        r#"{{"seller": "one", "albums": [{{"seller": "one", "title": "T",
            "album_url": "https://one.x.yupoo.com/albums/1", "links": []{thumb}{album}}}]}}"#
    )
}

#[test]
fn shard_refusals_name_the_broken_field() {
    let cases = [
        (
            r#"{"seller": "one", "albums": {}}"#.to_string(),
            "shard albums must be an array",
        ),
        (
            r#"{"seller": "one", "albums": [1]}"#.into(),
            "invalid album ID",
        ),
        (shard(""), "invalid album ID"),
        (shard(r#", "id": true"#), "invalid album ID"),
        (shard(r#", "id": "1x""#), "invalid album ID"),
        (shard(r#", "id": 1"#), "duplicate or non-string album ID"),
        (
            shard(r#", "id": "1", "thumb": "thumbs/one""#),
            "unsafe cover path",
        ),
        (
            shard(r#", "id": "1", "thumb": "thumbs/one/1""#),
            "unsafe cover path",
        ),
        (
            shard(r#", "id": "1", "history": [{"at": 1}]"#),
            "invalid change history",
        ),
        (
            shard(r#", "id": "1", "history": {}"#),
            "invalid change history",
        ),
    ];
    for (text, reason) in cases {
        assert_eq!(refusal(&text, "catalog/one.json"), reason, "{text}");
    }
    let good = shard(r#", "id": "1", "thumb": "thumbs/one/1.webp", "history": [{"at": "x"}]"#);
    validate_metadata(&json(&good), "catalog/one.json").unwrap();
}

#[test]
fn canonical_links_refuse_what_the_viewer_cannot_key() {
    let refused = "canonical links require public HTTPS without credentials";
    let cases = [
        ("https:///albums/1", refused),
        ("http://one.x.yupoo.com/", refused),
        ("https://user@one.x.yupoo.com/", refused),
        ("https://user:pw@one.x.yupoo.com/", refused),
        ("https://*.yupoo.com/", "invalid canonical hostname"),
        ("https://1..2/", "invalid IPv4 hostname"),
    ];
    for (url, reason) in cases {
        let error = public_https_host(url).expect_err(url);
        assert_eq!(error.to_string(), reason, "{url}");
    }
}

#[test]
fn catalogue_paths_outside_the_known_names_are_refused() {
    let dir = common::tmp();
    for (relative, cover) in [
        ("index", false),
        ("catalog/one.txt", false),
        ("thumbs/one", true),
        ("thumbs/one/1", true),
    ] {
        let error = data_path(dir.path(), relative, cover).expect_err(relative);
        assert_eq!(error.to_string(), "unsafe catalogue path", "{relative}");
    }
    assert!(data_path(dir.path(), "runs/one.json", false).is_ok());
    assert!(data_path(dir.path(), "thumbs/one/1.png", true).is_ok());
}
