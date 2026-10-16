//! Generates a photo-like 1920×1080 test image (gradients, shapes, fine grain)
//! for comparing JXL encoder settings.
//!
//! Usage: `cargo run --release --example gen_photo -- out.png`
use image::{ImageBuffer, Rgb};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "photo_test.png".into());
    let (w, h) = (1920u32, 1080u32);
    let mut img = ImageBuffer::<Rgb<u8>, _>::new(w, h);

    // xorshift64 PRNG — deterministic, no external deps.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for y in 0..h {
        let fy = y as f32 / h as f32;
        for x in 0..w {
            let fx = x as f32 / w as f32;

            // Sky gradient.
            let mut r = 90.0 + 80.0 * fy;
            let mut g = 140.0 + 60.0 * fy;
            let mut b = 220.0 - 60.0 * fy;

            // Sun disc.
            let dx = fx - 0.72;
            let dy = fy - 0.25;
            if dx * dx + dy * dy < 0.006 {
                r = 250.0;
                g = 235.0;
                b = 180.0;
            }

            // Rolling hills.
            let horizon = 0.62 + 0.05 * (fx * 9.0).sin();
            if fy > horizon {
                let t = (fy - horizon) / (1.0 - horizon);
                r = 40.0 + 60.0 * t;
                g = 90.0 + 40.0 * t;
                b = 40.0 + 30.0 * t;
            }

            // Fine film-grain-style noise (expensive to compress, like photos).
            let n = (rnd() % 21) as f32 - 10.0;
            let px = Rgb([
                (r + n).clamp(0.0, 255.0) as u8,
                (g + n).clamp(0.0, 255.0) as u8,
                (b + n * 0.6).clamp(0.0, 255.0) as u8,
            ]);
            img.put_pixel(x, y, px);
        }
    }
    img.save(&out).expect("save png");
    println!("wrote {out}");
}
