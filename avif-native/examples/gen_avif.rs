//! Regenerates the small committed AVIF test fixtures under `assets/avif/`
//! using the representable-only `fixture-encoder` feature of `avif-native`.
//!
//! The shipped app/CLI never encodes AVIF; this tool exists so the fixtures
//! stay reproducible and reviewable:
//!
//! ```sh
//! cargo run -p avif-native --features fixture-encoder --example gen_avif
//! ```
//!
//! Fixtures produced (8-bit, YUV 4:4:4, encoder defaults):
//! * `basic.avif`      — 26×18 solid color, no alpha
//! * `alpha.avif`      — 26×18 solid color, straight alpha (convex mask)
//! * `rotated.avif`    — 18×26 image with irot 90° CCW (decodes 26×18)
//! * `exif.avif`       — 26×18 image carrying an EXIF payload (Make+Model)
//! * `hdr_pq.avif`     — 26×18 image signaling PQ (the viewer must refuse)

use avif_native::fixtures::FixtureSpec;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("assets")
        .join("avif");
    std::fs::create_dir_all(&dir)?;

    let write = |name: &str, bytes: Vec<u8>| -> Result<(), std::io::Error> {
        let path = dir.join(name);
        std::fs::write(&path, &bytes)?;
        println!("{} ({} bytes)", path.display(), bytes.len());
        Ok(())
    };

    // Minimal EXIF payload: Make = "avif" + Model = "fixture" (camera-y text).
    let exif = exif_tiff_block();

    write(
        "basic.avif",
        enc(&FixtureSpec {
            width: 26,
            height: 18,
            color: [0xB0, 0x5A, 0x2E],
            color_right: None,
            alpha: None,
            rotation_quarters: 0,
            mirror: None,
            exif: None,
            pq_transfer: false,
        })?,
    )?;

    write(
        "alpha.avif",
        enc(&FixtureSpec {
            width: 26,
            height: 18,
            color: [0x2C, 0x7A, 0xB8],
            // Constant straight alpha; `has_alpha` and values observable.
            alpha: Some(0x88),
            color_right: None,
            rotation_quarters: 0,
            mirror: None,
            exif: None,
            pq_transfer: false,
        })?,
    )?;

    // Two-frame animated sequence (the viewer must refuse it gracefully).
    write(
        "sequence.avif",
        enc_sequence(&[
            FixtureSpec {
                width: 26,
                height: 18,
                color: [0x40, 0x40, 0xC0],
                color_right: None,
                alpha: None,
                rotation_quarters: 0,
                mirror: None,
                exif: None,
                pq_transfer: false,
            },
            FixtureSpec {
                width: 26,
                height: 18,
                color: [0xC0, 0x40, 0x40],
                color_right: None,
                alpha: None,
                rotation_quarters: 0,
                mirror: None,
                exif: None,
                pq_transfer: false,
            },
        ])?,
    )?;

    // 18×26 stored; irot(90° CCW) → viewer displays 26×18. Two-tone so the
    // rotation direction is observable: stored-left = dark, stored-right =
    // bright → after CCW rotation, decoded top-left must be bright.
    write(
        "rotated.avif",
        enc(&FixtureSpec {
            width: 18,
            height: 26,
            color: [0x20, 0x40, 0x20],
            color_right: Some([0xD0, 0xE0, 0xD0]),
            alpha: None,
            rotation_quarters: 1,
            mirror: None,
            exif: None,
            pq_transfer: false,
        })?,
    )?;

    write(
        "exif.avif",
        enc(&FixtureSpec {
            width: 26,
            height: 18,
            color: [0x70, 0x40, 0x90],
            color_right: None,
            alpha: None,
            rotation_quarters: 0,
            mirror: None,
            exif: Some(exif.clone()),
            pq_transfer: false,
        })?,
    )?;

    write(
        "hdr_pq.avif",
        enc(&FixtureSpec {
            width: 26,
            height: 18,
            color: [0x40, 0x40, 0x40],
            color_right: None,
            alpha: None,
            rotation_quarters: 0,
            mirror: None,
            exif: None,
            pq_transfer: true,
        })?,
    )?;

    write(
        "anim-brand.avif",
        // Not produced by the encoder: the brand-only minimal container used
        // to test graceful refusal of sequences via detect_format + decode.
        b"\x00\x00\x00\x20ftypavis\x00\x00\x00\x00avis\x00\x00\x00\x00mif1\x00\x00\x00\x00\x00\x00\x00\x08free".to_vec(),
    )?;

    Ok(())
}

fn enc(spec: &FixtureSpec) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(avif_native::fixtures::encode_fixture(spec)?)
}

fn enc_sequence(specs: &[FixtureSpec]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(avif_native::fixtures::encode_sequence_fixture(specs)?)
}

/// Hand-built minimal TIFF/EXIF block (little-endian): Make "avif", Model
/// "fixture" (same layout as the metadata tests).
fn exif_tiff_block() -> Vec<u8> {
    let mut make = b"avif".to_vec();
    make.push(0);
    let mut model = b"fixture".to_vec();
    model.push(0);
    let n_entries: u16 = 2;
    let data_off: u32 = 8 + 2 + 12 * n_entries as u32 + 4;
    let mut v = Vec::new();
    v.extend_from_slice(b"II*\0");
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&n_entries.to_le_bytes());
    // Make (0x010F ASCII)
    v.extend_from_slice(&0x010Fu16.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&((make.len()) as u32).to_le_bytes());
    v.extend_from_slice(&data_off.to_le_bytes());
    // Model (0x0110 ASCII), data right after Make
    v.extend_from_slice(&0x0110u16.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&((model.len()) as u32).to_le_bytes());
    v.extend_from_slice(&(data_off + make.len() as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes()); // next IFD
    v.extend_from_slice(&make);
    v.extend_from_slice(&model);
    v
}
