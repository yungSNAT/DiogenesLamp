// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Ported from tests/test_maintenance.py.
mod common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use common::{json, lamp, obj, read_json, write};
use lamp::covers::{cover_facts, resize_cover};
use lamp::isodate::Date;
use lamp::maintenance::{diagnose, prepare_covers, restore, snapshot};
use lamp::pyjson::{self, Style, Value};
use lamp::schema::{data_path, validate_metadata};
use lamp::storage::{atomic_write_json, load_shard};

fn catalogue(root: &Path) {
    atomic_write_json(&root.join("index.json"), &json(r#"{"schema_version": 1, "sellers": [{"id": "example", "shard": "catalog/example.json", "album_count": 1}]}"#)).unwrap();
    atomic_write_json(
        &root.join("catalog/example.json"),
        &json(r#"{"seller": "example", "albums": [{"id": "123", "seller": "example", "title": "Example", "price": 0,
            "album_url": "https://example.x.yupoo.com/albums/123", "links": [], "thumb": "thumbs/example/123.jpg"}]}"#),
    )
    .unwrap();
}

fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn zip_member(path: &Path, name: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut data = Vec::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_end(&mut data)
        .unwrap();
    data
}

fn make_zip(path: &Path, members: &[(&str, Vec<u8>)]) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in members {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn doctor_is_read_only_and_reports_coverage() {
    let dir = common::tmp();
    catalogue(dir.path());
    let before = tree(dir.path());
    let result = diagnose(dir.path());
    assert_eq!(result["issues"].py_str(), "[]");
    for key in ["albums", "missing_covers", "unpriced"] {
        assert!(matches!(result[key], Value::Int(1)), "{key}");
    }
    assert_eq!(before, tree(dir.path()));
    let (code, _, _) = lamp(&["--data", &dir.path().to_string_lossy(), "doctor"], None);
    assert_eq!(code, 0);
}

/// The current UTC time as a snapshot name writes it, `YYYYMMDDTHHMMSS`.
fn utc_stamp() -> String {
    const UNIX_EPOCH_ORDINAL: i64 = 719_163; // 1970-01-01 as a proleptic Gregorian ordinal.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (y, m, d) = Date::from_ordinal(UNIX_EPOCH_ORDINAL + secs / 86_400)
        .unwrap()
        .ymd();
    let t = secs % 86_400;
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}",
        t / 3600,
        t / 60 % 60,
        t % 60
    )
}

#[test]
fn snapshot_names_carry_the_utc_time() {
    let dir = common::tmp();
    catalogue(dir.path());
    let before = utc_stamp();
    let archive = snapshot(dir.path()).unwrap();
    let after = utc_stamp();
    let name = archive.file_name().unwrap().to_str().unwrap();
    let shape = regex::Regex::new(r"^\d{8}T\d{6}-[0-9a-f]{8}\.zip$").unwrap();
    assert!(shape.is_match(name), "{name}");
    let stamp = &name[..15];
    assert!(
        before.as_str() <= stamp && stamp <= after.as_str(),
        "{before} {name} {after}"
    );
}

#[test]
fn snapshot_restores_corrupt_metadata_and_keeps_previous_bytes() {
    let dir = common::tmp();
    catalogue(dir.path());
    let archive = snapshot(dir.path()).unwrap();
    let shard = dir.path().join("catalog/example.json");
    let original = std::fs::read(&shard).unwrap();
    write(&shard, "corrupt");
    let previous = restore(dir.path(), &archive).unwrap().unwrap();
    assert_eq!(std::fs::read(&shard).unwrap(), original);
    assert_eq!(zip_member(&previous, "catalog/example.json"), b"corrupt");
    assert_eq!(diagnose(dir.path())["issues"].py_str(), "[]");
}

#[test]
fn restore_rejects_a_snapshot_with_a_duplicate_member() {
    let dir = common::tmp();
    catalogue(dir.path());
    let archive = snapshot(dir.path()).unwrap();
    let index = zip_member(&archive, "index.json");
    let shard = zip_member(&archive, "catalog/example.json");
    // The zip writer refuses a repeated name, so write a placeholder of the
    // same length and rename it in the local and central headers. Both
    // copies are valid, so only the repeated name can make restore refuse.
    let bad = dir.path().join("bad.zip");
    make_zip(
        &bad,
        &[
            ("index.json", index),
            ("catalog/example.json", shard.clone()),
            ("catalog/examplX.json", shard),
        ],
    );
    let bytes = std::fs::read(&bad).unwrap();
    let (from, to) = (b"catalog/examplX.json", b"catalog/example.json");
    let mut renamed = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(from) {
            renamed.extend_from_slice(to);
            i += from.len();
        } else {
            renamed.push(bytes[i]);
            i += 1;
        }
    }
    assert_ne!(renamed, bytes);
    std::fs::write(&bad, renamed).unwrap();
    let before = tree(dir.path());
    assert!(restore(dir.path(), &bad).is_err());
    assert_eq!(tree(dir.path()), before);
}

#[test]
fn snapshot_rejects_a_missing_indexed_shard() {
    let dir = common::tmp();
    catalogue(dir.path());
    std::fs::remove_file(dir.path().join("catalog/example.json")).unwrap();
    assert!(
        snapshot(dir.path())
            .unwrap_err()
            .to_string()
            .contains("catalog/example.json: missing indexed shard")
    );
    assert!(!dir.path().join("snapshots").exists());
}

#[test]
fn unsafe_or_incomplete_snapshot_never_changes_metadata() {
    for (name, payload) in [
        ("../outside.json", "{}"),
        ("catalog/../../outside.json", "{}"),
        ("catalog/C:bad.json", "{}"),
        ("index.json", r#"{"schema_version": 99}"#),
        (
            "index.json",
            r#"{"sellers": [{"id": "example", "shard": "catalog/example.json"}]}"#,
        ),
    ] {
        let dir = common::tmp();
        catalogue(dir.path());
        let before = std::fs::read(dir.path().join("index.json")).unwrap();
        let bad = dir.path().join("bad.zip");
        make_zip(&bad, &[(name, payload.as_bytes().to_vec())]);
        assert!(
            matches!(
                restore(dir.path(), &bad),
                Err(lamp::errors::Error::Value(_))
            ),
            "{name}"
        );
        assert_eq!(
            std::fs::read(dir.path().join("index.json")).unwrap(),
            before
        );
        assert!(!dir.path().join("snapshots").exists());
    }
}

#[test]
fn restore_rejects_metadata_the_viewer_cannot_use() {
    let cases: Vec<(&str, &str, Option<&str>)> = vec![
        ("shard", "seller", Some(r#""other""#)),
        ("shard", "seller", None),
        ("album", "seller", Some(r#""other""#)),
        ("album", "seller", None),
        ("album", "title", None),
        ("album", "album_url", None),
        ("album", "thumb", None),
        ("album", "links", None),
        (
            "album",
            "links",
            Some(r#"[{"url": "https://example.com/item"}]"#),
        ),
        (
            "album",
            "links",
            Some(r#"[{"platform": "shop", "url": ""}]"#),
        ),
        ("album", "price_evidence", Some(r#"{"candidates": {}}"#)),
        ("shard", "schema_version", Some("true")),
    ];
    for (target, field, value) in cases {
        let dir = common::tmp();
        catalogue(dir.path());
        let files = [
            dir.path().join("index.json"),
            dir.path().join("catalog/example.json"),
        ];
        let before: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let mut shard = read_json(&files[1]);
        let record = if target == "shard" {
            shard.as_object_mut().unwrap()
        } else {
            shard
                .as_object_mut()
                .unwrap()
                .get_mut("albums")
                .unwrap()
                .as_array_mut()
                .unwrap()[0]
                .as_object_mut()
                .unwrap()
        };
        match value {
            None => {
                record.shift_remove(field);
            }
            Some(v) => {
                record.insert(field.into(), json(v));
            }
        }
        let bad = dir.path().join("bad.zip");
        make_zip(
            &bad,
            &[
                ("index.json", before[0].clone()),
                (
                    "catalog/example.json",
                    pyjson::dumps(&shard, Style::Default).into_bytes(),
                ),
            ],
        );
        assert!(restore(dir.path(), &bad).is_err(), "{target} {field}");
        let after: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(before, after);
        assert!(!dir.path().join("snapshots").exists());
    }
}

#[test]
fn schema_rejects_duplicate_ids_unsafe_covers_and_links() {
    let dir = common::tmp();
    catalogue(dir.path());
    let shard = read_json(&dir.path().join("catalog/example.json"));
    let original = obj(&shard)["albums"].as_array().unwrap()[0].clone();
    for (key, value) in [
        ("thumb", r#""../outside.jpg""#),
        ("price", "NaN"),
        (
            "links",
            r#"[{"platform": "shop", "url": "javascript:alert(1)"}]"#,
        ),
    ] {
        let mut album = original.clone();
        album
            .as_object_mut()
            .unwrap()
            .insert(key.into(), json(value));
        let value = Value::Object(lamp::obj! { "seller" => "example", "albums" => vec![album] });
        assert!(
            validate_metadata(&value, "catalog/example.json").is_err(),
            "{key}"
        );
    }
    let value = Value::Object(
        lamp::obj! { "seller" => "example", "albums" => vec![original.clone(), original] },
    );
    assert!(validate_metadata(&value, "catalog/example.json").is_err());
    assert!(data_path(dir.path(), "runs/../index.json", false).is_err());
}

#[test]
fn cover_conversion_is_bounded_oriented_and_resumable() {
    let dir = common::tmp();
    catalogue(dir.path());
    let cover = dir.path().join("thumbs/example/123.jpg");
    let jpeg = common::jpeg_with_orientation(1600, 800, 6);
    write(&cover, &jpeg);
    let result = prepare_covers(dir.path(), 1).unwrap();
    assert!(
        matches!(result["resized"], Value::Int(1)) && matches!(result["processed"], Value::Int(1))
    );
    let after = match result["after_bytes"] {
        Value::Int(n) => n,
        _ => 0,
    };
    assert!(after < jpeg.len() as i128);
    let opened = lamp::covers::open(&cover).unwrap();
    assert_eq!(
        (opened.width, opened.height, opened.orientation),
        (320, 640, 1)
    );
    assert!(matches!(
        prepare_covers(dir.path(), 1).unwrap()["processed"],
        Value::Int(0)
    ));
    let facts = cover_facts(&cover);
    assert!(pyjson::py_eq(
        &Value::Object(facts.clone()),
        &Value::Object(cover_facts(&cover))
    ));
    assert_eq!(facts["thumb_version"].py_str().len(), 24);
    let stored = read_json(&dir.path().join("catalog/example.json"));
    assert_eq!(
        obj(&obj(&stored)["albums"].as_array().unwrap()[0])["thumb_version"].py_str(),
        facts["thumb_version"].py_str()
    );
}

#[test]
fn transparency_survives_and_failed_conversion_preserves_original() {
    let dir = common::tmp();
    let png = dir.path().join("alpha.png");
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1000,
        800,
        image::Rgba([20, 40, 60, 80]),
    ));
    write(&png, common::encode(&image, image::ImageFormat::Png));
    resize_cover(&png).unwrap();
    let decoded = image::open(&png).unwrap();
    assert!(matches!(decoded, image::DynamicImage::ImageRgba8(_)));
    assert_eq!(decoded.to_rgba8().get_pixel(0, 0).0[3], 80);
    write(&png, b"not an image");
    assert!(matches!(
        resize_cover(&png),
        Err(lamp::errors::Error::Os(_))
    ));
    assert_eq!(std::fs::read(&png).unwrap(), b"not an image");
}

#[test]
fn restore_preserves_orphaned_shards_when_index_is_missing() {
    let dir = common::tmp();
    catalogue(dir.path());
    let archive = snapshot(dir.path()).unwrap();
    std::fs::remove_file(dir.path().join("index.json")).unwrap();
    write(&dir.path().join("catalog/example.json"), "orphaned bytes");
    let previous = restore(dir.path(), &archive).unwrap().unwrap();
    assert_eq!(
        zip_member(&previous, "catalog/example.json"),
        b"orphaned bytes"
    );
}

#[test]
fn cover_fingerprints_tolerate_brightness_but_distinguish_geometry() {
    let dir = common::tmp();
    let mut base = image::RgbImage::from_pixel(256, 256, image::Rgb([255, 255, 255]));
    for y in 22..=178 {
        for x in 12..=83 {
            base.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    for y in 73..=221u32 {
        for x in 107..=236u32 {
            let (cx, cy, rx, ry) = (171.5f64, 147.0f64, 64.5f64, 74.0f64);
            if ((f64::from(x) - cx) / rx).powi(2) + ((f64::from(y) - cy) / ry).powi(2) <= 1.0 {
                base.put_pixel(x, y, image::Rgb([128, 128, 128]));
            }
        }
    }
    let darker = image::RgbImage::from_fn(256, 256, |x, y| {
        let p = base.get_pixel(x, y).0;
        image::Rgb(p.map(|v| (f64::from(v) * 0.85) as u8))
    });
    let mirrored = image::imageops::flip_horizontal(&base);
    let hashes: Vec<u64> = [base, darker, mirrored]
        .into_iter()
        .enumerate()
        .map(|(i, img)| {
            let path = dir.path().join(format!("sample{i}.png"));
            write(
                &path,
                common::encode(
                    &image::DynamicImage::ImageRgb8(img),
                    image::ImageFormat::Png,
                ),
            );
            u64::from_str_radix(&cover_facts(&path)["cover_hash"].py_str(), 16).unwrap()
        })
        .collect();
    assert!((hashes[0] ^ hashes[1]).count_ones() <= 8);
    assert!((hashes[0] ^ hashes[2]).count_ones() > 8);
}

#[test]
fn oversized_dimensions_do_not_abort_local_preparation() {
    let dir = common::tmp();
    catalogue(dir.path());
    // A PNG header claiming 9000 x 10 pixels: past the edge limit.
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend_from_slice(&9000u32.to_be_bytes());
    ihdr.extend_from_slice(&10u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(&ihdr);
    png.extend_from_slice(&crc32(&ihdr).to_be_bytes());
    for (kind, data) in [
        (
            &b"IDAT"[..],
            &[0x78u8, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01][..],
        ),
        (&b"IEND"[..], &[][..]),
    ] {
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    write(&dir.path().join("thumbs/example/123.jpg"), &png);
    let result = prepare_covers(dir.path(), 1).unwrap();
    assert!(matches!(result["resized"], Value::Int(0)));
    assert!(
        result["errors"].as_array().unwrap()[0]
            .py_str()
            .contains("cover dimensions exceed limits")
    );
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn doctor_counts_missing_descriptions_and_rejects_malformed_fields() {
    let dir = common::tmp();
    catalogue(dir.path());
    assert!(matches!(
        diagnose(dir.path())["without_description"],
        Value::Int(1)
    ));
    let mut shard = read_json(&dir.path().join("catalog/example.json"));
    {
        let album = shard
            .as_object_mut()
            .unwrap()
            .get_mut("albums")
            .unwrap()
            .as_array_mut()
            .unwrap()[0]
            .as_object_mut()
            .unwrap();
        album.insert("description".into(), "kept".into());
        album.insert("description_truncated".into(), true.into());
        album.insert("links_truncated".into(), true.into());
    }
    validate_metadata(&shard, "catalog/example.json").unwrap();
    atomic_write_json(&dir.path().join("catalog/example.json"), &shard).unwrap();
    assert!(matches!(
        diagnose(dir.path()).get("links_truncated"),
        Some(Value::Int(1))
    ));
    for (field, bad) in [
        ("description", "5"),
        ("description_truncated", r#""yes""#),
        ("links_truncated", r#""yes""#),
    ] {
        let mut broken = shard.clone();
        broken
            .as_object_mut()
            .unwrap()
            .get_mut("albums")
            .unwrap()
            .as_array_mut()
            .unwrap()[0]
            .as_object_mut()
            .unwrap()
            .insert(field.into(), json(bad));
        assert!(
            validate_metadata(&broken, "catalog/example.json").is_err(),
            "{field}"
        );
    }
}

fn with_price(root: &Path, price: &str) -> Value {
    let mut shard = read_json(&root.join("catalog/example.json"));
    shard
        .as_object_mut()
        .unwrap()
        .get_mut("albums")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .as_object_mut()
        .unwrap()
        .insert("price".into(), json(price));
    shard
}

#[test]
fn metadata_price_rules() {
    let dir = common::tmp();
    catalogue(dir.path());
    let big = format!("1{}", "0".repeat(308));
    let max_int = format!("{:.0}", f64::MAX);
    let max_int_plus = {
        let mut digits: Vec<u8> = max_int.bytes().collect();
        let last = digits.len() - 1;
        digits[last] += 1;
        String::from_utf8(digits).unwrap()
    };
    for ok in [
        "0",
        "-0.0",
        "1",
        "12.5",
        big.as_str(),
        "1.7976931348623157e308",
        max_int.as_str(),
        max_int_plus.as_str(),
    ] {
        validate_metadata(&with_price(dir.path(), ok), "catalog/example.json")
            .unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    let huge = format!("1{}", "0".repeat(400));
    for bad in [
        "true",
        "false",
        "null",
        r#""1""#,
        "[]",
        "{}",
        "-1",
        "-0.1",
        "NaN",
        "Infinity",
        "-Infinity",
        huge.as_str(),
        &format!("-{huge}"),
    ] {
        let err = validate_metadata(&with_price(dir.path(), bad), "catalog/example.json")
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid price"), "{bad}: {err}");
    }
    let mut missing = read_json(&dir.path().join("catalog/example.json"));
    missing
        .as_object_mut()
        .unwrap()
        .get_mut("albums")
        .unwrap()
        .as_array_mut()
        .unwrap()[0]
        .as_object_mut()
        .unwrap()
        .shift_remove("price");
    validate_metadata(&missing, "catalog/example.json").unwrap();
}

#[test]
fn maintenance_reports_invalid_price_without_changing_metadata() {
    let huge = format!("1{}", "0".repeat(400));
    for command in ["doctor", "snapshot", "restore"] {
        for price in [huge.as_str(), "-1"] {
            let dir = common::tmp();
            catalogue(dir.path());
            let files = [
                dir.path().join("index.json"),
                dir.path().join("catalog/example.json"),
            ];
            let shard = with_price(dir.path(), price);
            let data = dir.path().to_string_lossy().into_owned();
            let mut args = vec!["--data".to_string(), data, command.to_string()];
            if command == "restore" {
                let archive = dir.path().join("bad.zip");
                make_zip(
                    &archive,
                    &[
                        ("index.json", std::fs::read(&files[0]).unwrap()),
                        (
                            "catalog/example.json",
                            pyjson::dumps(&shard, Style::Default).into_bytes(),
                        ),
                    ],
                );
                args.push(archive.to_string_lossy().into_owned());
            } else {
                // Written as Python's json.dumps would, so large integers stay integers.
                write(&files[1], pyjson::dumps(&shard, Style::Compact));
            }
            let before: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (code, out, err) = lamp(&refs, None);
            assert_eq!(code, 1, "{command} {price}");
            assert!(format!("{out}{err}").contains("invalid price"));
            if command == "doctor" {
                assert!(!obj(&json(&out))["issues"].as_array().unwrap().is_empty());
            }
            let after: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
            assert_eq!(before, after);
            assert!(!dir.path().join("snapshots").exists());
        }
    }
}

#[test]
fn load_shard_reports_oversized_price_as_corrupt() {
    let dir = common::tmp();
    catalogue(dir.path());
    let shard = with_price(dir.path(), &format!("1{}", "0".repeat(400)));
    write(
        &dir.path().join("catalog/example.json"),
        pyjson::dumps(&shard, Style::Compact),
    );
    let err = load_shard(dir.path(), "example").unwrap_err().to_string();
    assert!(
        err.starts_with("corrupt shard") && err.contains("invalid price"),
        "{err}"
    );
}

fn several_sellers(root: &Path) {
    let sellers = ["first", "second", "third"];
    let list: Vec<String> = sellers
        .iter()
        .map(|s| format!(r#"{{"id": "{s}", "shard": "catalog/{s}.json"}}"#))
        .collect();
    atomic_write_json(
        &root.join("index.json"),
        &json(&format!(r#"{{"sellers": [{}]}}"#, list.join(","))),
    )
    .unwrap();
    for (position, seller) in sellers.iter().enumerate() {
        let mut albums = Vec::new();
        for number in 0..2 {
            let id = position * 10 + number + 1;
            let thumb = format!("thumbs/{seller}/{id}.png");
            write(&root.join(&thumb), common::png(8, 8, [255, 0, 0]));
            albums.push(format!(r#"{{"id": "{id}", "seller": "{seller}", "title": "Synthetic", "album_url": "https://{seller}.x.yupoo.com/albums/{id}", "links": [], "thumb": "{thumb}"}}"#));
        }
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

fn prepared(root: &Path, seller: &str) -> Vec<String> {
    let shard = read_json(&root.join(format!("catalog/{seller}.json")));
    obj(&shard)["albums"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            obj(a)
                .get("cover_prepared")
                .map_or("0".into(), Value::py_str)
        })
        .collect()
}

#[test]
fn cover_limit_leaves_later_shards_untouched_after_validating_the_snapshot() {
    for (limit, rewritten) in [
        (0, [false, false, false]),
        (1, [true, false, false]),
        (2, [true, false, false]),
        (3, [true, true, false]),
        (9, [true, true, true]),
    ] {
        let dir = common::tmp();
        let root = dir.path();
        several_sellers(root);
        let names = [
            "index.json",
            "catalog/first.json",
            "catalog/second.json",
            "catalog/third.json",
        ];
        let before: Vec<Vec<u8>> = names
            .iter()
            .map(|n| std::fs::read(root.join(n)).unwrap())
            .collect();
        let result = prepare_covers(root, limit).unwrap();
        assert!(pyjson::py_eq(
            &result["processed"],
            &Value::Int(limit.min(6))
        ));
        assert_eq!(result["errors"].py_str(), "[]");
        // A batch that prepares nothing changes nothing, so it has no snapshot.
        if limit == 0 {
            assert!(matches!(result["snapshot"], Value::Null));
        } else {
            let snapshot = PathBuf::from(result["snapshot"].py_str());
            for (name, bytes) in names.iter().zip(&before) {
                assert_eq!(&zip_member(&snapshot, name), bytes);
            }
        }
        let all: Vec<String> = ["first", "second", "third"]
            .iter()
            .flat_map(|s| prepared(root, s))
            .collect();
        let expected: Vec<String> = (0..6)
            .map(|i| if i < limit { "1".into() } else { "0".into() })
            .collect();
        assert_eq!(all, expected, "limit {limit}");
        assert_eq!(std::fs::read(root.join("index.json")).unwrap(), before[0]);
        for (i, changed) in rewritten.iter().enumerate() {
            if !changed {
                assert_eq!(
                    std::fs::read(root.join(names[i + 1])).unwrap(),
                    before[i + 1]
                );
            }
        }
    }
}

#[test]
fn a_cover_batch_with_nothing_left_to_prepare_writes_no_snapshot() {
    let dir = common::tmp();
    several_sellers(dir.path());
    let snapshots = || {
        std::fs::read_dir(dir.path().join("snapshots"))
            .unwrap()
            .count()
    };
    let first = prepare_covers(dir.path(), 9).unwrap();
    assert!(matches!(first["processed"], Value::Int(6)));
    assert_eq!(snapshots(), 1);
    let again = prepare_covers(dir.path(), 9).unwrap();
    assert!(matches!(again["processed"], Value::Int(0)));
    assert!(matches!(again["snapshot"], Value::Null));
    assert_eq!(snapshots(), 1);
}

#[test]
fn cover_batches_resume_across_sellers() {
    let dir = common::tmp();
    several_sellers(dir.path());
    for expected in [["1", "0"], ["1", "1"]] {
        assert!(matches!(
            prepare_covers(dir.path(), 1).unwrap()["processed"],
            Value::Int(1)
        ));
        assert_eq!(prepared(dir.path(), "first"), expected);
    }
    assert!(matches!(
        prepare_covers(dir.path(), 1).unwrap()["processed"],
        Value::Int(1)
    ));
    assert_eq!(prepared(dir.path(), "second"), ["1", "0"]);
}

#[test]
fn failed_cover_attempt_consumes_the_batch_and_remains_retryable() {
    let dir = common::tmp();
    several_sellers(dir.path());
    write(&dir.path().join("thumbs/first/1.png"), "not an image");
    let before: Vec<Vec<u8>> = ["first", "second", "third"]
        .iter()
        .map(|s| std::fs::read(dir.path().join(format!("catalog/{s}.json"))).unwrap())
        .collect();
    for _ in 0..2 {
        let result = prepare_covers(dir.path(), 1).unwrap();
        assert!(matches!(result["processed"], Value::Int(1)));
        assert_eq!(result["errors"].as_array().unwrap().len(), 1);
        let after: Vec<Vec<u8>> = ["first", "second", "third"]
            .iter()
            .map(|s| std::fs::read(dir.path().join(format!("catalog/{s}.json"))).unwrap())
            .collect();
        assert_eq!(before, after);
    }
}

#[test]
fn missing_covers_are_counted_without_using_the_batch_or_a_snapshot() {
    let dir = common::tmp();
    several_sellers(dir.path());
    for id in [1, 2] {
        std::fs::remove_file(dir.path().join(format!("thumbs/first/{id}.png"))).unwrap();
    }
    let first = prepare_covers(dir.path(), 1).unwrap();
    assert!(matches!(first["processed"], Value::Int(1)));
    assert_eq!(first["errors"].py_str(), "[]");
    assert_eq!(prepared(dir.path(), "second"), ["1", "0"]);
    assert!(matches!(first["missing_covers"], Value::Int(2)));
    prepare_covers(dir.path(), 9).unwrap();
    let again = prepare_covers(dir.path(), 9).unwrap();
    assert!(matches!(again["processed"], Value::Int(0)));
    assert!(matches!(again["missing_covers"], Value::Int(2)));
    assert_eq!(again["errors"].py_str(), "[]");
    assert!(matches!(again["snapshot"], Value::Null));
    assert_eq!(
        std::fs::read_dir(dir.path().join("snapshots"))
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn cover_limit_preserves_validation_of_later_shards() {
    let dir = common::tmp();
    several_sellers(dir.path());
    write(&dir.path().join("catalog/third.json"), "invalid");
    let before = tree(dir.path());
    assert!(prepare_covers(dir.path(), 1).is_err());
    for (path, bytes) in &before {
        assert!(
            std::fs::read(path).unwrap() == *bytes,
            "{} changed",
            path.display()
        );
    }
    assert!(!dir.path().join("snapshots").exists());
}

/// An album record for seller `seller` with cover path `thumb`.
fn album_json(id: &str, seller: &str, thumb: &str) -> String {
    format!(
        r#"{{"id": "{id}", "seller": "{seller}", "title": "Item {id}", "price": 10, "thumb": "{thumb}",
            "album_url": "https://{seller}.x.yupoo.com/albums/{id}", "links": []}}"#
    )
}

fn write_shard_json(root: &Path, seller: &str, albums: &[String]) {
    atomic_write_json(
        &root.join(format!("catalog/{seller}.json")),
        &json(&format!(
            r#"{{"seller": "{seller}", "albums": [{}]}}"#,
            albums.join(",")
        )),
    )
    .unwrap();
}

fn issues(root: &Path) -> Vec<String> {
    diagnose(root)["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::py_str)
        .collect()
}

#[test]
fn doctor_reports_an_index_that_disagrees_with_its_shards() {
    let dir = common::tmp();
    let root = dir.path();
    // `one` lists an album its shard lacks and the wrong count; `two` repeats
    // an ID `one` holds.
    atomic_write_json(
        &root.join("index.json"),
        &json(
            r#"{"sellers": [
                {"id": "one", "shard": "catalog/one.json", "album_ids": ["1", "9"], "album_count": 5},
                {"id": "two", "shard": "catalog/two.json", "album_count": 1}]}"#,
        ),
    )
    .unwrap();
    write_shard_json(root, "one", &[album_json("1", "one", "")]);
    write_shard_json(root, "two", &[album_json("1", "two", "")]);
    assert_eq!(
        issues(root),
        [
            "catalog/one.json: indexed album missing from shard",
            "catalog/one.json: index count differs from shard",
            "catalog/two.json: duplicate album ID across shards: 1",
        ]
    );

    // A selected album whose host is not canonical cannot stay selected.
    let dir = common::tmp();
    let root = dir.path();
    atomic_write_json(
        &root.join("index.json"),
        &json(
            r#"{"selection_version": 1, "sellers": [
                {"id": "three", "shard": "catalog/three.json", "album_ids": ["3"], "album_count": 1}]}"#,
        ),
    )
    .unwrap();
    let wild = album_json("3", "three", "")
        .replace("https://three.x.yupoo.com", "https://*.three.x.yupoo.com");
    write_shard_json(root, "three", &[wild]);
    assert_eq!(
        issues(root),
        ["catalog/three.json: selected album is inadmissible: 3"]
    );
    assert!(matches!(diagnose(root)["unadmitted"], Value::Int(1)));
}

#[test]
fn doctor_counts_failed_detail_pages_from_the_run_summary() {
    let dir = common::tmp();
    catalogue(dir.path());
    atomic_write_json(
        &dir.path().join("runs/example.json"),
        &json(r#"{"failed_ids": ["123", "456"]}"#),
    )
    .unwrap();
    let result = diagnose(dir.path());
    assert_eq!(result["issues"].py_str(), "[]");
    assert!(matches!(result["detail_failures"], Value::Int(2)));
}

/// A file of `len` bytes without writing them (sparse where the system allows).
fn sized(path: &Path, len: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(path).unwrap().set_len(len).unwrap();
}

#[test]
fn metadata_beyond_the_size_and_file_limits_is_refused_before_reading() {
    let dir = common::tmp();
    let root = dir.path();
    let big = root.join("catalog/big.json");
    sized(&big, lamp::maintenance::MAX_METADATA_BYTES + 1);
    let error = lamp::maintenance::read_metadata(&big, "catalog/big.json").unwrap_err();
    assert_eq!(error.to_string(), "metadata file exceeds 256 MB");
    let error = lamp::maintenance::metadata_files(root).unwrap_err();
    assert_eq!(error.to_string(), "metadata exceeds 256 MB");

    let dir = common::tmp();
    let runs = dir.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    for i in 0..=lamp::maintenance::MAX_METADATA_FILES {
        std::fs::File::create(runs.join(format!("s{i}.json"))).unwrap();
    }
    let error = lamp::maintenance::metadata_files(dir.path()).unwrap_err();
    assert_eq!(error.to_string(), "too many metadata files");
}

#[test]
fn snapshot_refuses_a_snapshots_entry_that_is_not_a_folder() {
    let dir = common::tmp();
    catalogue(dir.path());
    write(&dir.path().join("snapshots"), "not a folder");
    let error = snapshot(dir.path()).unwrap_err();
    assert!(matches!(error, lamp::errors::Error::Os(_)), "{error:?}");
    assert_eq!(common::read(&dir.path().join("snapshots")), "not a folder");
}

#[cfg(unix)]
#[test]
fn snapshot_refuses_a_snapshots_link_out_of_the_data_directory() {
    let dir = common::tmp();
    let root = dir.path().join("data");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    catalogue(&root);
    std::os::unix::fs::symlink(&outside, root.join("snapshots")).unwrap();
    let error = snapshot(&root).unwrap_err();
    assert_eq!(
        error.to_string(),
        "snapshot directory escapes data directory"
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn restore_refuses_oversized_corrupt_or_indexless_snapshots() {
    let dir = common::tmp();
    catalogue(dir.path());
    // The catalogue lock file restore takes is not metadata.
    let metadata = |root: &Path| {
        let mut files = tree(root);
        files.retain(|(path, _)| path.file_name() != Some(std::ffi::OsStr::new(".lamp.lock")));
        files
    };
    let before = metadata(dir.path());
    let index = std::fs::read(dir.path().join("index.json")).unwrap();
    let shard = std::fs::read(dir.path().join("catalog/example.json")).unwrap();
    let scratch = common::tmp();

    let many = scratch.path().join("many.zip");
    let names: Vec<String> = (0..=lamp::maintenance::MAX_METADATA_FILES)
        .map(|i| format!("runs/s{i}.json"))
        .collect();
    let members: Vec<(&str, Vec<u8>)> =
        names.iter().map(|n| (n.as_str(), b"{}".to_vec())).collect();
    make_zip(&many, &members);
    let error = restore(dir.path(), &many).unwrap_err();
    assert_eq!(error.to_string(), "snapshot exceeds recovery limits");

    let indexless = scratch.path().join("indexless.zip");
    make_zip(&indexless, &[("catalog/example.json", shard)]);
    let error = restore(dir.path(), &indexless).unwrap_err();
    assert_eq!(error.to_string(), "snapshot has no index");

    // A stored member whose bytes no longer match its CRC-32.
    let corrupt = scratch.path().join("corrupt.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&corrupt).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("index.json", stored).unwrap();
    writer.write_all(&index).unwrap();
    writer.finish().unwrap();
    let mut bytes = std::fs::read(&corrupt).unwrap();
    let at = bytes
        .windows(index.len())
        .position(|w| w == index.as_slice())
        .unwrap();
    bytes[at + index.len() - 2] ^= 0x01;
    std::fs::write(&corrupt, bytes).unwrap();
    let error = restore(dir.path(), &corrupt).unwrap_err();
    assert!(matches!(error, lamp::errors::Error::BadZip(_)), "{error:?}");
    assert_eq!(error.to_string(), "Bad CRC-32 for file 'index.json'");

    assert_eq!(before, metadata(dir.path()));
}

#[test]
fn restore_into_an_empty_data_directory_keeps_no_previous_snapshot() {
    let source = common::tmp();
    catalogue(source.path());
    let archive = snapshot(source.path()).unwrap();
    let empty = common::tmp();
    assert_eq!(restore(empty.path(), &archive).unwrap(), None);
    for name in ["index.json", "catalog/example.json"] {
        assert_eq!(
            std::fs::read(empty.path().join(name)).unwrap(),
            std::fs::read(source.path().join(name)).unwrap(),
            "{name}"
        );
    }
    assert!(!empty.path().join("snapshots").exists());
}

#[test]
fn an_oversized_cover_is_reported_and_left_unprepared() {
    let dir = common::tmp();
    let root = dir.path();
    atomic_write_json(
        &root.join("index.json"),
        &json(r#"{"sellers": [{"id": "one", "shard": "catalog/one.json", "album_count": 2}]}"#),
    )
    .unwrap();
    write_shard_json(
        root,
        "one",
        &[
            album_json("1", "one", "thumbs/one/1.jpg"),
            album_json("2", "one", ""),
        ],
    );
    sized(&root.join("thumbs/one/1.jpg"), 8 * 1024 * 1024 + 1);
    let result = prepare_covers(root, 9).unwrap();
    assert_eq!(result["errors"].py_str(), r#"["1: cover exceeds 8 MB"]"#);
    assert!(matches!(result["processed"], Value::Int(1)));
    assert!(matches!(result["missing_covers"], Value::Int(0)));
    let shard = read_json(&root.join("catalog/one.json"));
    for album in obj(&shard)["albums"].as_array().unwrap() {
        assert!(!obj(album).contains_key("cover_prepared"));
    }
}

#[test]
fn export_refuses_a_bad_since_date() {
    let dir = common::tmp();
    catalogue(dir.path());
    let error = lamp::maintenance::export_rows(dir.path(), Some("2026-13-01")).unwrap_err();
    assert_eq!(error.to_string(), "not a date: 2026-13-01");
}

#[test]
fn export_site_refuses_a_file_target_and_an_oversized_alias_list() {
    let dir = common::tmp();
    let root = dir.path().join("data");
    catalogue(&root);
    let target = dir.path().join("site");
    write(&target, "a file");
    let Err(error) = lamp::maintenance::export_site(&root, &target) else {
        panic!("a file target was accepted");
    };
    assert!(matches!(error, lamp::errors::Error::Os(_)), "{error:?}");
    assert_eq!(common::read(&target), "a file");

    sized(
        &root.join("aliases.json"),
        lamp::maintenance::MAX_METADATA_BYTES + 1,
    );
    let site = dir.path().join("site2");
    let Err(error) = lamp::maintenance::export_site(&root, &site) else {
        panic!("an oversized alias list was accepted");
    };
    assert_eq!(error.to_string(), "aliases.json exceeds 256 MB");
    assert!(!site.exists());
}

#[test]
fn export_site_without_aliases_or_covers_copies_the_metadata() {
    let dir = common::tmp();
    let root = dir.path().join("data");
    std::fs::create_dir_all(&root).unwrap();
    atomic_write_json(
        &root.join("index.json"),
        &json(r#"{"sellers": [{"id": "one", "shard": "catalog/one.json", "album_count": 1}]}"#),
    )
    .unwrap();
    write_shard_json(&root, "one", &[album_json("1", "one", "")]);
    let site = dir.path().join("site");
    let made = lamp::maintenance::export_site(&root, &site).unwrap();
    assert_eq!((made.sellers, made.covers, made.missing_covers), (1, 0, 0));
    assert_eq!(
        std::fs::read(site.join("data/catalog/one.json")).unwrap(),
        std::fs::read(root.join("catalog/one.json")).unwrap()
    );
    assert!(!site.join("data/aliases.json").exists());
}

/// The lines the snapshot and restore commands print name the files a user
/// recovers from.
#[test]
fn snapshot_and_restore_commands_print_their_paths() {
    let dir = common::tmp();
    catalogue(dir.path());
    let data = dir.path().to_string_lossy().into_owned();
    let (code, out, err) = lamp(&["--data", &data, "snapshot"], None);
    assert_eq!(code, 0, "{err}");
    let archive = out.trim_end().to_string();
    assert!(Path::new(&archive).is_file(), "{out}");
    let shard = dir.path().join("catalog/example.json");
    let original = std::fs::read(&shard).unwrap();
    write(&shard, "corrupt");
    let (code, out, err) = lamp(&["--data", &data, "restore", &archive], None);
    assert_eq!(code, 0, "{err}");
    let previous = out
        .trim_end()
        .strip_prefix("Metadata restored. Previous files saved at: ")
        .unwrap_or_else(|| panic!("{out}"));
    assert_eq!(
        zip_member(Path::new(previous), "catalog/example.json"),
        b"corrupt"
    );
    assert_eq!(std::fs::read(&shard).unwrap(), original);
}

/// `prepare-covers` prints its report and exits 1 only when a cover failed.
#[test]
fn prepare_covers_command_exits_1_only_when_a_cover_failed() {
    let dir = common::tmp();
    several_sellers(dir.path());
    let data = dir.path().to_string_lossy().into_owned();
    let (code, out, err) = lamp(&["--data", &data, "prepare-covers", "--limit", "2"], None);
    assert_eq!(code, 0, "{err}");
    let report = common::json(&out);
    assert!(matches!(report["processed"], Value::Int(2)), "{out}");
    assert_eq!(report["errors"].py_str(), "[]");
    write(&dir.path().join("thumbs/third/21.png"), "not an image");
    let (code, out, _) = lamp(&["--data", &data, "prepare-covers"], None);
    assert_eq!(code, 1, "{out}");
    let report = common::json(&out);
    assert!(matches!(report["processed"], Value::Int(4)), "{out}");
    assert_eq!(report["errors"].as_array().unwrap().len(), 1);
}
