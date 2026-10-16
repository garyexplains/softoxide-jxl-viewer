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
fn animated_jxl_saves_as_animated_gif() {
    const W: u32 = 32;
    const H: u32 = 32;
    const COLORS: [[u8; 3]; 4] = [[200, 40, 40], [40, 200, 40], [40, 40, 200], [220, 200, 40]];
    let frames: Vec<jxl_native::Pixels> = COLORS
        .iter()
        .map(|rgb| match &solid_frame(W, H, *rgb).data {
            jxlviewer::raster::Samples::U8(v) => jxl_native::Pixels::U8(v.clone()),
            _ => unreachable!(),
        })
        .collect();

    let jxl = jxl_native::encode_anim_dynamic(jxl_native::AnimEncodeInput {
        width: W,
        height: H,
        has_alpha: false,
        frames,
        durations_ms: vec![100; 4],
        loops: 0,
        options: jxl_native::EncodeOptions {
            lossless: true,
            effort: 3,
            ..Default::default()
        },
    })
    .expect("animated encode");

    // Our decoder reads it back as an animation.
    let img = codec::decode_from_bytes(&jxl, Some("jxl")).expect("decode");
    let anim = img.animation.expect("animation");

    // Save as animated GIF and read every frame back with the GIF decoder.
    let dir = std::env::temp_dir().join(format!("jxlviewer-gif-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gif_path = dir.join("out.gif");
    codec::save_animation_to_path(
        &gif_path,
        &anim.frames,
        &anim.durations_ms,
        anim.loops,
        &codec::SaveOptions::Gif,
    )
    .expect("gif save");

    let gif_bytes = std::fs::read(&gif_path).unwrap();
    use image::{AnimationDecoder as _, ImageDecoder as _};
    let decoder =
        image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&gif_bytes)).expect("gif open");
    assert_eq!(decoder.dimensions(), (W, H));
    let frames: Vec<_> = decoder
        .into_frames()
        .collect::<Result<Vec<_>, _>>()
        .expect("gif frames");
    assert_eq!(frames.len(), 4, "all frames in the GIF");

    // Delays are ≈100 ms (GIF stores 10 ms units).
    for f in &frames {
        let (n, d) = f.delay().numer_denom_ms();
        let ms = n as f64 / d as f64;
        assert!((80.0..=120.0).contains(&ms), "delay {ms} ms");
    }

    // Colors survive quantization within a small tolerance.
    for (idx, f) in frames.iter().enumerate() {
        let buf = f.buffer();
        let want = COLORS[idx];
        let mut max_delta = 0u8;
        for p in buf.chunks_exact(4) {
            for c in 0..3 {
                max_delta = max_delta.max(p[c].abs_diff(want[c]));
            }
        }
        assert!(max_delta <= 24, "frame {idx} color drift {max_delta}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gif_to_jxl_roundtrip() {
    // Animated JXL → animated GIF (image crate encoder, quantized) → our GIF
    // decoder (disposal-aware) → animated JXL again. Colors must survive to
    // within quantization tolerance in the GIF stage and exactly in the final
    // lossless JXL.
    const W: u32 = 40;
    const H: u32 = 30;
    const COLORS: [[u8; 3]; 4] = [[200, 40, 40], [40, 200, 40], [40, 40, 200], [220, 200, 40]];
    let frames: Vec<jxl_native::Pixels> = COLORS
        .iter()
        .map(|rgb| match &solid_frame(W, H, *rgb).data {
            jxlviewer::raster::Samples::U8(v) => jxl_native::Pixels::U8(v.clone()),
            _ => unreachable!(),
        })
        .collect();
    let jxl1 = jxl_native::encode_anim_dynamic(jxl_native::AnimEncodeInput {
        width: W,
        height: H,
        has_alpha: false,
        frames,
        durations_ms: vec![100; 4],
        loops: 7,
        options: jxl_native::EncodeOptions {
            lossless: true,
            effort: 3,
            ..Default::default()
        },
    })
    .expect("encode anim jxl");

    // Stage 1: decode the animated JXL.
    let img1 = codec::decode_from_bytes(&jxl1, Some("jxl")).expect("decode anim jxl");
    let anim1 = img1.animation.expect("animation");
    assert_eq!(anim1.frame_count(), 4);
    assert_eq!(anim1.loops, 7);

    // Stage 2: save as animated GIF.
    let dir = std::env::temp_dir().join(format!("jxlviewer-gif2jxl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let gif_path = dir.join("mid.gif");
    codec::save_animation_to_path(
        &gif_path,
        &anim1.frames,
        &anim1.durations_ms,
        anim1.loops,
        &codec::SaveOptions::Gif,
    )
    .expect("gif save");

    // Stage 3: open the GIF — must come back as an animation.
    let gif_bytes = std::fs::read(&gif_path).unwrap();
    let img2 = codec::decode_from_bytes(&gif_bytes, Some("gif")).expect("decode gif");
    let anim2 = img2.animation.expect("GIF should decode as animation");
    assert_eq!(anim2.frame_count(), 4);
    assert_eq!(anim2.loops, 7, "loop count preserved");
    for (i, d) in anim2.durations_ms.iter().enumerate() {
        assert!((90..=110).contains(d), "frame {i} delay {d} ms");
    }
    for (idx, frame) in anim2.frames.iter().enumerate() {
        let px = frame.to_rgba8_rgba();
        let want = COLORS[idx];
        let mut max_delta = 0u8;
        for p in px.chunks_exact(4) {
            for c in 0..3 {
                max_delta = max_delta.max(p[c].abs_diff(want[c]));
            }
        }
        assert!(
            max_delta <= 24,
            "GIF frame {idx} quantization drift {max_delta}"
        );
    }

    // Stage 4: save the GIF's frames as an animated (lossless) JXL.
    let jxl2 = codec::encode_anim_jxl(
        &anim2.frames,
        &anim2.durations_ms,
        anim2.loops,
        &codec::JxlSaveOptions {
            lossless: true,
            effort: 3,
            ..Default::default()
        },
    )
    .expect("re-encode anim jxl");

    // Stage 5: the new JXL must carry the same animation (exact colors now).
    let img3 = codec::decode_from_bytes(&jxl2, Some("jxl")).expect("decode re-encoded jxl");
    let anim3 = img3.animation.expect("animation after re-encode");
    assert_eq!(anim3.frame_count(), 4);
    assert_eq!(anim3.loops, 7);
    for (idx, frame) in anim3.frames.iter().enumerate() {
        let px = frame.to_rgba8_rgba();
        let want = COLORS[idx];
        for p in px.chunks_exact(4) {
            assert_eq!(&p[0..3], want, "lossless JXL frame {idx} must be exact");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
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
