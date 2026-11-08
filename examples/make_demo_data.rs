// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Write synthetic catalogue data to a directory you keep, for viewer
//! development and the browser benchmark. `lamp demo` serves the same data
//! from a temporary directory instead.
//!
//! Usage: cargo run --example make_demo_data -- [data_dir] [sellers] [albums_per_seller] [cover_limit_per_seller]
//! Then:  lamp --data <data_dir> serve --no-browser

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = PathBuf::from(args.first().map_or("demo-data", String::as_str));
    let n_sellers: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let n_albums: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(2000);
    let cover_limit: usize = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(n_albums);
    let covers =
        lamp::demo::write(&data_dir, n_sellers, n_albums, cover_limit).expect("write demo data");
    println!(
        "wrote {n_sellers} sellers x {n_albums} albums ({covers} covers) to {}/",
        data_dir.display()
    );
}
