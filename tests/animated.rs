//! Animated JPEG XL end-to-end test: `jxl-native` (libjxl) *encodes* a 4-frame
//! animation and `jxl-oxide` — an independent implementation — decodes it.
//! Frames, colors, order, and durations must all survive the round-trip.

use jxlviewer::codec;
use jxlviewer::raster::Raster;

fn solid_frame(w: u32, h: u32, rgb: [u8; 3]) -> Raster {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for _ in 0..w * h {
        data.extend_from_slice(&rgb);
    }
    Raster::from_rgba8(w, h, false, data)
}

#[test]
fn animated_jxl_frames_and_durations() {
    const W: u32 = 64;
    const H: u32 = 48;
    const COLORS: [[u8; 3]; 4] = [[200, 40, 40], [40, 200, 40], [40, 40, 200], [220, 200, 40]];

    let frames: Vec<jxl_native::Pixels> = COLORS
        .iter()
        .map(|rgb| match &solid_frame(W, H, *rgb).data {
            jxlviewer::raster::Samples::U8(v) => jxl_native::Pixels::U8(v.clone()),
            _ => unreachable!(),
        })
        .collect();
    let durations = vec![100u32; 4]; // 100 ms per frame

    let jxl = jxl_native::encode_anim_dynamic(jxl_native::AnimEncodeInput {
        width: W,
        height: H,
        has_alpha: false,
        frames,
        durations_ms: durations.clone(),
        loops: 0, // infinite
        options: jxl_native::EncodeOptions {
            lossless: true,
            effort: 3,
            ..Default::default()
        },
    })
    .expect("animated encode");

    let img = codec::decode_from_bytes(&jxl, Some("jxl")).expect("animated decode");

    let anim = img.animation.expect("animation should be present");
    assert_eq!(anim.frame_count(), 4, "all 4 keyframes decoded");
    assert!(
        anim.durations_ms.iter().all(|d| (90..=110).contains(d)),
        "durations ≈ 100 ms, got {:?}",
        anim.durations_ms
    );

    // Frames are in authoring order: solid colors R, G, B, Y.
    for (idx, (frame, want)) in anim.frames.iter().zip(COLORS.iter()).enumerate() {
        let px = frame.to_rgba8_rgba();
        let mid = ((W as usize / 2) + (H as usize / 2) * W as usize) * 4;
        assert_eq!(&px[mid..mid + 3], want, "frame {idx} center color mismatch");
        // Every pixel of the frame matches the solid color (opaque alpha).
        for p in px.chunks_exact(4) {
            assert_eq!(&p[0..3], want, "frame {idx} not solid");
            assert_eq!(p[3], 255);
        }
    }

    // The document raster is the first frame.
    let first_px = img.raster.to_rgba8_rgba();
    assert_eq!(&first_px[0..3], &[200, 40, 40]);
}

#[test]
fn animation_duration_math() {
    use jxlviewer::codec::animation_duration_ms as dur;
    // tps = 1000/100 → 10 ticks per second → 1 tick = 100 ms.
    assert_eq!(dur(1, 1000, 100), 100);
    assert_eq!(dur(5, 1000, 100), 500);
    // Degenerate metadata falls back to 100 ms.
    assert_eq!(dur(1, 0, 0), 100);
}
