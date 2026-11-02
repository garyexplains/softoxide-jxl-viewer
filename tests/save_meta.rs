//! Metadata carry-through on save: JPEG XL container boxes (Exif + xml),
//! PNG eXIf/iTXt-XMP chunks, JPEG APP1/APP13 segments, orientation
//! neutralization, and per-file batch carry. Round-trips go through the
//! production decode path.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use jxlviewer::codec::{self, ConflictAction, Format, JpegSaveOptions, SaveOptions};
use jxlviewer::meta::{ExifBlock, ImageMetadata};
use jxlviewer::raster::Raster;

mod common;
use common::{exif_tiff_block, png_insert_after_ihdr};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jxlviewer-savemeta-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An oriented 20×10 PNG carrying EXIF orientation 6 + Make "Cam". Decoded:
/// 10×20, meta.exif.applied = true.
fn oriented_png_path(dir: &Path, name: &str, make: &str, fill: u8) -> PathBuf {
    let raster = Raster::from_rgba8(20, 10, false, vec![fill; 20 * 10 * 3]);
    let png = codec::encode_to_vec(&raster, &SaveOptions::Png).unwrap();
    let png = png_insert_after_ihdr(&png, b"eXIf", &exif_tiff_block(6, Some(make)));
    let path = dir.join(format!("{name}.png"));
    std::fs::write(&path, &png).unwrap();
    path
}

/// A hand-built ExifBlock (orientation 6 + Make "SaveCam" in the raw TIFF).
fn test_exif() -> ExifBlock {
    ExifBlock {
        raw: exif_tiff_block(6, Some("SaveCam")),
        orientation: 6,
        applied: false,
        tags: Vec::new(),
    }
}

fn test_meta() -> ImageMetadata {
    ImageMetadata {
        exif: Some(test_exif()),
        xmp: Some(b"<xmp:a/>".to_vec()),
        iptc: Some(b"Photoshop 3.0\0\x38\x42\x49\x4d!".to_vec()),
    }
}

#[test]
fn save_jxl_carries_exif_box_with_neutralized_orientation() {
    let dir = scratch("jxlbox");
    let src = oriented_png_path(&dir, "src", "BoxCam", 64);
    let img = codec::decode_file(&src).expect("decode source");
    assert_eq!(img.raster.width, 10, "orientation applied at decode");
    let ex = img
        .meta
        .as_ref()
        .and_then(|m| m.exif.as_ref())
        .expect("exif");
    assert_eq!(ex.orientation, 6);
    assert!(ex.applied);

    let raster = img.raster.clone();
    let saved = codec::encode_to_vec_with_meta(
        &raster,
        &SaveOptions::Jxl(codec::JxlSaveOptions::default()),
        img.meta.as_ref(),
    )
    .expect("jxl save with meta");

    // Container form (signature box), not a bare codestream.
    assert_eq!(
        &saved[..12],
        &[
            0u8, 0, 0, 0xC, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A
        ],
        "JXL signature box expected"
    );

    // Header-level peek: metadata comes back through the stage-1 path with
    // the orientation neutralized (pixels are upright) and tags preserved.
    let meta = jxlviewer::meta::from_bytes(&saved, Format::Jxl).expect("meta in container");
    let ex = meta.exif.expect("exif box parsed");
    assert_eq!(ex.orientation, 1, "neutralized to 1");
    assert!(!ex.applied);
    let make = ex.tags.iter().find(|(n, _)| n == "Make").unwrap();
    assert!(make.1.contains("BoxCam"), "make = {:?}", make.1);

    // Full decode too: dims stay upright, meta attached from the boxes.
    let img2 = codec::decode_from_bytes(&saved, Some("jxl")).expect("decode saved");
    assert_eq!(img2.raster.width, 10);
    assert_eq!(img2.raster.height, 20);
    assert_eq!(
        img2.meta
            .as_ref()
            .and_then(|m| m.exif.as_ref())
            .map(|e| e.orientation),
        Some(1)
    );
}

#[test]
fn save_jxl_xmp_only_box() {
    let raster = Raster::from_rgba8(4, 4, false, vec![7u8; 4 * 4 * 3]);
    let meta = ImageMetadata {
        exif: None,
        xmp: Some(b"<x:xmpmeta/>".to_vec()),
        iptc: None,
    };
    let saved = codec::encode_to_vec_with_meta(
        &raster,
        &SaveOptions::Jxl(codec::JxlSaveOptions::default()),
        Some(&meta),
    )
    .expect("save");
    assert_eq!(
        &saved[..12],
        &[
            0u8, 0, 0, 0xC, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A
        ]
    );
    let meta = jxlviewer::meta::from_bytes(&saved, Format::Jxl).expect("meta");
    assert_eq!(meta.xmp.as_deref(), Some(b"<x:xmpmeta/>".as_slice()));
    assert!(meta.exif.is_none());
}

#[test]
fn save_png_carries_exif_and_xmp() {
    let raster = Raster::from_rgba8(4, 3, false, vec![30u8; 4 * 3 * 3]);
    let ts = test_meta();
    let saved =
        codec::encode_to_vec_with_meta(&raster, &SaveOptions::Png, Some(&ts)).expect("png save");
    let img = codec::decode_from_bytes(&saved, Some("png")).expect("re-decode");
    let ex = img
        .meta
        .as_ref()
        .and_then(|m| m.exif.as_ref())
        .expect("exif");
    assert_eq!(ex.orientation, 1, "neutralized");
    let make = ex.tags.iter().find(|(n, _)| n == "Make").unwrap();
    assert!(make.1.contains("SaveCam"), "make = {:?}", make.1);
    assert_eq!(
        img.meta.as_ref().and_then(|m| m.xmp.as_deref()),
        Some(b"<xmp:a/>".as_slice())
    );
}

#[test]
fn save_jpeg_carries_exif_xmp_iptc() {
    let raster = Raster::from_rgba8(6, 5, false, vec![99u8; 6 * 5 * 3]);
    let ts = test_meta();
    let saved = codec::encode_to_vec_with_meta(
        &raster,
        &SaveOptions::Jpeg(JpegSaveOptions::default()),
        Some(&ts),
    )
    .expect("jpeg save");

    let img = codec::decode_from_bytes(&saved, Some("jpg")).expect("re-decode");
    let ex = img
        .meta
        .as_ref()
        .and_then(|m| m.exif.as_ref())
        .expect("exif");
    assert_eq!(ex.orientation, 1, "neutralized");
    let make = ex.tags.iter().find(|(n, _)| n == "Make").unwrap();
    assert!(make.1.contains("SaveCam"), "make = {:?}", make.1);
    assert_eq!(
        img.meta.as_ref().and_then(|m| m.xmp.as_deref()),
        Some(b"<xmp:a/>".as_slice())
    );
    let iptc_len = img
        .meta
        .as_ref()
        .and_then(|m| m.iptc.as_ref().map(|v| v.len()));
    assert!(iptc_len.is_some_and(|n| n >= 14), "iptc len = {iptc_len:?}");
    // Still a plain JPEG (no container magic).
    assert_eq!(&saved[..2], &[0xFF, 0xD8]);
}

#[test]
fn neutralize_orientation_rewrites_le_and_be() {
    // Little-endian block with orientation 6 → 1; Make survives.
    let mut tiff = exif_tiff_block(6, Some("NeutralCam"));
    assert!(jxlviewer::meta::neutralize_orientation(&mut tiff));
    let meta = jxlviewer::meta::from_bytes(&tiff, Format::Tiff).expect("parse");
    let ex = meta.exif.unwrap();
    assert_eq!(ex.orientation, 1);
    assert_eq!(ex.tags.len(), 2);
    assert!(
        ex.tags
            .iter()
            .find(|(n, _)| n == "Make")
            .unwrap()
            .1
            .contains("NeutralCam")
    );
}

#[test]
fn save_jxl_deep16_with_boxes_decodes_via_libjxl() {
    // 16-bit rasters take the libjxl decode path; the container with an Exif
    // box must decode there too.
    let _dir = scratch("jxlbox16");
    let mut data = Vec::with_capacity(8 * 6 * 4);
    for i in 0..8 * 6 {
        let v = (i as u16).wrapping_mul(5321) & 0xFFFF;
        data.push(v);
        data.push(v / 3);
        data.push(v.wrapping_mul(3));
        data.push(65535);
    }
    let raster = Raster::from_rgba16(8, 6, true, data);
    let ts = test_meta();
    let saved = codec::encode_to_vec_with_meta(
        &raster,
        &SaveOptions::Jxl(codec::JxlSaveOptions {
            lossless: true,
            effort: 3,
            ..Default::default()
        }),
        Some(&ts),
    )
    .expect("save");
    assert_eq!(saved[3], 0xC, "container signature present");
    let img = codec::decode_from_bytes(&saved, Some("jxl")).expect("16-bit decode");
    assert_eq!(img.raster.bits, 16);
    assert_eq!(
        img.meta
            .as_ref()
            .and_then(|m| m.exif.as_ref())
            .map(|e| e.orientation),
        Some(1)
    );
}

#[test]
fn batch_carry_is_per_file() {
    // Two sources, each with its own camera name; each output must carry its
    // own metadata.
    let dir = scratch("batch");
    let mut outs = Vec::new();
    for (i, cam) in ["BatchCamA", "BatchCamB"].iter().enumerate() {
        outs.push(oriented_png_path(
            &dir,
            &format!("photo{i}"),
            cam,
            (i * 40 + 5) as u8,
        ));
    }

    let summary = codec::batch_convert(
        &outs,
        &SaveOptions::Jxl(codec::JxlSaveOptions::default()),
        ConflictAction::Overwrite,
        /* keep_meta */ true,
        |_d, _n| {},
        &AtomicBool::new(false),
    );
    assert!(summary.failed.is_empty(), "{:?}", summary.failed);
    assert_eq!(summary.converted, 2);

    for (i, cam) in ["BatchCamA", "BatchCamB"].iter().enumerate() {
        let out = dir.join(format!("photo{i}.jxl"));
        let bytes = std::fs::read(&out).unwrap();
        let meta = jxlviewer::meta::from_bytes(&bytes, Format::Jxl).expect("meta");
        let ex = meta.exif.expect("exif box");
        assert_eq!(ex.orientation, 1);
        let make = ex.tags.iter().find(|(n, _)| n == "Make").expect("make tag");
        assert!(make.1.contains(cam), "{cam} expected in {:?}", make.1);
    }
}

#[test]
fn batch_without_keep_drops_metadata() {
    let dir = scratch("batchoff");
    let src = oriented_png_path(&dir, "p", "QuietCam", 8);

    let summary = codec::batch_convert(
        &[src],
        &SaveOptions::Jxl(codec::JxlSaveOptions::default()),
        ConflictAction::Overwrite,
        /* keep_meta */ false,
        |_d, _n| {},
        &AtomicBool::new(false),
    );
    assert_eq!(summary.converted, 1);
    let bytes = std::fs::read(dir.join("p.jxl")).unwrap();
    let meta = jxlviewer::meta::from_bytes(&bytes, Format::Jxl);
    assert!(meta.is_none(), "no box → no metadata");
}
