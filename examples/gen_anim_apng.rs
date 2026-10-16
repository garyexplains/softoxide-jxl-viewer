//! Writes a small animated APNG (4 frames, 100 ms each) for testing animated
//! JPEG XL authoring:
//!
//!   cargo run --example gen_anim_apng [out.apng]
//!   cjxl anim.apng anim.jxl -d 0 -e 3
//!
//! Frame N fills the image with a distinct color so playback order is obvious.

use std::io::BufWriter;

fn main() -> std::io::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "anim_test.apng".into());
    const W: usize = 160;
    const H: usize = 120;
    const FRAMES: usize = 4;

    let colors: [[u8; 3]; FRAMES] = [[200, 40, 40], [40, 200, 40], [40, 40, 200], [220, 200, 40]];

    let file = std::fs::File::create(out)?;
    let mut enc = png::Encoder::new(BufWriter::new(file), W as u32, H as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_animated(FRAMES as u32, 0)?; // 0 = loop forever
    let mut writer = enc.write_header()?;

    for (idx, rgb) in colors.iter().enumerate() {
        // 10/100 s = 100 ms per frame.
        writer.set_frame_delay(10, 100)?;
        let mut buf = vec![0u8; W * H * 4];
        // A moving stripe so frames are distinguishable beyond color.
        let stripe_x = idx * (W / FRAMES);
        for y in 0..H {
            for x in 0..W {
                let (r, g, b) = if x >= stripe_x && x < stripe_x + W / 8 {
                    (255 - rgb[0], 255 - rgb[1], 255 - rgb[2])
                } else {
                    (rgb[0], rgb[1], rgb[2])
                };
                let o = (y * W + x) * 4;
                buf[o] = r;
                buf[o + 1] = g;
                buf[o + 2] = b;
                buf[o + 3] = 255;
            }
        }
        writer.write_image_data(&buf)?;
    }
    writer.finish()?;
    Ok(())
}
