// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Synthetic catalogue data for `lamp demo` and viewer development.
//!
//! Two albums in every 3 get a cover, spread across each seller rather than
//! sitting on its first albums. The rest are left cover-less deliberately:
//! Yupoo has no cover image for some albums, and the viewer draws its titled
//! colour block for those. Spread rather than prefixed because the viewer
//! orders by posting date, which is unrelated to position.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::isodate::Date;
use crate::pyjson::{self, Style, Value};

const WORDS: [&str; 16] = [
    "alpha", "bravo", "canvas", "denim", "echo", "fleece", "grid", "hood", "indigo", "jacket",
    "knit", "lunar", "mesh", "nylon", "orbit", "polo",
];
const UNCOVERED_EVERY: usize = 3;
const COVER_PX: u32 = 240;
const HUE_BUCKETS: u32 = 64;

/// splitmix64: a small seeded generator, so a dataset is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
}

fn has_cover(index: usize) -> bool {
    !index.is_multiple_of(UNCOVERED_EVERY)
}

fn hsv_to_rgb(hue: f64, sat: f64, val: f64) -> [u8; 3] {
    let sector = (hue * 6.0) as i64 % 6;
    let offset = hue * 6.0 - (hue * 6.0).floor();
    let (p, q, t) = (
        val * (1.0 - sat),
        val * (1.0 - sat * offset),
        val * (1.0 - sat * (1.0 - offset)),
    );
    let (r, g, b) = [
        (val, t, p),
        (q, val, p),
        (p, val, t),
        (p, q, val),
        (t, p, val),
        (val, p, q),
    ][sector as usize];
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

/// A diagonal 2-tone gradient, hue keyed on the album id, cached per bucket.
fn cover_bytes(seed: u64, cache: &mut HashMap<u32, Vec<u8>>) -> io::Result<Vec<u8>> {
    let bucket = (((seed as f64) * 0.618_033_988_75) % 1.0 * f64::from(HUE_BUCKETS)) as u32;
    if let Some(bytes) = cache.get(&bucket) {
        return Ok(bytes.clone());
    }
    let hue = f64::from(bucket) / f64::from(HUE_BUCKETS);
    let top = hsv_to_rgb(hue, 0.45, 0.85);
    let bottom = hsv_to_rgb((hue + 0.08) % 1.0, 0.60, 0.45);
    let image = image::RgbImage::from_fn(COVER_PX, COVER_PX, |x, y| {
        let ratio = (f64::from(y) + f64::from(x) * 0.35) / (f64::from(COVER_PX) * 1.35);
        image::Rgb([0, 1, 2].map(|i| {
            (f64::from(top[i]) + (f64::from(bottom[i]) - f64::from(top[i])) * ratio) as u8
        }))
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(io::Error::other)?;
    cache.insert(bucket, out.clone());
    Ok(out)
}

fn json(text: &str) -> Value {
    pyjson::loads(text).expect("static JSON")
}

/// Today, so posting dates spread across the last 3 years and the age
/// filter has something to cut.
fn today() -> Date {
    use chrono::Datelike;
    let now = chrono::Utc::now().date_naive();
    Date::from_ymd(
        i64::from(now.year()),
        i64::from(now.month()),
        i64::from(now.day()),
    )
    .expect("today is a valid date")
}

/// One synthetic album record.
fn album(seller: &str, j: usize, aid: &str, covered: bool, today: Date, rng: &mut Rng) -> Value {
    let run_ts = RUN_TS;
    let mut words: Vec<&str> = WORDS.to_vec();
    let title: Vec<&str> = (0..3)
        .map(|_| words.remove(rng.below(words.len())))
        .collect();
    let title = format!("{} {aid}", title.join(" "));
    let mut links = Vec::new();
    if j.is_multiple_of(3) {
        links.push(json(&format!(
            r#"{{"platform": "weidian", "url": "https://weidian.com/item.html?itemID={j}"}}"#
        )));
    }
    if j.is_multiple_of(4) {
        links.push(json(&format!(
            r#"{{"platform": "taobao", "url": "https://item.taobao.com/item.htm?id={j}"}}"#
        )));
    }
    let age_days = rng.range(0, 1095);
    let changed = j % 6 == 3;
    let history = if changed {
        json(
            r#"[{"at": "2026-09-25T12:00:00+10:00", "field": "price", "before": 220, "after": 180}]"#,
        )
    } else {
        json("[]")
    };
    let evidence = if changed {
        json(
            r#"{"amount": 180, "token": "CNY180", "rule": "currency", "field": "description", "candidates": [180]}"#,
        )
    } else {
        json("{}")
    };
    Value::Object(crate::obj! {
        "id" => aid,
        "seller" => seller,
        "title" => title.as_str(),
        "list_title" => title.as_str(),
        "thumb" => if covered { format!("thumbs/{seller}/{aid}.png") } else { String::new() },
        "album_url" => format!("https://{seller}.x.yupoo.com/albums/{aid}?uid=1"),
        "links" => links,
        "category" => ["hoodies", "tees", "shoes", ""][rng.below(4)],
        "price" => [0i64, 89, 128, 180, 280, 499][j % 6],
        "last_seen" => run_ts,
        "gone_since" => if j.is_multiple_of(97) { "2026-09-20T12:00:00+10:00" } else { "" },
        "history" => history,
        "price_evidence" => evidence,
        "photo_count" => rng.range(3, 40),
        "date_published" => Date { ordinal: today.ordinal - age_days }.isoformat(),
        "date_modified" => Date { ordinal: today.ordinal - age_days / 2 }.isoformat(),
        "first_seen" => if j.is_multiple_of(10) { run_ts } else { "2026-08-01T00:00:00+10:00" },
        "updated_at" => run_ts,
    })
}

const RUN_TS: &str = "2026-09-26T12:00:00+10:00";

/// Write `sellers` x `albums` synthetic albums under `data_dir`, with
/// generated covers on 2 albums in every 3 among each seller's first
/// `cover_limit`. Fixtures include known and unknown prices, changed prices
/// and gone listings. Returns the number of covers written.
pub fn write(
    data_dir: &Path,
    sellers: usize,
    albums: usize,
    cover_limit: usize,
) -> io::Result<usize> {
    let mut rng = Rng(42);
    let mut cache = HashMap::new();
    let today = today();
    std::fs::create_dir_all(data_dir.join("catalog"))?;
    let mut entries = Vec::new();
    let mut covers = 0;
    for i in 0..sellers {
        let seller = format!("demoseller{i}");
        let thumb_dir = data_dir.join("thumbs").join(&seller);
        std::fs::create_dir_all(&thumb_dir)?;
        let mut records = Vec::new();
        for j in 0..albums {
            // Globally unique ids, like real Yupoo ids.
            let aid = ((i + 1) * 10_000_000 + j).to_string();
            let covered = has_cover(j) && j < cover_limit;
            if covered {
                let seed = aid.parse().unwrap_or(0);
                std::fs::write(
                    thumb_dir.join(format!("{aid}.png")),
                    cover_bytes(seed, &mut cache)?,
                )?;
                covers += 1;
            }
            records.push(album(&seller, j, &aid, covered, today, &mut rng));
        }
        let shard = crate::obj! { "seller" => seller.as_str(), "albums" => records };
        std::fs::write(
            data_dir.join("catalog").join(format!("{seller}.json")),
            pyjson::dumps(&Value::Object(shard), Style::Default),
        )?;
        entries.push(Value::Object(crate::obj! {
            "id" => seller.as_str(),
            "name" => seller.as_str(),
            "url" => format!("https://{seller}.x.yupoo.com"),
            "album_count" => albums,
            "shard" => format!("catalog/{seller}.json"),
        }));
    }
    let index = crate::obj! {
        "generated_at" => RUN_TS,
        "sellers" => entries,
        "shops" => json(r#"[{"id": "weidian-1", "platform": "weidian", "name": "demo shop", "url": "https://weidian.com/?userid=1"}]"#),
    };
    std::fs::write(
        data_dir.join("index.json"),
        pyjson::dumps(&Value::Object(index), Style::Default),
    )?;
    Ok(covers)
}
