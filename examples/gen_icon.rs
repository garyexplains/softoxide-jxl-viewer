//! Generates the jxlviewer icon set entirely from vector math (signed
//! distance fields) — original artwork, no fonts or third-party assets.
//!
//! Outputs:
//!   assets/icon.png                1024×1024 master
//!   assets/icons/jxlviewer-N.png   16/32/48/64/128/256 (Linux hicolor sizes)
//!   assets/icon.ico                Windows (BMP entries ≤64, PNG 128/256)
//!   assets/icon.icns               macOS (PNG-payload chunks)
//!
//! Run: cargo run --release --example gen_icon
use image::imageops::FilterType;
use image::{ImageBuffer, ImageFormat, Rgba};
use std::io::Cursor;

const SIZE: u32 = 1024;

fn main() {
    let master = render();
    std::fs::create_dir_all("assets/icons").expect("mkdir assets/icons");
    master.save("assets/icon.png").expect("save master");

    let mut scaled: Vec<(u32, ImageBuffer<Rgba<u8>, Vec<u8>>)> = Vec::new();
    for s in [16u32, 32, 48, 64, 128, 256, 512] {
        let small = image::imageops::resize(&master, s, s, FilterType::Lanczos3);
        small
            .save(format!("assets/icons/jxlviewer-{s}.png"))
            .expect("save size");
        scaled.push((s, small));
    }

    write_ico(&master, &scaled);
    write_icns(&master, &scaled);
    println!("icon set written to assets/");
}

// ---------------------------------------------------------------------------
// Artwork: rounded square, diagonal indigo→teal gradient, white "JXL"
// letterforms drawn as thick round-capped strokes, amber accent dot.
// ---------------------------------------------------------------------------

fn render() -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(SIZE, SIZE);
    let s = SIZE as f32;

    // Letterform geometry (cap top 352, baseline 672, stroke half-width 36).
    let u = 36.0;
    let segs: [[f32; 4]; 6] = [
        // J: vertical bar + bottom hook to the left
        [300.0, 352.0, 300.0, 640.0],
        [300.0, 640.0, 184.0, 640.0],
        // X: two crossing diagonals
        [436.0, 352.0, 592.0, 672.0],
        [592.0, 352.0, 436.0, 672.0],
        // L: vertical + baseline foot
        [700.0, 352.0, 700.0, 672.0],
        [700.0, 672.0, 826.0, 672.0],
    ];
    let accent = (838.0f32, 240.0f32, 36.0f32); // cx, cy, r

    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);

            // Background: rounded square with a diagonal indigo→teal gradient.
            let d_bg = sd_rounded_box(px, py, s * 0.5, s * 0.5, s * 0.5, s * 0.5, 200.0);
            let a_bg = cov(d_bg);
            let t = ((px + py) / (2.0 * s)).clamp(0.0, 1.0);
            let bg = lerp_rgb([35, 46, 82], [15, 110, 104], t);

            // Letterforms: union of capsules.
            let mut d = f32::INFINITY;
            for [ax, ay, bx, by] in segs {
                d = d.min(sd_seg(px, py, ax, ay, bx, by, u));
            }
            let a_l = cov(d);

            // Amber accent dot.
            let a_d = cov(sd_circle(px, py, accent.0, accent.1, accent.2));

            // Over-composite: bg ← accent ← letters.
            let mut rgba = [bg[0], bg[1], bg[2], (a_bg * 255.0).round() as u8];
            over(&mut rgba, [255, 176, 59], a_d);
            over(&mut rgba, [245, 248, 255], a_l);
            img.put_pixel(x, y, Rgba(rgba));
        }
    }
    img
}

/// Over-composites a solid-color layer with coverage `a` onto `dst` (straight alpha).
fn over(dst: &mut [u8; 4], src: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    if oa <= 0.0 {
        return;
    }
    for i in 0..3 {
        let c = src[i] as f32 * a + dst[i] as f32 * da * (1.0 - a);
        dst[i] = (c / oa).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |x: u8, y: u8| -> u8 { (x as f32 + (y as f32 - x as f32) * t).round() as u8 };
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

// --- SDF primitives --------------------------------------------------------

/// Signed distance to a rounded box centered at (cx, cy) with half-extents
/// (hw, hh) and corner radius r; negative inside.
fn sd_rounded_box(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    ox.hypot(oy) + qx.max(qy).min(0.0) - r
}

/// Signed distance to a capsule (segment a→b stroked with radius r).
fn sd_seg(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, r: f32) -> f32 {
    let (vx, vy) = (bx - ax, by - ay);
    let len2 = vx * vx + vy * vy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((px - ax) * vx + (py - ay) * vy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (ax + t * vx, ay + t * vy);
    (px - cx).hypot(py - cy) - r
}

fn sd_circle(px: f32, py: f32, cx: f32, cy: f32, r: f32) -> f32 {
    (px - cx).hypot(py - cy) - r
}

/// 1-px anti-aliased coverage from a signed distance.
fn cov(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Container formats
// ---------------------------------------------------------------------------

fn png_bytes(img: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

/// ICO with classic BMP (BGRA + AND mask) entries for ≤64 px and PNG entries
/// for 128/256 px.
fn write_ico(
    master: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    scaled: &[(u32, ImageBuffer<Rgba<u8>, Vec<u8>>)],
) {
    let sizes: [(u32, bool); 6] = [
        (16, false),
        (32, false),
        (48, false),
        (64, false),
        (128, true),
        (256, true),
    ];
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    for (s, png) in sizes {
        let img: &ImageBuffer<Rgba<u8>, Vec<u8>> = if s == SIZE {
            master
        } else {
            &scaled.iter().find(|(n, _)| *n == s).expect("size").1
        };
        blobs.push(if png {
            png_bytes(img)
        } else {
            bmp_entry(s, img)
        });
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(blobs.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * blobs.len();
    for (i, (s, _png)) in sizes.iter().enumerate() {
        out.push(if *s == 256 { 0 } else { *s as u8 });
        out.push(if *s == 256 { 0 } else { *s as u8 });
        out.push(0); // colors
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(blobs[i].len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += blobs[i].len();
    }
    for b in blobs {
        out.extend_from_slice(&b);
    }
    std::fs::write("assets/icon.ico", out).expect("write ico");
}

fn mask_row_bytes(w: usize) -> usize {
    (w + 31) / 32 * 4
}

fn bmp_entry(s: u32, img: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> Vec<u8> {
    let (w, h) = (s as usize, s as usize);
    let mask = mask_row_bytes(w) * h;
    let mut out = Vec::new();
    // BITMAPINFOHEADER (height doubled: XOR + AND planes)
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as u32).to_le_bytes());
    out.extend_from_slice(&((h as u32) * 2).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((w * h * 4 + mask) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // x pels
    out.extend_from_slice(&0u32.to_le_bytes()); // y pels
    out.extend_from_slice(&0u32.to_le_bytes()); // colors used
    out.extend_from_slice(&0u32.to_le_bytes()); // important
    // Pixel rows bottom-up, BGRA.
    for yy in (0..h).rev() {
        for xx in 0..w {
            let p = img.get_pixel(xx as u32, yy as u32);
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    // AND mask: all zero → alpha channel decides opacity.
    out.extend(std::iter::repeat(0u8).take(mask));
    out
}

/// ICNS: PNG payloads in icp4/icp5/ic12/ic07/ic08/ic09/ic10 chunks.
fn write_icns(
    master: &ImageBuffer<Rgba<u8>, Vec<u8>>,
    scaled: &[(u32, ImageBuffer<Rgba<u8>, Vec<u8>>)],
) {
    let chunks: [(&[u8; 4], u32); 7] = [
        (b"icp4", 16),
        (b"icp5", 32),
        (b"ic12", 64),
        (b"ic07", 128),
        (b"ic08", 256),
        (b"ic09", 512),
        (b"ic10", 1024),
    ];
    let mut body = Vec::new();
    for (tag, s) in chunks {
        let img: &ImageBuffer<Rgba<u8>, Vec<u8>> = if s == SIZE {
            master
        } else {
            &scaled.iter().find(|(n, _)| *n == s).expect("size").1
        };
        let data = png_bytes(img);
        body.extend_from_slice(tag);
        body.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
        body.extend_from_slice(&data);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(&body);
    std::fs::write("assets/icon.icns", out).expect("write icns");
}
