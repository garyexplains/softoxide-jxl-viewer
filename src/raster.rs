//! Pixel buffer for the viewer: straight (non-premultiplied) RGB or RGBA
//! samples, 8 or 16 bits per component, row-major interleaved.
//!
//! All pixels are stored full-range for the channel depth (0..255 or
//! 0..65535), regardless of the source file's original bit depth (e.g. 12-bit
//! PNG). Depth is tracked so lossless exports can keep fidelity.

use eframe::egui::ColorImage;

#[derive(Debug, Clone)]
pub enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

impl Samples {
    pub fn zeroed_like(&self, len: usize) -> Samples {
        match self {
            Samples::U8(_) => Samples::U8(vec![0; len]),
            Samples::U16(_) => Samples::U16(vec![0; len]),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Samples::U8(v) => v.len(),
            Samples::U16(v) => v.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Lanczos-3 etc. exposed by fast_image_resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeFilter {
    Nearest,
    Box,
    Bilinear,
    CatmullRom,
    #[default]
    Lanczos3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    /// Canonical sampling depth of `data` (8 or 16). Higher than the source's
    /// native depth means we upscaled (e.g. a 10-bit JXL expanded to u16).
    pub bits: u8,
    pub data: Samples,
}

impl Raster {
    pub fn from_samples(
        width: u32,
        height: u32,
        has_alpha: bool,
        bits: u8,
        data: Samples,
    ) -> Raster {
        let channels = if has_alpha { 4 } else { 3 };
        assert_eq!(data.len(), width as usize * height as usize * channels);
        Raster {
            width,
            height,
            has_alpha,
            bits,
            data: if bits == 16 {
                match data {
                    Samples::U8(_) => upscale_u8_to_u16(&data, width, height, has_alpha),
                    other => other,
                }
            } else {
                match data {
                    Samples::U16(_) => downscale_u16_to_u8(&data, width, height, has_alpha),
                    other => other,
                }
            },
        }
    }

    pub fn from_rgba8(width: u32, height: u32, has_alpha: bool, data: Vec<u8>) -> Raster {
        Raster::from_samples(width, height, has_alpha, 8, Samples::U8(data))
    }

    pub fn from_rgba16(width: u32, height: u32, has_alpha: bool, data: Vec<u16>) -> Raster {
        Raster::from_samples(width, height, has_alpha, 16, Samples::U16(data))
    }

    pub fn channels(&self) -> usize {
        if self.has_alpha { 4 } else { 3 }
    }

    /// Pixel coordinates for the image bounds.
    pub fn bounds(&self) -> RasterRect {
        RasterRect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }

    /// Crops to the intersection of `rect` and the image bounds.
    pub fn crop(&self, rect: RasterRect) -> Raster {
        let x0 = rect.x.min(self.width.saturating_sub(1));
        let y0 = rect.y.min(self.height.saturating_sub(1));
        let w = rect.width.min(self.width - x0).max(1);
        let h = rect.height.min(self.height - y0).max(1);
        let ch = self.channels();
        let mut out = self.data.zeroed_like(w as usize * h as usize * ch);
        let row_bytes = w as usize * ch;

        for row in 0..h {
            let src_off = ((y0 + row) as usize * self.width as usize + x0 as usize) * ch;
            let dst_off = row as usize * row_bytes;
            match (&self.data, &mut out) {
                (Samples::U8(s), Samples::U8(d)) => d[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&s[src_off..src_off + row_bytes]),
                (Samples::U16(s), Samples::U16(d)) => d[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&s[src_off..src_off + row_bytes]),
                _ => unreachable!("sample type changed"),
            }
        }

        Raster {
            width: w,
            height: h,
            has_alpha: self.has_alpha,
            bits: self.bits,
            data: out,
        }
    }

    /// Rotates 90° clockwise.
    pub fn rotate_cw(&self) -> Raster {
        let ch = self.channels();
        let mut out = self.data.zeroed_like(self.data.len());
        for y in 0..self.height {
            for x in 0..self.width {
                let dst_x = self.height - 1 - y;
                let dst_y = x;
                let src = (y as usize * self.width as usize + x as usize) * ch;
                let dst = (dst_y as usize * self.height as usize + dst_x as usize) * ch;
                copy_pixel(&self.data, src, &mut out, dst, ch);
            }
        }
        Raster {
            width: self.height,
            height: self.width,
            has_alpha: self.has_alpha,
            bits: self.bits,
            data: out,
        }
    }

    /// Rotates 90° counter-clockwise.
    pub fn rotate_ccw(&self) -> Raster {
        let ch = self.channels();
        let mut out = self.data.zeroed_like(self.data.len());
        for y in 0..self.height {
            for x in 0..self.width {
                let dst_x = y;
                let dst_y = self.width - 1 - x;
                let src = (y as usize * self.width as usize + x as usize) * ch;
                let dst = (dst_y as usize * self.height as usize + dst_x as usize) * ch;
                copy_pixel(&self.data, src, &mut out, dst, ch);
            }
        }
        Raster {
            width: self.height,
            height: self.width,
            has_alpha: self.has_alpha,
            bits: self.bits,
            data: out,
        }
    }

    /// Rotates 180°.
    pub fn rotate_180(&self) -> Raster {
        let ch = self.channels();
        let mut out = self.data.zeroed_like(self.data.len());
        let n = (self.width as usize * self.height as usize) * ch;
        for i in 0..n {
            let j = n - 1 - i;
            write_sample(&self.data, i, &mut out, j);
        }
        Raster {
            width: self.width,
            height: self.height,
            has_alpha: self.has_alpha,
            bits: self.bits,
            data: out,
        }
    }

    pub fn flip_h(&self) -> Raster {
        let ch = self.channels();
        let mut out = self.data.zeroed_like(self.data.len());
        for y in 0..self.height {
            for x in 0..self.width {
                let src = (y as usize * self.width as usize + x as usize) * ch;
                let dst = (y as usize * self.width as usize + (self.width - 1 - x) as usize) * ch;
                copy_pixel(&self.data, src, &mut out, dst, ch);
            }
        }
        Raster {
            width: self.width,
            height: self.height,
            has_alpha: self.has_alpha,
            bits: self.bits,
            data: out,
        }
    }

    pub fn flip_v(&self) -> Raster {
        let ch = self.channels();
        let mut out = self.data.zeroed_like(self.data.len());
        for y in 0..self.height {
            for x in 0..self.width {
                let src = (y as usize * self.width as usize + x as usize) * ch;
                let dst = ((self.height - 1 - y) as usize * self.width as usize + x as usize) * ch;
                copy_pixel(&self.data, src, &mut out, dst, ch);
            }
        }
        Raster {
            width: self.width,
            height: self.height,
            has_alpha: self.has_alpha,
            bits: self.bits,
            data: out,
        }
    }

    /// Resizes with the given filter. Result keeps the sample depth and alpha.
    pub fn resized(&self, new_w: u32, new_h: u32, filter: ResizeFilter) -> Result<Raster, String> {
        if new_w == 0 || new_h == 0 {
            return Err("size must be at least 1×1".into());
        }
        use fast_image_resize::{
            FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image,
            images::ImageRef,
        };

        let pixel_type = if self.bits == 16 {
            if self.has_alpha {
                PixelType::U16x4
            } else {
                PixelType::U16x3
            }
        } else if self.has_alpha {
            PixelType::U8x4
        } else {
            PixelType::U8x3
        };

        let src: ImageRef<'_> =
            ImageRef::new(self.width, self.height, as_bytes(&self.data), pixel_type)
                .map_err(|e| format!("bad source image: {e}"))?;
        let mut dst = Image::new(new_w, new_h, pixel_type);

        let alg = match filter {
            ResizeFilter::Nearest => ResizeAlg::Nearest,
            ResizeFilter::Box => ResizeAlg::Convolution(FilterType::Box),
            ResizeFilter::Bilinear => ResizeAlg::Convolution(FilterType::Bilinear),
            ResizeFilter::CatmullRom => ResizeAlg::Convolution(FilterType::CatmullRom),
            ResizeFilter::Lanczos3 => ResizeAlg::Convolution(FilterType::Lanczos3),
        };

        let mut resizer = Resizer::new();
        resizer
            .resize(&src, &mut dst, Some(&ResizeOptions::new().resize_alg(alg)))
            .map_err(|e| format!("resize failed: {e}"))?;

        let dst_bytes = dst.buffer().to_vec();
        let data = if self.bits == 16 {
            let mut v16 = vec![0u16; dst_bytes.len() / 2];
            v16.copy_from_slice(bytemuck_helpers::slice_u8_as_u16(&dst_bytes));
            Samples::U16(v16)
        } else {
            Samples::U8(dst_bytes)
        };
        Ok(Raster {
            width: new_w,
            height: new_h,
            has_alpha: self.has_alpha,
            bits: self.bits,
            data,
        })
    }

    /// Samples flattened over the given background (used for alpha-less saves
    /// like JPEG/BMP). Result has 3 channels, same bit depth.
    pub fn flattened_over(&self, bg: [u16; 3]) -> Raster {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = self.data.zeroed_like(w * h * 3);
        match (&self.data, &mut out) {
            (Samples::U8(s), Samples::U8(d)) => {
                for i in 0..w * h {
                    if self.has_alpha {
                        let a = s[i * 4 + 3];
                        let bg8: [u8; 3] = [
                            (bg[0].min(255) as u8),
                            (bg[1].min(255) as u8),
                            (bg[2].min(255) as u8),
                        ];
                        for c in 0..3 {
                            d[i * 3 + c] = (s[i * 4 + c] as u16 * (a as u16) / 255
                                + bg8[c] as u16 * (255 - a as u16) / 255)
                                as u8;
                        }
                    } else {
                        d[i * 3..i * 3 + 3].copy_from_slice(&s[i * 3..i * 3 + 3]);
                    }
                }
            }
            (Samples::U16(s), Samples::U16(d)) => {
                for i in 0..w * h {
                    if self.has_alpha {
                        let a = s[i * 4 + 3];
                        for c in 0..3 {
                            d[i * 3 + c] = (s[i * 4 + c] as u32 * a as u32 / 65535
                                + bg[c] as u32 * (65535 - a as u32) / 65535)
                                as u16;
                        }
                    } else {
                        d[i * 3..i * 3 + 3].copy_from_slice(&s[i * 3..i * 3 + 3]);
                    }
                }
            }
            _ => unreachable!("sample type changed"),
        }
        Raster {
            width: self.width,
            height: self.height,
            has_alpha: false,
            bits: self.bits,
            data: out,
        }
    }

    /// Drops the alpha channel (alpha is discarded, not composited).
    pub fn without_alpha(&self) -> Raster {
        if !self.has_alpha {
            return self.clone();
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = self.data.zeroed_like(w * h * 3);
        match (&self.data, &mut out) {
            (Samples::U8(s), Samples::U8(d)) => {
                for i in 0..w * h {
                    d[i * 3..i * 3 + 3].copy_from_slice(&s[i * 4..i * 4 + 3]);
                }
            }
            (Samples::U16(s), Samples::U16(d)) => {
                for i in 0..w * h {
                    d[i * 3..i * 3 + 3].copy_from_slice(&s[i * 4..i * 4 + 3]);
                }
            }
            _ => unreachable!("sample type changed"),
        }
        Raster {
            width: self.width,
            height: self.height,
            has_alpha: false,
            bits: self.bits,
            data: out,
        }
    }

    /// Texture-ready 8-bit RGBA with alpha channel; opaque pixels get alpha 255.
    pub fn to_rgba8_rgba(&self) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = vec![255u8; w * h * 4];
        match &self.data {
            Samples::U8(s) => {
                for i in 0..w * h {
                    if self.has_alpha {
                        out[i * 4..i * 4 + 4].copy_from_slice(&s[i * 4..i * 4 + 4]);
                    } else {
                        out[i * 4..i * 4 + 3].copy_from_slice(&s[i * 3..i * 3 + 3]);
                    }
                }
            }
            Samples::U16(s) => {
                for i in 0..w * h {
                    for c in 0..3 {
                        out[i * 4 + c] = u16_to_u8(s[i * (if self.has_alpha { 4 } else { 3 }) + c]);
                    }
                    if self.has_alpha {
                        out[i * 4 + 3] = u16_to_u8(s[i * 4 + 3]);
                    }
                }
            }
        }
        out
    }

    /// egui texture image using unmultiplied alpha (straight).
    pub fn to_color_image(&self) -> ColorImage {
        ColorImage::from_rgba_unmultiplied(
            [self.width as usize, self.height as usize],
            &self.to_rgba8_rgba(),
        )
    }
}

/// ColorImage abstraction moved out to keep raster.rs format-neutral for tests.
pub mod bytemuck_helpers {
    pub fn slice_u8_as_u16(bytes: &[u8]) -> &[u16] {
        // fast_image_resize hands back aligned plain Vec<u8>; treat as u16x4 data.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2) }
    }
}

#[inline]
fn u16_to_u8(v: u16) -> u8 {
    ((v as u16 + 128) * 255 / 65535) as u8
}

fn upscale_u8_to_u16(data: &Samples, width: u32, height: u32, has_alpha: bool) -> Samples {
    let ch = if has_alpha { 4 } else { 3 };
    if let Samples::U8(s) = data {
        let mut out = vec![0u16; s.len()];
        for (i, v) in s.iter().enumerate() {
            out[i] = (*v as u16) * 257;
        }
        let _ = (width, height, ch);
        Samples::U16(out)
    } else {
        assert!(false, "upscale called on u16 data");
        unreachable!()
    }
}

fn downscale_u16_to_u8(data: &Samples, width: u32, height: u32, has_alpha: bool) -> Samples {
    let ch = if has_alpha { 4 } else { 3 };
    if let Samples::U16(s) = data {
        let mut out = vec![0u8; s.len()];
        for (i, v) in s.iter().enumerate() {
            out[i] = u16_to_u8(*v);
        }
        let _ = (width, height, ch);
        Samples::U8(out)
    } else {
        assert!(false, "downscale called on u8 data");
        unreachable!()
    }
}

fn as_bytes(data: &Samples) -> &[u8] {
    match data {
        Samples::U8(v) => v.as_slice(),
        Samples::U16(v) => unsafe {
            std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2)
        },
    }
}

#[inline]
fn copy_pixel(src: &Samples, src_off: usize, dst: &mut Samples, dst_off: usize, ch: usize) {
    match (src, &mut *dst) {
        (Samples::U8(s), Samples::U8(d)) => {
            d[dst_off..dst_off + ch].copy_from_slice(&s[src_off..src_off + ch]);
        }
        (Samples::U16(s), Samples::U16(d)) => {
            d[dst_off..dst_off + ch].copy_from_slice(&s[src_off..src_off + ch]);
        }
        _ => unreachable!("sample type changed"),
    }
}

#[inline]
fn write_sample(src: &Samples, src_i: usize, dst: &mut Samples, dst_i: usize) {
    match (src, &mut *dst) {
        (Samples::U8(s), Samples::U8(d)) => d[dst_i] = s[src_i],
        (Samples::U16(s), Samples::U16(d)) => d[dst_i] = s[src_i],
        _ => unreachable!("sample type changed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_and_rotate() {
        // 3x2 pixels, RGB
        let mut data = Vec::new();
        for y in 0..2 {
            for x in 0..3 {
                data.push((x * 10 + y) as u8);
                data.push(0);
                data.push(0);
            }
        }
        let r = Raster::from_rgba8(3, 2, false, data.clone());
        let c = r.crop(RasterRect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        });
        assert_eq!(c.width, 2);
        assert_eq!(c.height, 2);
        assert_eq!(c.data.len(), 12);

        let cc = r.rotate_cw();
        assert_eq!(cc.width, 2);
        assert_eq!(cc.height, 3);

        let ccw = r.rotate_ccw();
        assert_eq!(ccw.width, 2);
        assert_eq!(ccw.height, 3);

        let rot180 = r.rotate_180();
        assert_eq!(rot180.data.len(), r.data.len());

        let flipped = r.flip_v();
        assert_eq!(flipped.width, 3);
        assert_eq!(flipped.height, 2);
    }

    #[test]
    fn resize_sizes() {
        let mut data = Vec::new();
        for i in 0..64 {
            data.push(i as u8);
            data.push(0);
            data.push(255);
        }
        let r = Raster::from_rgba8(8, 8, false, data);
        let resized = r.resized(4, 3, ResizeFilter::Lanczos3).unwrap();
        assert_eq!(resized.width, 4);
        assert_eq!(resized.height, 3);
        assert_eq!(resized.channels(), 3);
        assert_eq!(resized.data.len(), 36);
    }
}
