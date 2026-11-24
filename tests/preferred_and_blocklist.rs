// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for preferred sellers and seller blocklist handling.

mod common;

use std::collections::HashSet;

use common::{json, lamp, obj, read_json, write};
use lamp::blocklist::{
    Blocklist, SourcesConfig, filter_blocked_sources, load_sources_config,
    merge_sources_with_blocklist, write_sources_config,
};
use lamp::dedup::newest_listings_with_preferred;
use lamp::ingest::{Source, Sources, load_sources, merge_sources, parse_dump, write_sources};
use lamp::pyjson::{Map, Value};
use lamp::schema::validate_metadata;
use lamp::scrape::write_scrape_index;
use lamp::storage::{Shard, reindex, write_index, write_shard};

fn album(aid: &str, seller: &str, extra: &str) -> Map {
    let mut map = obj(&json(&format!(
        r#"{{"id": "{aid}", "seller": "{seller}", "title": "Cashmere coat", "thumb": "",
            "album_url": "https://{seller}.x.yupoo.com/albums/{aid}",
            "links": [{{"platform": "weidian", "url": "https://weidian.com/item.html?itemID=42"}}]}}"#
    )))
    .clone();
    if !extra.is_empty() {
        for (k, v) in obj(&json(extra)) {
            map.insert(k.clone(), v.clone());
        }
    }
    map
}

#[test]
fn preferred_flag_round_trips_in_sources_and_rejects_non_booleans() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sources.toml");
    let mut sources = Sources::new();
    let mut fav = Source::new(
        "yupoo",
        "trusted",
        "yupoo",
        "https://trusted.x.yupoo.com",
        "Trusted Seller",
    );
    fav.preferred = true;
    let mut shop = Source::new(
        "shop",
        "taobao-favshop",
        "taobao",
        "https://favshop.taobao.com/",
        "favshop",
    );
    shop.preferred = true;
    let plain = Source::new(
        "yupoo",
        "other",
        "yupoo",
        "https://other.x.yupoo.com",
        "other",
    );
    assert_ne!(fav, plain);

    sources.insert("yupoo:trusted".into(), fav);
    sources.insert("shop:taobao-favshop".into(), shop);
    sources.insert("yupoo:other".into(), plain);
    write_sources(&path, &sources).unwrap();

    let loaded = load_sources(&path).unwrap();
    assert_eq!(loaded, sources);
    assert!(loaded["yupoo:trusted"].preferred);
    assert!(loaded["shop:taobao-favshop"].preferred);
    assert!(!loaded["yupoo:other"].preferred);

    // Re-importing the same seller preserves preferred = true.
    let (merged, report) = merge_sources(
        &loaded,
        &parse_dump("https://trusted.x.yupoo.com/\nhttps://fresh.x.yupoo.com/"),
    );
    assert_eq!(report.yupoo_known, 1);
    assert_eq!(report.yupoo_new, 1);
    assert!(merged["yupoo:trusted"].preferred);
    assert!(!merged["yupoo:fresh"].preferred);

    // Non-boolean `preferred` values are rejected on both yupoo and shops entries.
    for (body, expected) in [
        (
            "[yupoo.trusted]\nurl = \"https://trusted.x.yupoo.com\"\npreferred = \"false\"\n",
            "[yupoo.trusted].preferred must be a boolean",
        ),
        (
            "[shops.taobao-fav]\nurl = \"https://fav.taobao.com/\"\npreferred = 1\n",
            "[shops.taobao-fav].preferred must be a boolean",
        ),
    ] {
        write(&path, body);
        let err = load_sources(&path).unwrap_err().to_string();
        assert!(err.contains(expected), "expected {expected:?}, got {err:?}");
    }
}

#[test]
fn preferred_seller_dedup_ranks_live_before_preferred_and_persists_through_reindex() {
    let older_pref = album("100", "trusted", r#"{"date_published": "2026-01-01"}"#);
    let newer_other = album("200", "other", r#"{"date_published": "2026-03-01"}"#);
    let newer_pref = album("300", "trusted", r#"{"date_published": "2026-02-01"}"#);
    let pref = HashSet::from(["trusted".to_string()]);

    let (selected, aliases) =
        newest_listings_with_preferred(&[newer_other.clone(), older_pref.clone()], &pref);
    assert_eq!(
        selected
            .iter()
            .map(|a| a["id"].py_str())
            .collect::<Vec<_>>(),
        ["100"]
    );
    assert_eq!(aliases, vec![("200".into(), "100".into())]);

    // Among two live preferred listings, newest posting wins.
    let (selected2, aliases2) = newest_listings_with_preferred(
        &[older_pref.clone(), newer_other.clone(), newer_pref.clone()],
        &pref,
    );
    assert_eq!(
        selected2
            .iter()
            .map(|a| a["id"].py_str())
            .collect::<Vec<_>>(),
        ["300"]
    );
    assert_eq!(aliases2.len(), 2);

    // A gone preferred listing must not hide a live non-preferred duplicate (in either input order).
    let gone_pref = album(
        "400",
        "trusted",
        r#"{"date_published": "2026-04-01", "gone_since": "2026-11-01"}"#,
    );
    let live_other = album("200", "other", r#"{"date_published": "2026-01-15"}"#);
    assert_eq!(
        newest_listings_with_preferred(&[gone_pref.clone(), live_other.clone()], &pref)
            .0
            .iter()
            .map(|a| a["id"].py_str())
            .collect::<Vec<_>>(),
        ["200"]
    );
    assert_eq!(
        newest_listings_with_preferred(&[live_other.clone(), gone_pref.clone()], &pref)
            .0
            .iter()
            .map(|a| a["id"].py_str())
            .collect::<Vec<_>>(),
        ["200"]
    );

    // When every listing in the cluster is gone, preferred still wins as fallback.
    let gone_other = album(
        "200",
        "other",
        r#"{"date_published": "2026-05-01", "gone_since": "2026-10-01"}"#,
    );
    assert_eq!(
        newest_listings_with_preferred(&[gone_other, gone_pref], &pref)
            .0
            .iter()
            .map(|a| a["id"].py_str())
            .collect::<Vec<_>>(),
        ["400"]
    );

    // End-to-end through write_shard, write_scrape_index, write_index and reindex.
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path();
    let mut trusted_shard = Shard::new();
    trusted_shard.insert("100".into(), older_pref);
    write_shard(data_dir, "trusted", &trusted_shard).unwrap();

    let mut other_shard = Shard::new();
    other_shard.insert("200".into(), newer_other);
    write_shard(data_dir, "other", &other_shard).unwrap();

    let mut trusted_src = Source::new(
        "yupoo",
        "trusted",
        "yupoo",
        "https://trusted.x.yupoo.com",
        "trusted",
    );
    trusted_src.preferred = true;
    let other_src = Source::new(
        "yupoo",
        "other",
        "yupoo",
        "https://other.x.yupoo.com",
        "other",
    );
    let mut shop_src = Source::new(
        "shop",
        "taobao-fav",
        "taobao",
        "https://fav.taobao.com/",
        "fav",
    );
    shop_src.preferred = true;

    write_scrape_index(
        data_dir,
        &[&trusted_src, &other_src],
        &[&shop_src],
        "2026-11-24T00:00:00+00:00",
    )
    .unwrap();

    let index = read_json(&data_dir.join("index.json"));
    validate_metadata(&index, "index.json").unwrap();
    let idx_map = obj(&index);
    assert_eq!(obj(&idx_map["album_aliases"])["200"].as_str(), Some("100"));
    let sellers = idx_map["sellers"].as_array().unwrap();
    assert!(matches!(obj(&sellers[0])["preferred"], Value::Bool(true)));
    assert!(obj(&sellers[1]).get("preferred").is_none());
    let shops = idx_map["shops"].as_array().unwrap();
    assert!(matches!(obj(&shops[0])["preferred"], Value::Bool(true)));

    // reindex preserves preferred preference.
    let (indexed, dups) = reindex(data_dir).unwrap();
    assert_eq!(indexed, 1);
    assert_eq!(dups, 1);
    let rebuilt = read_json(&data_dir.join("index.json"));
    assert_eq!(
        obj(&obj(&rebuilt)["album_aliases"])["200"].as_str(),
        Some("100")
    );

    // Schema rejects non-boolean preferred field on index sellers.
    let bad = json(
        r#"{"schema_version": 1, "sellers": [{"id": "trusted", "shard": "catalog/trusted.json", "preferred": "yes"}], "shops": []}"#,
    );
    let err = validate_metadata(&bad, "index.json")
        .unwrap_err()
        .to_string();
    assert!(err.contains("preferred"), "unexpected error: {err}");

    // write_index directly with preferred: false behaves as non-preferred.
    let plain_sellers = vec![
        obj(&json(
            r#"{"id": "trusted", "shard": "catalog/trusted.json", "preferred": false}"#,
        ))
        .clone(),
        obj(&json(r#"{"id": "other", "shard": "catalog/other.json"}"#)).clone(),
    ];
    write_index(
        data_dir,
        &plain_sellers,
        &Value::Array(Vec::new()),
        "2026-11-24T00:00:00+00:00",
        true,
    )
    .unwrap();
    let plain_idx = read_json(&data_dir.join("index.json"));
    assert_eq!(
        obj(&obj(&plain_idx)["album_aliases"])["100"].as_str(),
        Some("200")
    );
}

#[test]
fn blocklist_strict_validation_alias_matching_and_write_sources_preservation() {
    let mut bl = Blocklist::new();
    assert!(bl.is_empty());
    assert!(bl.entries().is_empty());
    bl.insert_raw("ScamSeller").unwrap();
    bl.insert_raw("https://badurl.x.yupoo.com/albums").unwrap();
    bl.insert_raw("yupoo:explicitbad").unwrap();
    bl.insert_raw("shop:taobao-bad").unwrap();
    assert!(!bl.is_empty());
    assert!(bl.entries().contains("scamseller"));
    assert!(bl.entries().contains("yupoo:badurl"));
    assert!(bl.entries().contains("yupoo:explicitbad"));
    assert!(bl.entries().contains("shop:taobao-bad"));
    assert!(bl.matches_yupoo_id("SCAMSELLER"));
    assert!(bl.matches_yupoo_id("badurl"));
    assert!(!bl.matches_yupoo_id("goodseller"));

    // Alias source whose table ID differs from its URL-derived Yupoo identity is still matched.
    let alias_src = Source::new(
        "yupoo",
        "alias",
        "yupoo",
        "https://badurl.x.yupoo.com",
        "Alias Seller",
    );
    assert!(bl.matches(&alias_src));

    // Malformed entries fail closed rather than loading as silent no-ops.
    for invalid in [
        "   ",
        "yupoo: bad",
        "other:scamseller",
        "ftp://bad.x.yupoo.com",
        "bad seller",
        "yupoo:",
        "shop:bad id",
    ] {
        assert!(
            bl.insert_raw(invalid).is_err(),
            "expected rejection for {invalid:?}"
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.toml");
    assert_eq!(
        load_sources_config(&missing).unwrap(),
        SourcesConfig::default()
    );

    // Round-trip config with both [blocklist] and top-level skip_sellers, plus alias entry.
    let path = dir.path().join("sources.toml");
    write(
        &path,
        r#"
skip_sellers = ["topbad"]

[blocklist]
skip_sellers = ["scamseller", "https://urlbad.x.yupoo.com"]

[yupoo.good]
url = "https://good.x.yupoo.com"
name = "good"

[yupoo.scamseller]
url = "https://scamseller.x.yupoo.com"
name = "scamseller"

[yupoo.alias]
url = "https://urlbad.x.yupoo.com"
name = "alias"

[shops.taobao-bad]
platform = "taobao"
url = "https://bad.taobao.com/"
name = "bad"
"#,
    );
    let cfg = load_sources_config(&path).unwrap();
    assert!(cfg.blocklist.matches_yupoo_id("topbad"));
    assert!(cfg.blocklist.matches_yupoo_id("scamseller"));
    assert!(cfg.blocklist.matches_yupoo_id("urlbad"));

    let (kept, blocked_yupoo) = filter_blocked_sources(&cfg.sources, &cfg.blocklist);
    assert!(kept.contains_key("yupoo:good"));
    assert!(!kept.contains_key("yupoo:scamseller"));
    assert!(!kept.contains_key("yupoo:alias"));
    assert_eq!(
        blocked_yupoo,
        vec!["alias".to_string(), "scamseller".to_string()]
    );

    // Public load_sources + write_sources preserves existing [blocklist] on the target file.
    let loaded_only = load_sources(&path).unwrap();
    write_sources(&path, &loaded_only).unwrap();
    let after_write_sources = load_sources_config(&path).unwrap();
    assert_eq!(after_write_sources.blocklist, cfg.blocklist);

    // Shop filtering in filter_blocked_sources.
    let mut shop_bl = cfg.blocklist.clone();
    shop_bl.insert_raw("shop:taobao-bad").unwrap();
    let (kept2, blocked_yupoo2) = filter_blocked_sources(&cfg.sources, &shop_bl);
    assert!(!kept2.contains_key("shop:taobao-bad"));
    assert_eq!(
        blocked_yupoo2,
        vec!["alias".to_string(), "scamseller".to_string()]
    );

    // Write config back and reload cleanly.
    let out_path = dir.path().join("rewritten.toml");
    write_sources_config(
        &out_path,
        &SourcesConfig {
            sources: kept.clone(),
            blocklist: cfg.blocklist.clone(),
        },
    )
    .unwrap();
    let reloaded = load_sources_config(&out_path).unwrap();
    assert_eq!(reloaded.sources, kept);
    assert_eq!(reloaded.blocklist, cfg.blocklist);

    // Empty blocklist writes no [blocklist] section.
    let clean_path = dir.path().join("clean.toml");
    write_sources_config(
        &clean_path,
        &SourcesConfig {
            sources: kept.clone(),
            blocklist: Blocklist::new(),
        },
    )
    .unwrap();
    assert!(
        !std::fs::read_to_string(&clean_path)
            .unwrap()
            .contains("[blocklist]")
    );

    // Validation errors on malformed blocklist config and unreadable/invalid files.
    let bad_cases = [
        ("blocklist = \"not-a-table\"", "[blocklist] must be a table"),
        (
            "[blocklist]\nskip_sellers = \"not-an-array\"",
            "array of strings",
        ),
        (
            "[blocklist]\nskip_sellers = [123]",
            "entries must be strings",
        ),
        ("[blocklist]\nskip_sellers = [\"\"]", "non-empty strings"),
        (
            "[blocklist]\nskip_sellers = [\"yupoo: bad\"]",
            "invalid blocklist entry",
        ),
        ("invalid = [", "invalid sources file"),
    ];
    for (body, expected) in bad_cases {
        write(&path, body);
        let err = load_sources_config(&path).unwrap_err().to_string();
        assert!(
            err.contains(expected),
            "case {body:?}: expected {expected:?}, got {err:?}"
        );
        // write_sources also fails closed if target file carries malformed policy.
        assert!(write_sources(&path, &kept).is_err());
    }

    // Empty [blocklist] table without skip_sellers key is allowed.
    write(&path, "[blocklist]\n");
    assert!(load_sources_config(&path).unwrap().blocklist.is_empty());

    // Invalid UTF-8 file triggers read_text error path.
    std::fs::write(&path, [0xffu8, 0xfe]).unwrap();
    assert!(load_sources_config(&path).is_err());

    // merge_sources_with_blocklist refuses blocked URLs cleanly.
    let raws = parse_dump(
        "https://good.x.yupoo.com/\nhttps://scamseller.x.yupoo.com/\nhttps://urlbad.x.yupoo.com/",
    );
    let (merged, report) = merge_sources_with_blocklist(&Sources::new(), &raws, &cfg.blocklist);
    assert_eq!(merged.len(), 1);
    assert!(merged.contains_key("yupoo:good"));
    assert_eq!(report.yupoo_new, 1);
    assert_eq!(report.blocked.len(), 2);
}

#[test]
fn cli_import_and_scrape_refuse_blocked_and_reconcile_index() {
    let dir = tempfile::tempdir().unwrap();
    let sources = dir.path().join("sources.toml");
    let dump = dir.path().join("links.txt");
    let data = dir.path().join("data");

    write(
        &sources,
        r#"
[blocklist]
skip_sellers = ["scamseller", "yupoo:badtwo"]
"#,
    );
    write(
        &dump,
        "https://goodone.x.yupoo.com/\nhttps://scamseller.x.yupoo.com/\nhttps://badtwo.x.yupoo.com/\nhttps://scamseller.x.yupoo.com/albums\n",
    );

    let (code, stdout, stderr) = lamp(
        &[
            "--sources",
            &sources.to_string_lossy(),
            "import",
            &dump.to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("blocked: 3 refused (yupoo:badtwo, yupoo:scamseller)"),
        "stdout: {stdout}"
    );
    let rewritten = std::fs::read_to_string(&sources).unwrap();
    assert!(rewritten.contains("[yupoo.goodone]"));
    assert!(!rewritten.contains("[yupoo.scamseller]"));
    assert!(rewritten.contains("[blocklist]"));

    // Explicit `lamp scrape --seller <blocked>` refuses cleanly (direct ID and URL-derived alias).
    write(
        &sources,
        r#"
[yupoo.alias]
url = "https://scamseller.x.yupoo.com"
name = "Alias"

[shops.taobao-ok]
platform = "taobao"
url = "https://ok.taobao.com/"
name = "ok"

[blocklist]
skip_sellers = ["scamseller"]
"#,
    );
    for target in ["scamseller", "alias"] {
        let (code, _, stderr) = lamp(
            &[
                "--sources",
                &sources.to_string_lossy(),
                "--data",
                &data.to_string_lossy(),
                "scrape",
                "--seller",
                target,
                "--dry-run",
            ],
            None,
        );
        assert_eq!(code, 1);
        assert!(
            stderr.contains(&format!(
                "seller {target} is blocked by [blocklist] skip_sellers"
            )),
            "stderr: {stderr}"
        );
    }

    // Seed an existing index.json publishing the now-blocked seller.
    let mut old_shard = Shard::new();
    old_shard.insert(
        "100".into(),
        album("100", "alias", r#"{"date_published": "2026-01-01"}"#),
    );
    write_shard(&data, "alias", &old_shard).unwrap();
    let alias_src = Source::new(
        "yupoo",
        "alias",
        "yupoo",
        "https://scamseller.x.yupoo.com",
        "Alias",
    );
    let ok_shop = Source::new(
        "shop",
        "taobao-ok",
        "taobao",
        "https://ok.taobao.com/",
        "ok",
    );
    write_scrape_index(
        &data,
        &[&alias_src],
        &[&ok_shop],
        "2026-11-23T00:00:00+00:00",
    )
    .unwrap();
    let before_dry = std::fs::read_to_string(data.join("index.json")).unwrap();
    assert!(before_dry.contains("\"alias\""));

    // Dry-run when all configured Yupoo sellers are blocked writes nothing.
    let (code, stdout, stderr) = lamp(
        &[
            "--sources",
            &sources.to_string_lossy(),
            "--data",
            &data.to_string_lossy(),
            "scrape",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("1 blocked (alias)"), "stdout: {stdout}");
    assert_eq!(
        std::fs::read_to_string(data.join("index.json")).unwrap(),
        before_dry
    );

    // Non-dry-run when all configured Yupoo sellers are blocked reconciles index.json:
    // removes blocked sellers while retaining allowed shops.
    let (code, stdout, stderr) = lamp(
        &[
            "--sources",
            &sources.to_string_lossy(),
            "--data",
            &data.to_string_lossy(),
            "scrape",
        ],
        None,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("1 blocked (alias)"), "stdout: {stdout}");
    let reconciled = read_json(&data.join("index.json"));
    validate_metadata(&reconciled, "index.json").unwrap();
    let rec_map = obj(&reconciled);
    assert!(rec_map["sellers"].as_array().unwrap().is_empty());
    assert_eq!(rec_map["shops"].as_array().unwrap().len(), 1);
    assert_eq!(
        obj(&rec_map["shops"].as_array().unwrap()[0])["id"].as_str(),
        Some("taobao-ok")
    );

    // When one seller is skipped and another is blocked, scrape --dry-run succeeds and reports both.
    write(
        &sources,
        r#"
[yupoo.skipme]
url = "https://skipme.x.yupoo.com"
name = "skipme"
skip = true

[yupoo.scamseller]
url = "https://scamseller.x.yupoo.com"
name = "scamseller"

[blocklist]
skip_sellers = ["scamseller"]
"#,
    );
    let (code, stdout, stderr) = lamp(
        &[
            "--sources",
            &sources.to_string_lossy(),
            "--data",
            &data.to_string_lossy(),
            "scrape",
            "--dry-run",
        ],
        None,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("1 skipped (skipme), 1 blocked (scamseller)"),
        "stdout: {stdout}"
    );

    // Import into a missing parent directory reports write_sources_config failure cleanly.
    let unwritable = dir.path().join("no-such-dir").join("sources.toml");
    let (code, _, stderr) = lamp(
        &[
            "--sources",
            &unwritable.to_string_lossy(),
            "import",
            &dump.to_string_lossy(),
        ],
        None,
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("error:"));
}
