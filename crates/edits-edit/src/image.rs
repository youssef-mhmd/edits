//! Pictures for stamps: JPEG files are embedded as they are (DCTDecode);
//! PNG files are decoded and stored compressed, with transparency as a soft mask.

use std::io::Write;

use serde::{Deserialize, Serialize};

/// An image ready for a PDF image XObject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfImage {
    pub width: u32,
    pub height: u32,
    /// DeviceRGB, DeviceGray or DeviceCMYK.
    pub color_space: String,
    /// DCTDecode (JPEG data as is) or FlateDecode.
    pub filter: String,
    #[serde(with = "b64")]
    pub data: Vec<u8>,
    /// Flate-compressed 8-bit alpha, if the picture has transparency.
    #[serde(with = "b64_opt", default)]
    pub alpha: Option<Vec<u8>>,
}

/// Largest picture accepted, in pixels per side.
const MAX_SIDE: u32 = 12_000;

impl PdfImage {
    /// Reads a JPEG or PNG file's bytes.
    pub fn from_bytes(b: &[u8]) -> Result<Self, String> {
        if b.starts_with(&[0xFF, 0xD8]) {
            jpeg(b)
        } else if b.starts_with(b"\x89PNG") {
            png_image(b)
        } else {
            Err("only JPEG and PNG pictures can be added".into())
        }
    }
}

fn jpeg(b: &[u8]) -> Result<PdfImage, String> {
    // Find the frame header (SOFn) for size and components.
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) || marker == 0xFF {
            i += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            let comps = b[i + 9];
            if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
                return Err("the picture's size is not supported".into());
            }
            let cs = match comps {
                1 => "DeviceGray",
                4 => "DeviceCMYK",
                _ => "DeviceRGB",
            };
            return Ok(PdfImage {
                width: w,
                height: h,
                color_space: cs.into(),
                filter: "DCTDecode".into(),
                data: b.to_vec(),
                alpha: None,
            });
        }
        i += 2 + len;
    }
    Err("the JPEG file is damaged".into())
}

fn deflate(d: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = z.write_all(d);
    z.finish().unwrap_or_default()
}

fn png_image(b: &[u8]) -> Result<PdfImage, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(b));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().map_err(|e| format!("the PNG file cannot be read: {e}"))?;
    let (w, h) = (r.info().width, r.info().height);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err("the picture's size is not supported".into());
    }
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).map_err(|e| format!("the PNG file cannot be read: {e}"))?;
    let px = &buf[..info.buffer_size()];
    let (channels, cs) = match info.color_type {
        png::ColorType::Grayscale => (1, "DeviceGray"),
        png::ColorType::GrayscaleAlpha => (2, "DeviceGray"),
        png::ColorType::Rgb => (3, "DeviceRGB"),
        png::ColorType::Rgba => (4, "DeviceRGB"),
        png::ColorType::Indexed => return Err("unexpected palette image".into()),
    };
    let has_alpha = channels == 2 || channels == 4;
    let colour = if has_alpha { channels - 1 } else { channels };
    let mut col = Vec::with_capacity((w * h) as usize * colour);
    let mut alpha = Vec::with_capacity(if has_alpha { (w * h) as usize } else { 0 });
    for p in px.chunks_exact(channels) {
        col.extend_from_slice(&p[..colour]);
        if has_alpha {
            alpha.push(p[colour]);
        }
    }
    let opaque = alpha.iter().all(|a| *a == 255);
    Ok(PdfImage {
        width: w,
        height: h,
        color_space: cs.into(),
        filter: "FlateDecode".into(),
        data: deflate(&col),
        alpha: (has_alpha && !opaque).then(|| deflate(&alpha)),
    })
}

/// Bytes as base64 text in JSON (crash-recovery files).
mod b64 {
    use serde::{Deserialize, Deserializer, Serializer};

    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(d: &[u8]) -> String {
        let mut s = String::with_capacity(d.len().div_ceil(3) * 4);
        for c in d.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for k in 0..4 {
                if k <= c.len() {
                    s.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut acc = 0u32;
        let mut bits = 0;
        for c in s.bytes().filter(|c| *c != b'=') {
            let v = T.iter().position(|t| *t == c)? as u32;
            acc = acc << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Some(out)
    }

    pub fn serialize<S: Serializer>(d: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(d))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).ok_or_else(|| serde::de::Error::custom("bad base64"))
    }

    #[cfg(test)]
    #[test]
    fn round_trip() {
        for d in [&b""[..], b"a", b"ab", b"abc", b"\xff\x00\x10hello"] {
            assert_eq!(decode(&encode(d)).unwrap(), d);
        }
        assert_eq!(encode(b"Man"), "TWFu");
    }
}

mod b64_opt {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_some(&super::b64::encode(d)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|s| super::b64::decode(&s).ok_or_else(|| serde::de::Error::custom("bad base64")))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_png_with_alpha() {
        // A 2x1 RGBA PNG: one opaque red pixel, one transparent.
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, 2, 1);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[255, 0, 0, 255, 0, 0, 0, 0]).unwrap();
        }
        let img = PdfImage::from_bytes(&out).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.color_space, "DeviceRGB");
        assert!(img.alpha.is_some());
        assert!(PdfImage::from_bytes(b"GIF89a").is_err());
    }

    #[test]
    fn reads_jpeg_header() {
        // SOI, SOF0 (8-bit, 3x2, 3 components), EOI.
        let j = [
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x02, 0x00, 0x03, 0x03, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1,
            0xFF, 0xD9,
        ];
        let img = PdfImage::from_bytes(&j).unwrap();
        assert_eq!((img.width, img.height, img.filter.as_str()), (3, 2, "DCTDecode"));
    }
}
