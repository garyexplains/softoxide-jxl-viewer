//! Writes a small animated JPEG XL (4 solid-color frames with a moving stripe,
//! 100 ms each, lossless) using the vendored libjxl encoder — handy for
//! testing animation playback:
//!
//!   cargo run --example gen_anim_jxl [out.jxl]

use std::io::Write as _;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "anim_test.jxl".into());
    const W: u32 = 160;
    const H: u32 = 120;
    const COLORS: [[u8; 3]; 4] = [[200, 40, 40], [40, 200, 40], [40, 40, 200], [220, 200, 40]];

    let mut frames: Vec<jxl_native::Pixels> = Vec::new();
    for (idx, rgb) in COLORS.iter().enumerate() {
        let mut data = Vec::with_capacity((W * H * 3) as usize);
        let stripe = idx * (W as usize / COLORS.len());
        for _y in 0..H {
            for x in 0..W {
                let (r, g, b) = if (x as usize) >= stripe && (x as usize) < stripe + W as usize / 8
                {
                    (255 - rgb[0], 255 - rgb[1], 255 - rgb[2])
                } else {
                    (rgb[0], rgb[1], rgb[2])
                };
                data.extend_from_slice(&[r, g, b]);
            }
        }
        frames.push(jxl_native::Pixels::U8(data));
    }

    let bytes = jxl_native::encode_anim_dynamic(jxl_native::AnimEncodeInput {
        width: W,
        height: H,
        has_alpha: false,
        frames,
        durations_ms: vec![100; COLORS.len()],
        loops: 0, // loop forever
        options: jxl_native::EncodeOptions {
            lossless: true,
            effort: 4,
            ..Default::default()
        },
    })?;

    let mut f = std::fs::File::create(out)?;
    f.write_all(&bytes)?;
    Ok(())
}
