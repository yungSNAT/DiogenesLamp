// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: Apache-2.0

//! Cover fingerprints and preparation: an image hashes the same whatever its
//! EXIF orientation or pixel format, and a cover that cannot be fingerprinted
//! or prepared says why.
mod common;

use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use lamp::covers::{COVER_EDGE, cover_facts, image_hash, open, resize_cover};

/// 9 columns by 8 rows of greys; no two horizontal neighbours are equal and
/// no flip or rotation maps the table onto itself.
const TABLE: [[u8; 9]; 8] = [
    [10, 145, 55, 235, 100, 190, 10, 145, 55],
    [190, 55, 235, 10, 145, 100, 235, 55, 190],
    [100, 235, 10, 190, 55, 145, 100, 10, 235],
    [55, 10, 145, 100, 235, 10, 190, 145, 100],
    [235, 100, 190, 55, 10, 235, 55, 100, 10],
    [145, 190, 100, 145, 190, 55, 145, 235, 145],
    [10, 55, 235, 100, 55, 190, 10, 100, 55],
    [190, 145, 55, 10, 235, 100, 235, 190, 10],
];

/// The difference hash of `TABLE` itself: a bit per left-right pair, set
/// where the left grey is brighter.
fn table_hash() -> String {
    let mut bits = 0u64;
    for row in TABLE {
        for col in 0..8 {
            bits = (bits << 1) | u64::from(row[col] > row[col + 1]);
        }
    }
    format!("{bits:016x}")
}

/// `TABLE` with each cell `cell` pixels wide and `rows` pixels high.
fn upright(cell: u32, rows: u32) -> GrayImage {
    GrayImage::from_fn(9 * cell, 8 * rows, |x, y| {
        Luma([TABLE[(y / rows) as usize][(x / cell) as usize]])
    })
}

fn save(path: &Path, bytes: &[u8]) -> std::path::PathBuf {
    common::write(path, bytes);
    path.to_path_buf()
}

fn hash_of(dir: &Path, name: &str, bytes: &[u8]) -> String {
    image_hash(&save(&dir.join(name), bytes)).unwrap()
}

fn jpeg(image: &DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    image
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out, 100,
        ))
        .unwrap();
    out
}

fn inverse(orientation: Orientation) -> Orientation {
    match orientation {
        Orientation::Rotate90 => Orientation::Rotate270,
        Orientation::Rotate270 => Orientation::Rotate90,
        other => other,
    }
}

#[test]
fn an_exact_nine_by_eight_image_hashes_its_own_comparisons() {
    let dir = common::tmp();
    let image = DynamicImage::ImageLuma8(upright(1, 1));
    let expected = table_hash();
    assert_eq!(
        hash_of(
            dir.path(),
            "a.png",
            &common::encode(&image, ImageFormat::Png)
        ),
        expected
    );
    // Only the width needs resampling here.
    let wide = DynamicImage::ImageLuma8(upright(2, 1));
    assert_eq!(
        hash_of(
            dir.path(),
            "b.png",
            &common::encode(&wide, ImageFormat::Png)
        ),
        expected
    );
    // Only the height: Pillow's LANCZOS gives the same comparisons.
    let tall = DynamicImage::ImageLuma8(upright(1, 2));
    assert_eq!(
        hash_of(
            dir.path(),
            "c.png",
            &common::encode(&tall, ImageFormat::Png)
        ),
        expected
    );
}

#[test]
fn every_exif_orientation_hashes_and_prepares_as_the_upright_image() {
    let dir = common::tmp();
    let upright = DynamicImage::ImageLuma8(upright(1, 1));
    let expected = hash_of(
        dir.path(),
        "upright.png",
        &common::encode(&upright, ImageFormat::Png),
    );
    assert_eq!(expected, table_hash());
    for tag in 1..=8u8 {
        let orientation = Orientation::from_exif(tag).unwrap();
        let mut stored = upright.clone();
        stored.apply_orientation(inverse(orientation));
        let bytes = common::with_orientation(&jpeg(&stored), u16::from(tag));
        let path = save(&dir.path().join(format!("{tag}.jpg")), &bytes);
        assert_eq!(open(&path).unwrap().orientation, tag);
        assert_eq!(image_hash(&path).unwrap(), expected, "orientation {tag}");
        let facts = cover_facts(&path);
        assert_eq!(facts["cover_hash"].py_str(), expected, "orientation {tag}");

        assert_eq!(resize_cover(&path).unwrap(), tag != 1, "orientation {tag}");
        let prepared = open(&path).unwrap();
        assert_eq!(
            (prepared.width, prepared.height, prepared.orientation),
            (9, 8, 1),
            "orientation {tag}"
        );
        assert_eq!(image_hash(&path).unwrap(), expected, "orientation {tag}");
    }
}

#[test]
fn every_pixel_format_hashes_as_its_greys() {
    let dir = common::tmp();
    let grey = upright(1, 1);
    let expected = table_hash();
    let wide = |v: u8| u16::from(v) * 257;
    let images = [
        DynamicImage::ImageLuma8(grey.clone()),
        DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(9, 8, |x, y| {
            Luma([u16::from(grey.get_pixel(x, y).0[0])])
        })),
        DynamicImage::ImageLumaA8(image::ImageBuffer::from_fn(9, 8, |x, y| {
            image::LumaA([grey.get_pixel(x, y).0[0], 255])
        })),
        DynamicImage::ImageLumaA16(image::ImageBuffer::from_fn(9, 8, |x, y| {
            image::LumaA([wide(grey.get_pixel(x, y).0[0]), 65535])
        })),
        DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(9, 8, |x, y| {
            let v = grey.get_pixel(x, y).0[0];
            image::Rgb([v, v, v])
        })),
        DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(9, 8, |x, y| {
            let v = grey.get_pixel(x, y).0[0];
            image::Rgba([v, v, v, 255])
        })),
        DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(9, 8, |x, y| {
            let v = wide(grey.get_pixel(x, y).0[0]);
            image::Rgb([v, v, v])
        })),
        DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(9, 8, |x, y| {
            let v = wide(grey.get_pixel(x, y).0[0]);
            image::Rgba([v, v, v, 65535])
        })),
    ];
    for (i, image) in images.iter().enumerate() {
        let name = format!("{i}.png");
        let bytes = common::encode(image, ImageFormat::Png);
        assert_eq!(
            hash_of(dir.path(), &name, &bytes),
            expected,
            "{:?}",
            image.color()
        );
    }
}

#[test]
fn files_that_cannot_be_fingerprinted_say_why() {
    let dir = common::tmp();
    let error = image_hash(dir.path()).unwrap_err();
    assert_eq!(error, "not a file");
    let wide = DynamicImage::ImageLuma8(GrayImage::new(8193, 1));
    let path = save(
        &dir.path().join("wide.png"),
        &common::encode(&wide, ImageFormat::Png),
    );
    assert_eq!(
        image_hash(&path).unwrap_err(),
        "over 20 megapixels or 8192 pixels on an edge"
    );
    // The digest still records the bytes; the fingerprint stays empty.
    assert_eq!(cover_facts(&path)["cover_hash"].py_str(), "");
    assert_eq!(cover_facts(&path)["thumb_version"].py_str().len(), 24);

    let text = save(&dir.path().join("note.jpg"), b"not an image");
    let facts = cover_facts(&text);
    assert_eq!(facts["thumb_version"].py_str().len(), 24);
    assert_eq!(facts["cover_hash"].py_str(), "");
    assert!(cover_facts(&dir.path().join("missing.jpg")).is_empty());
    assert!(cover_facts(dir.path()).is_empty());

    // A header that opens but pixel data that does not decode.
    let whole = common::encode(&DynamicImage::ImageLuma8(upright(4, 4)), ImageFormat::Png);
    let cut = save(&dir.path().join("cut.png"), &whole[..whole.len() * 3 / 5]);
    assert!(open(&cut).is_ok());
    let facts = cover_facts(&cut);
    assert_eq!(facts["thumb_version"].py_str().len(), 24);
    assert_eq!(facts["cover_hash"].py_str(), "");
}

// A file that cannot be read has no facts at all.
#[cfg(unix)]
#[test]
fn unreadable_covers_have_no_facts() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::tmp();
    let path = save(
        &dir.path().join("locked.png"),
        &common::png(9, 8, [1, 2, 3]),
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        return; // root reads it regardless
    }
    assert!(cover_facts(&path).is_empty());
}

#[test]
fn preparation_refuses_unsupported_formats_and_shrinks_extreme_shapes() {
    let dir = common::tmp();
    let gif = save(
        &dir.path().join("a.gif"),
        b"GIF89a\x01\x00\x01\x00\x00\x00\x00;",
    );
    let before = std::fs::read(&gif).unwrap();
    assert_eq!(
        resize_cover(&gif).unwrap_err().to_string(),
        "unsupported cover format"
    );
    assert_eq!(std::fs::read(&gif).unwrap(), before);

    // A thin strip keeps at least one row.
    let strip = DynamicImage::ImageLuma8(GrayImage::new(8192, 2));
    let path = save(
        &dir.path().join("strip.png"),
        &common::encode(&strip, ImageFormat::Png),
    );
    assert!(resize_cover(&path).unwrap());
    let opened = open(&path).unwrap();
    assert_eq!((opened.width, opened.height), (COVER_EDGE, 1));

    // A large WebP stays WebP.
    let photo = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        2 * COVER_EDGE,
        COVER_EDGE,
        image::Rgb([20, 120, 200]),
    ));
    let path = save(
        &dir.path().join("big.webp"),
        &common::encode(&photo, ImageFormat::WebP),
    );
    assert!(resize_cover(&path).unwrap());
    let opened = open(&path).unwrap();
    assert_eq!(opened.format_name(), "WEBP");
    assert_eq!((opened.width, opened.height), (COVER_EDGE, COVER_EDGE / 2));
}

/// A cover another program holds open keeps its bytes, and the shrunk copy
/// that could not replace it is removed.
#[cfg(windows)]
#[test]
fn a_cover_held_open_is_left_as_it_was() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = common::tmp();
    let photo = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        2 * COVER_EDGE,
        COVER_EDGE,
        image::Rgb([20, 120, 200]),
    ));
    let path = save(
        &dir.path().join("big.png"),
        &common::encode(&photo, ImageFormat::Png),
    );
    let before = std::fs::read(&path).unwrap();
    // Shared for reading only (FILE_SHARE_READ), so a rename over it fails.
    let _held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    assert!(matches!(
        resize_cover(&path),
        Err(lamp::errors::Error::Os(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
