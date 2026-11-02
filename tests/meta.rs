//! Metadata extraction (EXIF/XMP/IPTC) and EXIF-orientation auto-apply at
//! decode. Fixtures are crafted by splicing segments/chunks into real
//! encoder-produced containers, so the production decode path is exercised.

use image::ImageEncoder;
use jxlviewer::codec::{self, Format, JpegSaveOptions, SaveOptions};
use jxlviewer::raster::{Raster, Samples};

mod common;
use common::{exif_tiff_block, jpeg_with_marker, png_insert_after_ihdr};

/// Appends an EXIF chunk to a RIFF/WebP file (image-webp demands VP8/VP8L as
/// the first chunk; extra chunks go at the end, size fixed up).
fn webp_insert_exif(webp: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(b"EXIF");
    chunk.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
    chunk.extend_from_slice(tiff);
    if tiff.len() & 1 == 1 {
        chunk.push(0); // chunks are word-aligned
    }

    let mut out = Vec::with_capacity(webp.len() + chunk.len());
    out.extend_from_slice(webp);
    out.extend_from_slice(&chunk);
    let sz = u32::from_le_bytes([out[4], out[5], out[6], out[7]]) + chunk.len() as u32;
    out[4..8].copy_from_slice(&sz.to_le_bytes());
    out
}

fn u8_samples(s: &Samples) -> &[u8] {
    match s {
        Samples::U8(v) => v,
        Samples::U16(_) => panic!("expected u8 samples"),
    }
}

// --- tests -------------------------------------------------------------------

#[test]
fn jpeg_exif_orientation_and_tags() {
    let raster = Raster::from_rgba8(20, 10, false, vec![50u8; 20 * 10 * 3]);
    let jpeg = codec::encode_to_vec(&raster, &SaveOptions::Jpeg(JpegSaveOptions::default()))
        .expect("jpeg encode");

    // No EXIF → empty metadata, dims unchanged.
    let plain = codec::decode_from_bytes(&jpeg, Some("jpg")).unwrap();
    assert_eq!(plain.raster.width, 20);
    assert_eq!(plain.raster.height, 10);
    assert!(plain.meta.as_ref().is_some_and(|m| m.is_empty()));

    // Orientation 6 → rotate 90° CW: 20×10 becomes 10×20.
    let tiff = exif_tiff_block(6, Some("TestCam"));
    let mut seg = b"Exif\0\0".to_vec();
    seg.extend_from_slice(&tiff);
    let img = codec::decode_from_bytes(&jpeg_with_marker(&jpeg, 0xE1, &seg), Some("jpg")).unwrap();
    assert_eq!(img.raster.width, 10);
    assert_eq!(img.raster.height, 20);
    let ex = img
        .meta
        .as_ref()
        .and_then(|m| m.exif.as_ref())
        .expect("exif present");
    assert_eq!(ex.orientation, 6);
    assert!(ex.applied, "viewer rotated the pixels");
    assert_eq!(ex.tags.len(), 2, "Make + Orientation");
    let make = ex.tags.iter().find(|(n, _)| n == "Make").unwrap();
    assert!(make.1.contains("TestCam"), "make = {:?}", make.1);
}

#[test]
fn png_exif_orientation_matches_rotate_cw() {
    // Distinct per-pixel RGB content so the rotated compare is meaningful.
    let mut data = Vec::with_capacity(20 * 10 * 3);
    for y in 0..10u32 {
        for x in 0..20u32 {
            data.push((x * 13) as u8);
            data.push((y * 31) as u8);
            data.push((x * 3 + y) as u8);
        }
    }
    let raster = Raster::from_rgba8(20, 10, false, data);
    let png = codec::encode_to_vec(&raster, &SaveOptions::Png).expect("png encode");
    let plain = codec::decode_from_bytes(&png, Some("png")).unwrap().raster;
    assert_eq!(plain.width, 20);

    let png2 = png_insert_after_ihdr(&png, b"eXIf", &exif_tiff_block(6, None));
    let img = codec::decode_from_bytes(&png2, Some("png")).unwrap();
    assert_eq!(img.raster.width, 10);
    assert_eq!(img.raster.height, 20);
    let expect = plain.rotate_cw();
    assert_eq!(u8_samples(&img.raster.data), u8_samples(&expect.data));
    let ex = img.meta.as_ref().and_then(|m| m.exif.as_ref()).unwrap();
    assert_eq!(ex.orientation, 6);
    assert!(ex.applied);
}

#[test]
fn png_xmp_itxt() {
    let raster = Raster::from_rgba8(4, 4, false, vec![9u8; 4 * 4 * 3]);
    let png = codec::encode_to_vec(&raster, &SaveOptions::Png).expect("png encode");

    let xmp_text = b"<x:xmpmeta><rdf:RDF/></x:xmpmeta>";
    let mut data = b"XML:com.adobe.xmp\0".to_vec();
    data.extend_from_slice(&[0, 0, 0, 0]); // flag, method, lang\0, translated\0
    data.extend_from_slice(xmp_text);
    let png2 = png_insert_after_ihdr(&png, b"iTXt", &data);

    let img = codec::decode_from_bytes(&png2, Some("png")).unwrap();
    let xmp = img.meta.as_ref().and_then(|m| m.xmp.as_ref()).expect("xmp");
    assert_eq!(xmp, xmp_text);
}

#[test]
fn jpeg_xmp_and_iptc() {
    let raster = Raster::from_rgba8(8, 6, false, vec![77u8; 8 * 6 * 3]);
    let jpeg = codec::encode_to_vec(&raster, &SaveOptions::Jpeg(JpegSaveOptions::default()))
        .expect("jpeg encode");

    // XMP APP1 ("http://ns.adobe.com/xap/1.0/\0" header).
    let mut seg = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    seg.extend_from_slice(b"<x:xmpmeta/>");
    let img = codec::decode_from_bytes(&jpeg_with_marker(&jpeg, 0xE1, &seg), Some("jpg")).unwrap();
    let meta = img.meta.as_ref().expect("meta");
    assert_eq!(
        meta.xmp.as_ref().map(|v| v.as_slice()),
        Some(&b"<x:xmpmeta/>"[..])
    );

    // IPTC APP13 ("Photoshop 3.0\0" header).
    let mut ps = b"Photoshop 3.0\0".to_vec();
    ps.extend_from_slice(b"\x38\x42\x49\x4d\x04\x04\0\0\0\0"); // 8BIM head
    let img = codec::decode_from_bytes(&jpeg_with_marker(&jpeg, 0xED, &ps), Some("jpg")).unwrap();
    let meta = img.meta.as_ref().expect("meta");
    assert_eq!(meta.iptc.as_ref().map(|v| v.len()), Some(ps.len()));
    assert!(meta.xmp.is_none());
}

#[test]
fn webp_exif_orientation() {
    let img = image::RgbImage::from_fn(20, 10, |x, y| {
        image::Rgb([(x * 7) as u8, (y * 9) as u8, 55])
    });
    let mut webp = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
        .write_image(img.as_raw(), 20, 10, image::ExtendedColorType::Rgb8)
        .expect("webp encode");

    let webp2 = webp_insert_exif(&webp, &exif_tiff_block(6, Some("WebbCam")));
    let img = codec::decode_from_bytes(&webp2, Some("webp")).unwrap();
    assert_eq!(img.raster.width, 10);
    assert_eq!(img.raster.height, 20);
    let ex = img.meta.as_ref().and_then(|m| m.exif.as_ref()).unwrap();
    assert_eq!(ex.orientation, 6);
    assert!(ex.applied);
    let make = ex.tags.iter().find(|(n, _)| n == "Make").unwrap();
    assert!(make.1.contains("WebbCam"));
}

#[test]
fn tiff_metadata_from_container() {
    // A whole-file parse of the crafted TIFF block (same parser the JXL Exif
    // box payload goes through).
    let tiff = exif_tiff_block(6, Some("TiffCam"));
    let meta = jxlviewer::meta::from_bytes(&tiff, Format::Tiff).expect("meta");
    let ex = meta.exif.expect("exif");
    assert_eq!(ex.orientation, 6);
    assert_eq!(ex.tags.len(), 2);
    let make = ex.tags.iter().find(|(n, _)| n == "Make").unwrap();
    assert!(make.1.contains("TiffCam"));
}

#[test]
fn gif_has_no_metadata() {
    let raster = Raster::from_rgba8(4, 4, false, vec![12u8; 4 * 4 * 3]);
    let gif = codec::encode_to_vec(&raster, &SaveOptions::Gif).expect("gif encode");
    let img = codec::decode_from_bytes(&gif, Some("gif")).unwrap();
    assert!(img.meta.is_none(), "GIF carries no tracked metadata");
}

#[test]
fn orientation_names_and_sizes() {
    assert_eq!(
        jxlviewer::meta::ImageMetadata::orientation_name(6),
        "Rotate 90 CW"
    );
    assert_eq!(
        jxlviewer::meta::ImageMetadata::orientation_name(1),
        "Normal"
    );
    assert_eq!(
        jxlviewer::meta::ImageMetadata::orientation_name(9),
        "Unknown"
    );
    assert_eq!(jxlviewer::meta::human_size(500), "500 B");
    assert_eq!(jxlviewer::meta::human_size(2048), "2.0 KiB");
}
