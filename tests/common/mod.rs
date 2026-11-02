//! Shared fixture helpers for the metadata tests: a minimal hand-crafted
//! TIFF/EXIF block and CRC32 for chunk writing.

/// Minimal little-endian TIFF block: header, IFD0 with Orientation (0x0112
/// SHORT, value 1..8) and optionally Make (0x010F ASCII, data area).
pub fn exif_tiff_block(orientation: u16, make: Option<&str>) -> Vec<u8> {
    let make_data = make.map(|m| {
        let mut v = m.as_bytes().to_vec();
        v.push(0); // ASCII strings end with NUL
        v
    });
    let n_entries: u16 = 1 + make_data.is_some() as u16;
    let data_off: u32 = 8 + 2 + 12 * n_entries as u32 + 4;

    let mut v = Vec::new();
    v.extend_from_slice(b"II*\0"); // "II", 42 LE, IFD0 at 8
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&n_entries.to_le_bytes());
    // Entries in ascending tag order (Make 0x010F < Orientation 0x0112).
    if let Some(md) = &make_data {
        v.extend_from_slice(&0x010Fu16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes()); // ASCII
        v.extend_from_slice(&(md.len() as u32).to_le_bytes());
        v.extend_from_slice(&data_off.to_le_bytes()); // > 4 bytes → offset
    }
    v.extend_from_slice(&0x0112u16.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    v.extend_from_slice(&1u32.to_le_bytes()); // count
    v.extend_from_slice(&orientation.to_le_bytes()); // inline value
    v.extend_from_slice(&0u16.to_le_bytes()); // pad
    v.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    if let Some(md) = make_data {
        v.extend_from_slice(&md);
    }
    v
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        table[i as usize] = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Inserts a chunk after the 8-byte signature + IHDR (25 bytes → offset 33).
pub fn png_insert_after_ihdr(png: &[u8], ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
    chunk.extend_from_slice(ty);
    chunk.extend_from_slice(data);
    let mut crc = ty.to_vec();
    crc.extend_from_slice(data);
    chunk.extend_from_slice(&crc32(&crc).to_be_bytes());

    let mut out = Vec::with_capacity(png.len() + chunk.len());
    out.extend_from_slice(&png[..33]);
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&png[33..]);
    out
}

/// Inserts one marker segment right after the JPEG SOI.
#[allow(dead_code)]
pub fn jpeg_with_marker(jpeg: &[u8], marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8, 0xFF, marker];
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}
