//! Writes a synthetic HDR test image as a color PFM (Portable Float Map):
//! a horizontal ramp of PQ/HLG-encoded signal values (0..1 → up to 10000 nits
//! when declared Rec.2100 PQ) with saturated R/G/B bars at PQ 0.75 (~1000 nits).
//!
//! Usage: cargo run --example gen_hdr_pfm [out.pfm]
//!
//! Encode to HDR JPEG XL with the vendored cjxl:
//!   cjxl hdr_gradient.pfm hdr_gradient.jxl -x color_space=Rec2100PQ -d 0 -e 3

use std::io::Write as _;

fn main() -> std::io::Result<()> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "hdr_gradient.pfm".into());
    let (w, h) = (512usize, 256usize);

    // PFM rows are stored bottom-to-top; scale -1.0 means little-endian.
    let mut data = Vec::with_capacity(w * h * 3 * 4);
    for y in (0..h).rev() {
        for x in 0..w {
            let t = x as f32 / (w - 1) as f32;
            let sig = if y >= h - 48 {
                // Saturated bars at PQ 0.75 ≈ 1000 nits.
                match x * 3 / w {
                    0 => [0.75f32, 0.10, 0.10],
                    1 => [0.10f32, 0.75, 0.10],
                    _ => [0.10f32, 0.10, 0.75],
                }
            } else {
                [t, t, t]
            };
            for v in sig {
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
    }

    let mut f = std::fs::File::create(out)?;
    write!(f, "PF\n{} {}\n-1.0\n", w, h)?;
    f.write_all(&data)?;
    Ok(())
}
