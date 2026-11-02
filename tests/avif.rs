//! AVIF decode integration: stills (incl. alpha, container rotation, EXIF
//! metadata), graceful refusal of animated sequences and PQ (HDR) inputs.
//! Fixtures are tiny committed files under assets/avif/ (regenerate with
//! `cargo run -p avif-native --features fixture-encoder --example gen_avif`).

use jxlviewer::codec::{self, Format};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("avif")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {} missing: {e}", path.display()))
}

#[test]
fn still_basic() {
    let bytes = fixture("basic.avif");
    assert_eq!(codec::detect_format(&bytes, None), Format::Avif);
    let img = codec::decode_from_bytes(&bytes, Some("avif")).expect("decode basic");
    assert_eq!(img.raster.width, 26);
    assert_eq!(img.raster.height, 18);
    assert!(!img.raster.has_alpha);
    assert_eq!(img.format, Format::Avif);
    // Solid color survives lossy encoding within a tolerance.
    let px = img.raster.to_rgba8_rgba();
    let center = (px.len() / 2 / 4) * 4;
    for (ch, want) in [0xB0i64, 0x5Ai64, 0x2Ei64].iter().enumerate() {
        let got = px[center + ch] as i64;
        assert!((got - *want).abs() <= 12, "ch {ch}: {got} vs {want}");
    }
    assert!(img.meta.is_none(), "no metadata in the basic fixture");
}

#[test]
fn still_alpha() {
    let bytes = fixture("alpha.avif");
    let img = codec::decode_from_bytes(&bytes, None).expect("decode alpha");
    assert_eq!(img.raster.width, 26);
    assert_eq!(img.raster.height, 18);
    assert!(img.raster.has_alpha);
    // Constant alpha 0x88 straight through (8-bit tolerance).
    if let jxlviewer::raster::Samples::U8(v) = &img.raster.data {
        let a = v[3] as i32;
        assert!((a - 0x88).abs() <= 12, "alpha {a} vs 0x88");
    } else {
        panic!("expected u8 samples");
    }
}

#[test]
fn irot_rotation_applied_and_direction() {
    // Stored 18×26 with irot(90° CCW): displays 26×18. Left half dark,
    // right half bright → after CCW rotation, decoded top-left is bright.
    let bytes = fixture("rotated.avif");
    let img = codec::decode_from_bytes(&bytes, None).expect("decode rotated");
    assert_eq!(img.raster.width, 26, "rotated to display width");
    assert_eq!(img.raster.height, 18, "rotated to display height");
    if let jxlviewer::raster::Samples::U8(v) = &img.raster.data {
        let tl = (&v[0..3]).to_vec();
        assert!(
            tl[0] > 0x90 && tl[1] > 0x90,
            "decoded TL must be the bright (stored-right) half: {tl:?}"
        );
        let br = (&v[v.len() - 4..v.len() - 1]).to_vec(); // last pixel = bottom-right = stored-left
        assert!(
            br[0] < 0x80 && br[1] < 0x80,
            "decoded bottom-right must be dark (stored-left) half: {br:?}"
        );
    } else {
        panic!("expected u8 samples");
    }
}

#[test]
fn exif_payload_parsed() {
    let bytes = fixture("exif.avif");
    let img = codec::decode_from_bytes(&bytes, None).expect("decode exif fixture");
    let ex = img
        .meta
        .as_ref()
        .and_then(|m| m.exif.as_ref())
        .expect("EXIF parsed");
    let make = ex.tags.iter().find(|(n, _)| n == "Make");
    assert!(make.is_some(), "Make tag present: {ex:?}");
    assert!(
        make.unwrap().1.contains("avif"),
        "make = {:?}",
        make.unwrap().1
    );
    let model = ex.tags.iter().find(|(n, _)| n == "Model");
    assert!(model.is_some_and(|(_, v)| v.contains("fixture")));
}

#[test]
fn hdr_pq_refused_gracefully() {
    let bytes = fixture("hdr_pq.avif");
    assert_eq!(codec::detect_format(&bytes, None), Format::Avif);
    let err = codec::decode_from_bytes(&bytes, None).expect_err("PQ must be refused");
    assert!(err.contains("HDR"), "friendly error: {err}");
    assert!(err.contains("PQ"), "friendly error: {err}");
}

#[test]
fn animated_sequence_refused_gracefully() {
    let bytes = fixture("sequence.avif");
    assert_eq!(codec::detect_format(&bytes, None), Format::Avif);
    let err = codec::decode_from_bytes(&bytes, None).expect_err("sequence must be refused");
    assert!(
        err.contains("animated"),
        "friendly error should mention animation: {err}"
    );
}

#[test]
fn avis_brand_detected_and_refused_without_panic() {
    let bytes = fixture("anim-brand.avif");
    assert_eq!(codec::detect_format(&bytes, None), Format::Avif);
    // Graceful: returns Err (parse failure for this stub), never panics.
    let _ = codec::decode_from_bytes(&bytes, None);
}

#[test]
fn meta_from_avif_payload_helpers() {
    use jxlviewer::meta;
    // Bare TIFF payload with Make+Model (same layout as the exid fixture).
    let tiff = [
        0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x02, 0x00, 0x0F, 0x01, 0x02, 0x00, 0x05,
        0x00, 0x00, 0x00, 0x26, 0x00, 0x00, 0x00, 0x10, 0x01, 0x02, 0x00, 0x08, 0x00, 0x00, 0x00,
        0x2B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0,
    ];
    // a 5-letter + 8-letter ASCII pair fits "avif\0" + "fixture\0".
    let mut tiff2 = tiff.to_vec();
    tiff2.extend_from_slice(b"avif\0fixture\0");
    let m = meta::from_avif(&tiff2, b"<xmp/>").expect("meta");
    assert!(m.exif.as_ref().is_some_and(|e| e.tags.len() >= 1));
    assert_eq!(m.xmp.as_deref(), Some(b"<xmp/>".as_slice()));
    assert!(m.iptc.is_none());
    assert!(meta::from_avif(&[], &[]).is_none());
}
