//! HDR → SDR handling for JPEG XL decoding.
//!
//! JPEG XL stores PQ (SMPTE ST 2084) / HLG (BT.2100) content in the Rec.2100
//! gamut. For display we convert to display-linear nits, tone-map luminance to
//! the SDR range (extended Reinhard with the content's peak as the shoulder),
//! convert Rec.2100 → sRGB primaries and apply the sRGB OETF.
//!
//! Constants follow BT.2100 / BT.2390 / BT.2408; the SDR reference white is
//! 203 nits per BT.2408.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdrTf {
    Pq,
    Hlg,
}

impl HdrTf {
    pub fn as_str(self) -> &'static str {
        match self {
            HdrTf::Pq => "PQ",
            HdrTf::Hlg => "HLG",
        }
    }
}

/// BT.2408 reference white for SDR content in an HDR container.
pub const SDR_WHITE_NITS: f32 = 203.0;

/// BT.2020/2100 luma weights (the Rec.2100 primaries equal BT.2020's).
pub fn luma2100(rgb: [f32; 3]) -> f32 {
    0.2627 * rgb[0] + 0.6780 * rgb[1] + 0.0593 * rgb[2]
}

/// PQ (SMPTE ST 2084) EOTF: signal [0,1] → absolute luminance in nits.
pub fn pq_eotf(e: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let e = e.clamp(0.0, 1.0);
    let p = e.powf(1.0 / M2);
    let num = (p - C1).max(0.0);
    let den = (C2 - C3 * p).max(1e-6);
    10000.0 * (num / den).powf(1.0 / M1)
}

/// HLG inverse OETF: signal [0,1] → scene-linear [0,1] (relative to HLG
/// scene reference white).
pub fn hlg_inverse_oetf(e: f32) -> f32 {
    const A: f32 = 0.17883277;
    const B: f32 = 0.28466892;
    const C: f32 = 0.55991073;
    let e = e.clamp(0.0, 1.0);
    if e <= 0.5 {
        e * e / 3.0
    } else {
        (((e - C) / A).exp() + B) / 12.0
    }
}

/// Converts one PQ/HLG-encoded Rec.2100 pixel into display-linear nits.
///
/// `peak_nits` is the display's (or content's) assumed peak for the HLG OOTF;
/// ignored for PQ, whose EOTF is absolute.
pub fn encoded_to_nits(rgb_e: [f32; 3], tf: HdrTf, peak_nits: f32) -> [f32; 3] {
    match tf {
        HdrTf::Pq => [pq_eotf(rgb_e[0]), pq_eotf(rgb_e[1]), pq_eotf(rgb_e[2])],
        HdrTf::Hlg => {
            let scene: [f32; 3] = rgb_e.map(hlg_inverse_oetf);
            let ys = luma2100(scene).max(1e-6);
            // Inverse OOTF for a `peak_nits` display, system gamma 1.2:
            // Ld = α·Ys^(γ−1)·Ys  →  per-channel scale α·Ys^(γ−1).
            let scale = ys.powf(1.2 - 1.0) * peak_nits.max(1.0);
            [scene[0] * scale, scene[1] * scale, scene[2] * scale]
        }
    }
}

/// sRGB transfer function (OETF), linear [0,1] → encoded [0,1].
pub fn srgb_oetf(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Rec.2100 (linear, nits) → tone-mapped, gamut-converted, sRGB-encoded [0,1]³.
///
/// Luminance is tone-mapped with extended Reinhard so that `peak_nits` maps to
/// SDR white; hue/saturation are preserved by scaling channels with a common
/// factor, then the gamut conversion clamps out-of-sRGB colors.
pub fn tonemap_to_srgb(rgb_nits: [f32; 3], peak_nits: f32) -> [f32; 3] {
    let y = luma2100(rgb_nits);
    let y_sdr = (y / SDR_WHITE_NITS).max(1e-6);
    let w = (peak_nits / SDR_WHITE_NITS).max(y_sdr).max(1.0);
    // Extended Reinhard: maps [0, w] → [0, 1).
    let y_tm = (y_sdr * (1.0 + y_sdr / (w * w)) / (1.0 + y_sdr)).min(1.0);
    let scale = y_tm / y_sdr;

    let lin = [
        rgb_nits[0] / SDR_WHITE_NITS * scale,
        rgb_nits[1] / SDR_WHITE_NITS * scale,
        rgb_nits[2] / SDR_WHITE_NITS * scale,
    ];

    // Rec.2100 → Rec.709/sRGB linear primaries.
    let srgb_lin = [
        1.6605 * lin[0] - 0.5876 * lin[1] - 0.0728 * lin[2],
        -0.1246 * lin[0] + 1.1329 * lin[1] - 0.0083 * lin[2],
        -0.0182 * lin[0] - 0.1006 * lin[1] + 1.1187 * lin[2],
    ];

    srgb_lin.map(srgb_oetf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn pq_endpoints_and_reference_points() {
        assert!(approx(pq_eotf(0.0), 0.0, 1e-3));
        assert!(approx(pq_eotf(1.0), 10000.0, 1.0));
        // Well-known values: PQ 0.50 ≈ 92 nits, PQ 0.75 ≈ 1000 nits.
        assert!(approx(pq_eotf(0.50), 92.0, 2.0), "{}", pq_eotf(0.50));
        assert!(approx(pq_eotf(0.75), 1000.0, 20.0), "{}", pq_eotf(0.75));
    }

    #[test]
    fn pq_is_monotonic() {
        let mut prev = -1.0;
        for i in 0..=100 {
            let y = pq_eotf(i as f32 / 100.0);
            assert!(y >= prev, "PQ not monotonic at {i}");
            prev = y;
        }
    }

    #[test]
    fn hlg_endpoints() {
        assert!(approx(hlg_inverse_oetf(0.0), 0.0, 1e-6));
        assert!(approx(hlg_inverse_oetf(0.5), 1.0 / 12.0, 1e-4));
        assert!(approx(hlg_inverse_oetf(1.0), 1.0, 1e-3));
    }

    #[test]
    fn tonemap_maps_white_and_peak() {
        // SDR reference white lands around 75% signal (upper-mid gray) with a
        // 1000-nit peak — standard for HDR→SDR mappings.
        let white = tonemap_to_srgb([SDR_WHITE_NITS; 3], 1000.0);
        for c in white {
            assert!(approx(c, 0.75, 0.05), "{c}");
        }
        // Black stays black.
        let black = tonemap_to_srgb([0.0; 3], 1000.0);
        assert!(black.iter().all(|c| *c < 1e-4));
        // The declared peak maps to full white.
        let peak = tonemap_to_srgb([1000.0; 3], 1000.0);
        assert!(peak.iter().all(|c| *c > 0.98), "{peak:?}");
    }

    #[test]
    fn tonemap_preserves_neutral_hue() {
        let g = tonemap_to_srgb([500.0, 500.0, 500.0], 1000.0);
        assert!(approx(g[0], g[1], 1e-4) && approx(g[1], g[2], 1e-4));
    }

    #[test]
    fn srgb_oetf_endpoints() {
        assert!(approx(srgb_oetf(0.0), 0.0, 1e-6));
        assert!(approx(srgb_oetf(1.0), 1.0, 1e-6));
        assert!(approx(srgb_oetf(0.5), 0.735, 0.01));
    }
}
