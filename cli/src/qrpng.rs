//! A QR code as a PNG, for a phone screen. The `qrcode` crate gives the
//! modules; the PNG is written here by hand — one grey channel, one IDAT,
//! zlib from flate2 — because pulling in an image library for a two-colour
//! bitmap is the wrong trade.

use std::io::Write;

const SCALE: usize = 8;
const QUIET: usize = 4;

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// The PNG bytes for `text`, or None if the text is too long for a QR code.
pub fn qr_png(text: &str) -> Option<Vec<u8>> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let modules = code.width();
    let colors = code.to_colors();
    let side = (modules + 2 * QUIET) * SCALE;
    // One filter byte (0, none) per row, then one grey byte per pixel.
    let mut raw = Vec::with_capacity((side + 1) * side);
    for y in 0..side {
        raw.push(0);
        let my = y / SCALE;
        for x in 0..side {
            let mx = x / SCALE;
            let dark = my >= QUIET
                && mx >= QUIET
                && my < QUIET + modules
                && mx < QUIET + modules
                && colors[(my - QUIET) * modules + (mx - QUIET)] == qrcode::Color::Dark;
            raw.push(if dark { 0x00 } else { 0xFF });
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).ok()?;
    let idat = z.finish().ok()?;
    let mut out = Vec::with_capacity(idat.len() + 64);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(side as u32).to_be_bytes());
    ihdr.extend_from_slice(&(side as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // 8-bit greyscale, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_with_the_right_shape_comes_out() {
        let png = qr_png("marigoldpay:test").expect("short text fits");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        let side = u32::from_be_bytes([png[16], png[17], png[18], png[19]]) as usize;
        assert_eq!(side, u32::from_be_bytes([png[20], png[21], png[22], png[23]]) as usize);
        assert!(side >= (21 + 8) * SCALE, "at least a version-1 code with its quiet zone");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        // For eyes: MARIGOLD_QR_PNG=/path writes it out.
        if let Ok(path) = std::env::var("MARIGOLD_QR_PNG") {
            std::fs::write(path, &png).unwrap();
        }
    }
}
