//! Encoders for the import cases' input files: ICC profiles, EXIF blocks, PNG, JPEG, TIFF, camera
//! RAW (DNG) and SVG. Everything is built from fixed numbers here, so the files are reproducible
//! and carry no third-party content.

use anyhow::Result;
use std::io::Cursor;

// ---------------------------------------------------------------------------------------------
// ICC profiles

/// A tone curve for a matrix/TRC profile.
#[derive(Clone, Copy)]
pub enum Trc {
    /// `curv` with one entry: a pure power law, stored as u8Fixed8.
    Gamma(u16),
    /// `para` function type 3 with the sRGB constants.
    Srgb,
}

/// An RGB display profile built from chromaticities (white D65), adapted to the D50 PCS with
/// Bradford, as ICC matrix/TRC profiles are. `v4` writes a version 4.3 profile (mluc text, `chad`,
/// D50 `wtpt`); otherwise version 2.1 (textDescription, media white point in `wtpt`).
pub fn icc_profile(description: &str, primaries: [[f64; 2]; 3], trc: Trc, v4: bool) -> Vec<u8> {
    let white = xyz_of([0.3127, 0.3290]);
    let d50 = [0.9642, 1.0, 0.8249];
    let to_xyz = rgb_to_xyz(primaries, white);
    let adapt = bradford(white, d50);
    let columns: Vec<[f64; 3]> = (0..3).map(|c| mul(&adapt, [to_xyz[0][c], to_xyz[1][c], to_xyz[2][c]])).collect();

    let text = |s: &str| -> Vec<u8> {
        if v4 {
            let utf16: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
            let mut b = b"mluc\0\0\0\0".to_vec();
            b.extend(1u32.to_be_bytes());
            b.extend(12u32.to_be_bytes());
            b.extend(b"enUS");
            b.extend((utf16.len() as u32).to_be_bytes());
            b.extend(28u32.to_be_bytes());
            b.extend(utf16);
            b
        } else {
            let mut b = b"text\0\0\0\0".to_vec();
            b.extend(s.as_bytes());
            b.push(0);
            b
        }
    };
    let description_tag = if v4 {
        text(description)
    } else {
        let mut b = b"desc\0\0\0\0".to_vec();
        b.extend((description.len() as u32 + 1).to_be_bytes());
        b.extend(description.as_bytes());
        b.push(0);
        b.extend([0u8; 4 + 4 + 2 + 1 + 67]);
        b
    };
    let xyz = |v: [f64; 3]| -> Vec<u8> {
        let mut b = b"XYZ \0\0\0\0".to_vec();
        for c in v {
            b.extend(s15(c).to_be_bytes());
        }
        b
    };
    let curve = match trc {
        Trc::Gamma(g) => {
            let mut b = b"curv\0\0\0\0".to_vec();
            b.extend(1u32.to_be_bytes());
            b.extend(g.to_be_bytes());
            b
        }
        Trc::Srgb => {
            let mut b = b"para\0\0\0\0".to_vec();
            b.extend(3u16.to_be_bytes());
            b.extend(0u16.to_be_bytes());
            for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
                b.extend(s15(v).to_be_bytes());
            }
            b
        }
    };
    let mut tags: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"desc", description_tag),
        (*b"cprt", text("No copyright, use freely")),
        (*b"wtpt", xyz(if v4 { d50 } else { white })),
        (*b"rXYZ", xyz(columns[0])),
        (*b"gXYZ", xyz(columns[1])),
        (*b"bXYZ", xyz(columns[2])),
        (*b"rTRC", curve.clone()),
        (*b"gTRC", curve.clone()),
        (*b"bTRC", curve),
    ];
    if v4 {
        let mut b = b"sf32\0\0\0\0".to_vec();
        for row in adapt {
            for v in row {
                b.extend(s15(v).to_be_bytes());
            }
        }
        tags.push((*b"chad", b));
    }
    let table_len = 4 + 12 * tags.len();
    let mut data = Vec::new();
    let mut entries = Vec::new();
    for (sig, bytes) in &tags {
        let offset = 128 + table_len + data.len();
        entries.push((*sig, offset as u32, bytes.len() as u32));
        data.extend(bytes);
        while data.len() % 4 != 0 {
            data.push(0);
        }
    }
    let size = 128 + table_len + data.len();
    let mut out = Vec::with_capacity(size);
    out.extend((size as u32).to_be_bytes());
    out.extend([0u8; 4]);
    out.extend(if v4 { 0x0430_0000u32 } else { 0x0210_0000u32 }.to_be_bytes());
    out.extend(b"mntrRGB XYZ ");
    for v in [2026u16, 1, 1, 0, 0, 0] {
        out.extend(v.to_be_bytes());
    }
    out.extend(b"acsp");
    out.extend([0u8; 4 + 4 + 4 + 4 + 8 + 4]);
    for c in d50 {
        out.extend(s15(c).to_be_bytes());
    }
    out.extend([0u8; 4 + 16 + 28]);
    debug_assert_eq!(out.len(), 128);
    out.extend((tags.len() as u32).to_be_bytes());
    for (sig, offset, len) in entries {
        out.extend(sig);
        out.extend(offset.to_be_bytes());
        out.extend(len.to_be_bytes());
    }
    out.extend(data);
    out
}

pub const DISPLAY_P3: [[f64; 2]; 3] = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];
pub const ADOBE_RGB: [[f64; 2]; 3] = [[0.64, 0.33], [0.21, 0.71], [0.15, 0.06]];

fn s15(v: f64) -> i32 {
    (v * 65536.0).round() as i32
}

fn xyz_of(xy: [f64; 2]) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

pub fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

pub fn mat_mul(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]))
}

pub fn invert(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    [
        [c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
        [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
        [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det],
    ]
}

/// Linear RGB to XYZ for these primaries and white.
pub fn rgb_to_xyz(primaries: [[f64; 2]; 3], white: [f64; 3]) -> [[f64; 3]; 3] {
    let p: Vec<[f64; 3]> = primaries.iter().map(|xy| xyz_of(*xy)).collect();
    let m = [[p[0][0], p[1][0], p[2][0]], [p[0][1], p[1][1], p[2][1]], [p[0][2], p[1][2], p[2][2]]];
    let s = mul(&invert(&m), white);
    [0, 1, 2].map(|r| [m[r][0] * s[0], m[r][1] * s[1], m[r][2] * s[2]])
}

fn bradford(from: [f64; 3], to: [f64; 3]) -> [[f64; 3]; 3] {
    let b = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    let (f, t) = (mul(&b, from), mul(&b, to));
    let d = [[t[0] / f[0], 0.0, 0.0], [0.0, t[1] / f[1], 0.0], [0.0, 0.0, t[2] / f[2]]];
    mat_mul(&invert(&b), &mat_mul(&d, &b))
}

// ---------------------------------------------------------------------------------------------
// EXIF

/// A big-endian TIFF block holding only an Orientation tag, as PNG's eXIf chunk and (after the
/// `Exif\0\0` header) a JPEG APP1 segment carry it.
pub fn exif_orientation(orientation: u16) -> Vec<u8> {
    let mut b = b"MM\0\x2a\0\0\0\x08".to_vec();
    b.extend(1u16.to_be_bytes());
    b.extend(0x0112u16.to_be_bytes());
    b.extend(3u16.to_be_bytes());
    b.extend(1u32.to_be_bytes());
    b.extend(orientation.to_be_bytes());
    b.extend([0, 0]);
    b.extend(0u32.to_be_bytes());
    b
}

// ---------------------------------------------------------------------------------------------
// PNG

#[derive(Default)]
pub struct PngOptions {
    pub srgb: bool,
    /// gAMA, as the PNG stores it (gamma × 100000).
    pub gamma: Option<u32>,
    /// cHRM white, red, green, blue as (x, y).
    pub chromaticities: Option<[[f64; 2]; 4]>,
    pub icc: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
    pub palette: Option<(Vec<u8>, Vec<u8>)>,
}

/// A PNG of `data` (big-endian samples for 16-bit), with the chunks `options` asks for.
pub fn png(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8], options: PngOptions) -> Result<Vec<u8>> {
    let mut info = png::Info::with_size(width, height);
    info.color_type = color;
    info.bit_depth = depth;
    if let Some(g) = options.gamma {
        info.source_gamma = Some(png::ScaledFloat::from_scaled(g));
    }
    if let Some([w, r, g, b]) = options.chromaticities {
        let s = |v: [f64; 2]| (png::ScaledFloat::new(v[0] as f32), png::ScaledFloat::new(v[1] as f32));
        info.source_chromaticities = Some(png::SourceChromaticities { white: s(w), red: s(r), green: s(g), blue: s(b) });
    }
    if options.srgb {
        info.srgb = Some(png::SrgbRenderingIntent::Perceptual);
    }
    info.icc_profile = options.icc.map(Into::into);
    info.exif_metadata = options.exif.map(Into::into);
    if let Some((palette, trns)) = options.palette {
        info.palette = Some(palette.into());
        if !trns.is_empty() {
            info.trns = Some(trns.into());
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::with_info(&mut out, info)?;
        encoder.set_compression(png::Compression::Balanced);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(data)?;
        writer.finish()?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// JPEG

pub struct JpegOptions {
    pub quality: u8,
    pub sampling: jpeg_encoder::SamplingFactor,
    pub progressive: bool,
    pub exif: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self { quality: 90, sampling: jpeg_encoder::SamplingFactor::F_1_1, progressive: false, exif: None, icc: None }
    }
}

pub fn jpeg(width: u32, height: u32, color: jpeg_encoder::ColorType, data: &[u8], options: JpegOptions) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, options.quality);
    encoder.set_sampling_factor(options.sampling);
    encoder.set_progressive(options.progressive);
    if let Some(exif) = &options.exif {
        encoder.add_exif_metadata(exif)?;
    }
    if let Some(icc) = &options.icc {
        encoder.add_icc_profile(icc)?;
    }
    encoder.encode(data, width as u16, height as u16, color)?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// TIFF

pub enum TiffPixels<'a> {
    Rgb8(&'a [u8]),
    Rgba8(&'a [u8]),
    Rgb16(&'a [u16]),
    Gray8(&'a [u8]),
}

pub fn tiff(width: u32, height: u32, pixels: TiffPixels, compression: tiff::encoder::Compression, orientation: Option<u16>) -> Result<Vec<u8>> {
    use tiff::encoder::{TiffEncoder, colortype};
    use tiff::tags::{ExtraSamples, Tag};
    let mut out = Cursor::new(Vec::new());
    let mut encoder = TiffEncoder::new(&mut out)?.with_compression(compression);
    macro_rules! write {
        ($ty:ty, $data:expr, $extra:expr) => {{
            let mut image = encoder.new_image::<$ty>(width, height)?;
            if $extra {
                image.extra_samples(&[ExtraSamples::UnassociatedAlpha])?;
            }
            if let Some(o) = orientation {
                image.encoder().write_tag(Tag::Orientation, o)?;
            }
            image.write_data($data)?;
        }};
    }
    match pixels {
        TiffPixels::Rgb8(d) => write!(colortype::RGB8, d, false),
        TiffPixels::Rgba8(d) => write!(colortype::RGB8, d, true),
        TiffPixels::Rgb16(d) => write!(colortype::RGB16, d, false),
        TiffPixels::Gray8(d) => write!(colortype::Gray8, d, false),
    }
    Ok(out.into_inner())
}

// ---------------------------------------------------------------------------------------------
// DNG

/// One TIFF directory entry: tag, field type, count, and the value's bytes (little-endian).
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    bytes: Vec<u8>,
}

fn short(tag: u16, values: &[u16]) -> Entry {
    Entry { tag, kind: 3, count: values.len() as u32, bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() }
}
fn long(tag: u16, values: &[u32]) -> Entry {
    Entry { tag, kind: 4, count: values.len() as u32, bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() }
}
fn byte(tag: u16, values: &[u8]) -> Entry {
    Entry { tag, kind: 1, count: values.len() as u32, bytes: values.to_vec() }
}
fn ascii(tag: u16, text: &str) -> Entry {
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    Entry { tag, kind: 2, count: bytes.len() as u32, bytes }
}
fn rational(tag: u16, values: &[f64]) -> Entry {
    let bytes = values.iter().flat_map(|v| [((v * 10000.0).round() as u32).to_le_bytes(), 10000u32.to_le_bytes()].concat()).collect();
    Entry { tag, kind: 5, count: values.len() as u32, bytes }
}
fn srational(tag: u16, values: &[f64]) -> Entry {
    let bytes = values.iter().flat_map(|v| [((v * 10000.0).round() as i32).to_le_bytes(), 10000i32.to_le_bytes()].concat()).collect();
    Entry { tag, kind: 10, count: values.len() as u32, bytes }
}

/// Appends a directory (its out-of-line values first) to `out`, returning its offset.
fn write_ifd(out: &mut Vec<u8>, mut entries: Vec<Entry>) -> u32 {
    entries.sort_by_key(|e| e.tag);
    let mut fields = Vec::new();
    for e in &entries {
        if e.bytes.len() <= 4 {
            let mut v = e.bytes.clone();
            v.resize(4, 0);
            fields.push(v);
        } else {
            while out.len() % 2 != 0 {
                out.push(0);
            }
            fields.push((out.len() as u32).to_le_bytes().to_vec());
            out.extend(&e.bytes);
        }
    }
    while out.len() % 2 != 0 {
        out.push(0);
    }
    let offset = out.len() as u32;
    out.extend((entries.len() as u16).to_le_bytes());
    for (e, field) in entries.iter().zip(fields) {
        out.extend(e.tag.to_le_bytes());
        out.extend(e.kind.to_le_bytes());
        out.extend(e.count.to_le_bytes());
        out.extend(field);
    }
    out.extend(0u32.to_le_bytes());
    offset
}

/// What a synthetic camera recorded, for a DNG.
pub struct RawCapture {
    pub width: u32,
    pub height: u32,
    /// `Some(pattern)`: one sample per pixel through this 2x2 CFA (0 red, 1 green, 2 blue, row by
    /// row); `None`: three samples per pixel (LinearRaw).
    pub cfa: Option<[u8; 4]>,
    /// 16-bit samples, row by row.
    pub samples: Vec<u16>,
    pub black: u16,
    pub white: u16,
    /// XYZ (D65) to camera, row by row.
    pub color_matrix: [[f64; 3]; 3],
    pub as_shot_neutral: [f64; 3],
    /// A small RGB8 preview for IFD 0, as DNG writers put there.
    pub preview: (u32, u32, Vec<u8>),
}

/// A DNG 1.4 file: IFD 0 holds the preview and the DNG tags, and a SubIFD holds the raw image,
/// uncompressed, as Adobe's converter lays them out.
pub fn dng(capture: &RawCapture) -> Vec<u8> {
    let mut out = b"II\x2a\0\0\0\0\0".to_vec();
    let (pw, ph, preview) = &capture.preview;
    let preview_offset = out.len() as u32;
    out.extend(preview);
    let raw_offset = out.len() as u32;
    out.extend(capture.samples.iter().flat_map(|v| v.to_le_bytes()));
    let samples_per_pixel: u16 = if capture.cfa.is_some() { 1 } else { 3 };
    let mut raw = vec![
        long(254, &[0]),
        long(256, &[capture.width]),
        long(257, &[capture.height]),
        short(258, &vec![16; samples_per_pixel as usize]),
        short(259, &[1]),
        short(262, &[if capture.cfa.is_some() { 32803 } else { 34892 }]),
        long(273, &[raw_offset]),
        short(277, &[samples_per_pixel]),
        long(278, &[capture.height]),
        long(279, &[capture.samples.len() as u32 * 2]),
        short(284, &[1]),
        short(50714, &[capture.black]),
        short(50717, &[capture.white]),
    ];
    if let Some(pattern) = capture.cfa {
        raw.push(short(33421, &[2, 2]));
        raw.push(byte(33422, &pattern));
    }
    let raw_ifd = write_ifd(&mut out, raw);
    let m = capture.color_matrix;
    let ifd0 = write_ifd(
        &mut out,
        vec![
            long(254, &[1]),
            long(256, &[*pw]),
            long(257, &[*ph]),
            short(258, &[8, 8, 8]),
            short(259, &[1]),
            short(262, &[2]),
            ascii(271, "Compositor"),
            ascii(272, "Parity Camera"),
            long(273, &[preview_offset]),
            short(274, &[1]),
            short(277, &[3]),
            long(278, &[*ph]),
            long(279, &[preview.len() as u32]),
            short(284, &[1]),
            ascii(305, "Compositor parity corpus"),
            long(330, &[raw_ifd]),
            byte(50706, &[1, 4, 0, 0]),
            byte(50707, &[1, 1, 0, 0]),
            ascii(50708, "Compositor Parity Camera"),
            srational(50721, &m.concat()),
            rational(50728, &capture.as_shot_neutral),
            short(50778, &[21]),
        ],
    );
    out[4..8].copy_from_slice(&ifd0.to_le_bytes());
    out
}

// ---------------------------------------------------------------------------------------------
// GIF

/// The smallest GIF: one white pixel. The app doesn't import GIFs.
pub const GIF_1X1: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
