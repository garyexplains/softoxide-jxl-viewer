//! End-to-end codec tests. These exercise the vendored libjxl encoder +
//! pure-Rust decoders: PNG → JXL (lossless) → pixels must round-trip exactly,
//! and PNG → JXL (lossy tiles) must still decode.

use jxlviewer::codec::{self, JxlSaveOptions, SaveOptions};
use jxlviewer::raster::Raster;

fn make_test_rgba(w: u32, h: u32) -> Raster {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push((x * 3 % 256) as u8);
            data.push((y * 5 % 256) as u8);
            data.push(((x + y) * 7 % 256) as u8);
            data.push(if x % 4 == 0 { 128 } else { 255 });
        }
    }
    Raster::from_rgba8(w, h, true, data)
}

fn make_test_rgb(w: u32, h: u32) -> Raster {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push((x * 3 % 256) as u8);
            data.push((y * 5 % 256) as u8);
            data.push((x * y % 256) as u8);
        }
    }
    Raster::from_rgba8(w, h, false, data)
}

#[test]
fn png_to_jxl_lossless_roundtrip() {
    let src = make_test_rgba(97, 61);
    let jxl = codec::encode_jxl(
        &src,
        &JxlSaveOptions {
            lossless: true,
            effort: 3,
            ..Default::default()
        },
    )
    .expect("jxl encode should work");
    assert!(jxl[..2] == [0x_FF, 0x0A], "bare jxl codestream signature");

    let loaded = codec::decode_from_bytes(&jxl, Some("jxl")).expect("jxl decode");
    assert_eq!(loaded.format, codec::Format::Jxl);
    let out = &loaded.raster;

    assert_eq!(out.width, src.width);
    assert_eq!(out.height, src.height);
    assert_eq!(out.bits, src.bits);
    assert_eq!(out.has_alpha, true);
    // lossless → identical pixels
    assert_eq!(out.to_rgba8_rgba(), src.to_rgba8_rgba());
}

#[test]
fn rgb_jxl_lossless_with_tiles() {
    let src = make_test_rgb(256, 192);
    let enc = codec::encode_jxl(
        &src,
        &JxlSaveOptions {
            lossless: true,
            modular: true,
            group_size: Some(jxl_native::GroupSize::G256),
            effort: 4,
            ..Default::default()
        },
    )
    .expect("encode with tiles");
    let decoded = codec::decode_from_bytes(&enc, Some("jxl")).expect("decode");
    assert_eq!(decoded.raster.width, src.width);
    assert_eq!(decoded.raster.height, src.height);
    assert!(!decoded.raster.has_alpha);
    assert_eq!(decoded.raster.to_rgba8_rgba(), src.to_rgba8_rgba());
}

#[test]
fn jxl_lossy_distance_roundtrip() {
    let src = make_test_rgba(128, 128);
    let enc = codec::encode_jxl(
        &src,
        &JxlSaveOptions {
            lossless: false,
            distance: 1.0,
            effort: 5,
            ..Default::default()
        },
    )
    .expect("lossy encode");
    let decoded = codec::decode_from_bytes(&enc, Some("jxl")).expect("decode");
    assert_eq!(decoded.raster.width, src.width);
    // lossy: pixels differ, but not wildly
    let a = src.to_rgba8_rgba();
    let b = decoded.raster.to_rgba8_rgba();
    let mut total: u64 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        total += (i64::from(*x) - i64::from(*y)).unsigned_abs();
    }
    assert!(
        total < (a.len() as u64) * 40,
        "lossy quality too poor: {total}"
    );
}

#[test]
fn png16_looked_lossless() {
    // 16-bit raster → JXL lossless → decode; still 16 bits.
    let w = 33;
    let h = 21;
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let v = ((x as u16 * 1234) ^ (y as u16 * 77)) & 0xFFFF;
            data.push(v);
            data.push(v / 2);
            data.push(v.wrapping_mul(2));
            data.push(65535);
        }
    }
    let src = Raster::from_rgba16(w, h, true, data);
    let enc = codec::encode_jxl(
        &src,
        &JxlSaveOptions {
            lossless: true,
            effort: 3,
            ..Default::default()
        },
    )
    .expect("encode 16-bit");
    let decoded = codec::decode_from_bytes(&enc, Some("jxl")).expect("decode");
    assert_eq!(decoded.raster.bits, 16, "16-bit chain preserved");
    match (&src.data, &decoded.raster.data) {
        (jxlviewer::raster::Samples::U16(a), jxlviewer::raster::Samples::U16(b)) => {
            assert_eq!(a, b, "lossless 16-bit pixels must match")
        }
        _ => panic!("sample depth changed"),
    }
}

#[test]
fn save_png_roundtrip() {
    let src = make_test_rgba(20, 33);
    let bytes = codec::encode_to_vec(&src, &SaveOptions::Png).expect("png encode");
    let back = codec::decode_from_bytes(&bytes, Some("png")).expect("png decode");
    assert_eq!(back.raster.width, 20);
    assert!(back.raster.has_alpha);
    assert_eq!(back.raster.to_rgba8_rgba(), src.to_rgba8_rgba());
}

#[test]
fn save_jpeg_flattens_alpha() {
    let src = make_test_rgba(32, 32);
    let bytes = codec::encode_to_vec(
        &src,
        &SaveOptions::Jpeg(codec::JpegSaveOptions { quality: 90 }),
    )
    .expect("jpeg encode");
    assert!(bytes.starts_with(&[0xFF, 0xD8]));
    // Hard to assert content details; just ensure decode works.
    let back = codec::decode_from_bytes(&bytes, Some("jpg")).expect("jpeg decode");
    assert_eq!(back.raster.width, 32);
    assert!(!back.raster.has_alpha, "jpeg output has no alpha");
}

#[test]
fn keep_drop_alpha_semantics() {
    let src = make_test_rgba(16, 16);
    let with_alpha = codec::encode_jxl(&src, &JxlSaveOptions::default()).expect("alpha jxl");
    let decoded = codec::decode_from_bytes(&with_alpha, Some("jxl")).expect("dec alpha");
    assert!(decoded.raster.has_alpha);

    let mut no_alpha_opts = JxlSaveOptions::default();
    no_alpha_opts.keep_alpha = false;
    let dropped = codec::encode_jxl(&src, &no_alpha_opts).expect("no-alpha jxl");
    let decoded2 = codec::decode_from_bytes(&dropped, Some("jxl")).expect("dec noalpha");
    assert!(!decoded2.raster.has_alpha);
}
