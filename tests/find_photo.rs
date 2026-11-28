// Copyright 2026 yungSNAT
// SPDX-License-Identifier: Apache-2.0

//! `lamp find-photo`: albums whose cover fingerprint is near an image's.
mod common;

use std::path::{Path, PathBuf};

use common::{json, lamp, write};
use lamp::covers::{cover_facts, image_hash};
use lamp::pyjson::Value;
use lamp::storage::atomic_write_json;

/// A patterned PNG whose fingerprint is neither uniform nor near it.
fn photo(dir: &Path) -> PathBuf {
    let image = image::RgbImage::from_fn(90, 80, |x, y| {
        let v = ((x * 37 + y * 11 + (x * y) % 23) % 256) as u8;
        image::Rgb([v, v / 2, 255 - v])
    });
    let path = dir.join("photo.png");
    write(
        &path,
        common::encode(
            &image::DynamicImage::ImageRgb8(image),
            image::ImageFormat::Png,
        ),
    );
    path
}

/// `hash` with its lowest `n` bits flipped.
fn flipped(hash: u64, n: u32) -> String {
    format!("{:016x}", hash ^ ((1u128 << n) - 1) as u64)
}

fn album(id: &str, seller: &str, cover_hash: Option<&str>, gone: bool) -> String {
    let hash = cover_hash.map_or(String::new(), |h| format!(r#", "cover_hash": "{h}""#));
    let gone = if gone {
        r#", "gone_since": "2026-11-01T00:00:00Z""#
    } else {
        ""
    };
    format!(
        r#"{{"id": "{id}", "seller": "{seller}", "title": "Album {id}", "album_url": "https://{seller}.x.yupoo.com/albums/{id}", "thumb": "thumbs/{seller}/{id}.jpg", "links": []{hash}{gone}}}"#
    )
}

/// 2 sellers: `one` selects 9 of its 10 albums, `two` has an older index entry
/// without `album_ids`, so all of its albums count.
fn catalogue(root: &Path, hash: u64) {
    let one = [
        album("11", "one", Some(&flipped(hash, 0)), false),
        album("14", "one", Some(&flipped(hash, 3)), false),
        album("12", "one", Some(&flipped(hash, 3)), false),
        album("13", "one", Some(&flipped(hash, 8)), false),
        album("15", "one", Some(&flipped(hash, 9)), false),
        album("16", "one", Some("0000000000000000"), false),
        album("17", "one", None, false),
        album("18", "one", Some(&flipped(hash, 0)), false),
        album("19", "one", Some(&flipped(hash, 0).to_uppercase()), false),
        album(
            "20",
            "one",
            Some(&format!("+{}", &flipped(hash, 0)[1..])),
            false,
        ),
    ];
    let two = [
        album("21", "two", Some(&flipped(hash, 1)), true),
        album("22", "two", Some("not-a-hash"), false),
    ];
    atomic_write_json(
        &root.join("index.json"),
        &json(
            r#"{"schema_version": 1, "sellers": [
            {"id": "one", "shard": "catalog/one.json", "album_ids": ["11", "14", "12", "13", "15", "16", "17", "19", "20"]},
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

fn find(root: &Path, args: &[&str]) -> (i32, String, String) {
    let data = root.join("data");
    let mut full = vec!["--data", data.to_str().unwrap(), "find-photo"];
    full.extend_from_slice(args);
    lamp(&full, None)
}

fn ids(report: &Value) -> Vec<(String, i128)> {
    common::obj(report)["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let m = common::obj(m);
            let Value::Int(distance) = m["distance"] else {
                panic!("distance is not an integer")
            };
            (m["id"].py_str(), distance)
        })
        .collect()
}

#[test]
fn the_image_is_fingerprinted_as_prepare_covers_fingerprints_a_cover() {
    let dir = common::tmp();
    let path = photo(dir.path());
    assert_eq!(
        image_hash(&path).unwrap(),
        cover_facts(&path)["cover_hash"].py_str()
    );
}

#[test]
fn selected_albums_within_8_bits_are_listed_nearest_first_then_by_id() {
    let dir = common::tmp();
    let path = photo(dir.path());
    let hash = u64::from_str_radix(&image_hash(&path).unwrap(), 16).unwrap();
    catalogue(&dir.path().join("data"), hash);

    let (code, stdout, stderr) = find(dir.path(), &[path.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let report = json(&stdout);
    let fields = common::obj(&report);
    assert_eq!(fields["image_hash"].py_str(), format!("{hash:016x}"));
    // Album 18 is not selected. 16, 17, 22 and, as in the viewer, the upper
    // case 19 and the signed 20 have no usable fingerprint.
    assert!(matches!(fields["searched"], Value::Int(6)));
    assert!(matches!(fields["without_hash"], Value::Int(5)));
    assert_eq!(
        ids(&report),
        [
            ("11".to_string(), 0),
            ("21".to_string(), 1),
            ("12".to_string(), 3),
            ("14".to_string(), 3),
            ("13".to_string(), 8),
        ]
    );
    let gone = common::obj(&fields["matches"].as_array().unwrap()[1]);
    assert_eq!(gone["seller"].py_str(), "two");
    assert_eq!(gone["title"].py_str(), "Album 21");
    assert_eq!(
        gone["album_url"].py_str(),
        "https://two.x.yupoo.com/albums/21"
    );
    assert_eq!(gone["gone_since"].py_str(), "2026-11-01T00:00:00Z");

    let (code, stdout, _) = find(dir.path(), &[path.to_str().unwrap(), "--limit", "2"]);
    assert_eq!(code, 0);
    assert_eq!(
        ids(&json(&stdout)),
        [("11".to_string(), 0), ("21".to_string(), 1)]
    );
}

#[test]
fn a_limit_outside_1_to_20_is_a_usage_error() {
    let dir = common::tmp();
    let path = photo(dir.path());
    for limit in ["0", "21", "x"] {
        let (code, stdout, stderr) = find(dir.path(), &[path.to_str().unwrap(), "--limit", limit]);
        assert_eq!(code, 2, "{limit}: {stderr}");
        assert!(stdout.is_empty());
    }
}

#[test]
fn an_image_that_cannot_be_fingerprinted_is_named_with_the_reason() {
    let dir = common::tmp();
    let root = dir.path();
    catalogue(&root.join("data"), 0x0123_4567_89ab_cdef);
    let solid = root.join("solid.png");
    write(&solid, common::png(64, 64, [40, 90, 200]));
    let gif = root.join("photo.gif");
    write(&gif, b"GIF89a\x01\x00\x01\x00\x00\x00\x00;");
    let big = root.join("big.jpg");
    write(&big, vec![0u8; 8 * 1024 * 1024 + 1]);
    let text = root.join("notes.jpg");
    write(&text, b"not an image");
    let missing = root.join("missing.png");
    for (path, reason) in [
        (&solid, "it is a blank or single-colour picture"),
        (&gif, "a GIF image; only JPEG, PNG and WebP are read"),
        (&big, "larger than 8 MiB"),
        (&text, "cannot identify image file"),
        (&missing, ""),
    ] {
        let (code, stdout, stderr) = find(root, &[path.to_str().unwrap()]);
        assert_eq!(code, 1, "{stderr}");
        assert!(stdout.is_empty(), "{stdout}");
        let shown = lamp::pyerr::path_str(path);
        assert!(
            stderr.starts_with(&format!("error: cannot fingerprint {shown}: {reason}")),
            "{stderr}"
        );
    }
}

#[test]
fn a_missing_index_or_shard_is_an_error_not_a_partial_search() {
    let dir = common::tmp();
    let root = dir.path();
    let path = photo(root);
    let (code, stdout, stderr) = find(root, &[path.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stdout.is_empty() && stderr.starts_with("error: "),
        "{stderr}"
    );

    catalogue(&root.join("data"), 1);
    std::fs::remove_file(root.join("data/catalog/two.json")).unwrap();
    let (code, stdout, stderr) = find(root, &[path.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stdout.is_empty() && stderr.starts_with("error: "),
        "{stderr}"
    );

    catalogue(&root.join("data"), 1);
    let index = root.join("data/index.json");
    let text = std::fs::read_to_string(&index)
        .unwrap()
        .replace(r#""17""#, r#""17", "30""#);
    std::fs::write(&index, text).unwrap();
    let (code, stdout, stderr) = find(root, &[path.to_str().unwrap()]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(
        stderr.contains("catalog/one.json: indexed album missing from shard"),
        "{stderr}"
    );
}

#[test]
fn malformed_index_or_shard_json_is_an_error_not_a_partial_search() {
    let dir = common::tmp();
    let root = dir.path();
    let path = photo(root);
    for broken in ["index.json", "catalog/two.json"] {
        catalogue(&root.join("data"), 1);
        std::fs::write(root.join("data").join(broken), "{not json").unwrap();
        let (code, stdout, stderr) = find(root, &[path.to_str().unwrap()]);
        assert_eq!(code, 1, "{broken}: {stderr}");
        assert!(
            stdout.is_empty() && stderr.starts_with("error: "),
            "{broken}: {stderr}"
        );
    }
}
