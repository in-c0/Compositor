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
    /// A small RGB8 preview for IFD 0, as DNG writers put there. Without one, IFD 0 is the raw
    /// image itself.
    pub preview: Option<(u32, u32, Vec<u8>)>,
    /// Which optional DNG tags to write.
    pub tags: DngTags,
}

/// Optional DNG tags, to find out which ones Apple's RAW engine needs.
#[derive(Clone, Copy, Default)]
pub struct DngTags {
    /// CFAPlaneColor and CFALayout.
    pub cfa_layout: bool,
    /// DefaultCropOrigin and DefaultCropSize (8 pixels in from each edge) and ActiveArea.
    pub crop: bool,
    /// AnalogBalance, BaselineExposure, BaselineNoise, BaselineSharpness, BayerGreenSplit,
    /// LinearResponseLimit, DefaultScale and BestQualityScale at their neutral values, and a
    /// second calibration (Standard Light A) with the same matrix.
    pub rendering: bool,
    /// BlackLevel and WhiteLevel with one value per sample (LinearRaw) or BlackLevelRepeatDim
    /// 2 x 2 with four black levels (CFA), as the specification counts them.
    pub full_levels: bool,
}

/// A DNG 1.4 file, uncompressed. With a preview, IFD 0 holds it and the DNG tags and a SubIFD
/// holds the raw image, as Adobe's converter lays them out; without one, IFD 0 is the raw image.
pub fn dng(capture: &RawCapture) -> Vec<u8> {
    let mut out = b"II\x2a\0\0\0\0\0".to_vec();
    let preview_offset = out.len() as u32;
    if let Some((_, _, preview)) = &capture.preview {
        out.extend(preview);
    }
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
    ];
    let t = capture.tags;
    match (t.full_levels, capture.cfa.is_some()) {
        (false, _) => {
            raw.push(short(50714, &[capture.black]));
            raw.push(short(50717, &[capture.white]));
        }
        (true, true) => {
            raw.push(short(50713, &[2, 2]));
            raw.push(short(50714, &[capture.black; 4]));
            raw.push(short(50717, &[capture.white]));
        }
        (true, false) => {
            raw.push(short(50714, &[capture.black; 3]));
            raw.push(short(50717, &[capture.white; 3]));
        }
    }
    if let Some(pattern) = capture.cfa {
        raw.push(short(33421, &[2, 2]));
        raw.push(byte(33422, &pattern));
        if t.cfa_layout {
            raw.push(byte(50710, &[0, 1, 2]));
            raw.push(short(50711, &[1]));
        }
    }
    if t.crop {
        raw.push(long(50719, &[8, 8]));
        raw.push(long(50720, &[capture.width - 16, capture.height - 16]));
        raw.push(long(50829, &[0, 0, capture.height, capture.width]));
    }
    if t.rendering {
        raw.push(rational(50718, &[1.0, 1.0]));
        raw.push(rational(50780, &[1.0]));
        if capture.cfa.is_some() {
            raw.push(long(50733, &[0]));
        }
    }
    let m = capture.color_matrix;
    let dng_tags = vec![
        ascii(271, "Compositor"),
        ascii(272, "Parity Camera"),
        short(274, &[1]),
        ascii(305, "Compositor parity corpus"),
        byte(50706, &[1, 4, 0, 0]),
        byte(50707, &[1, 1, 0, 0]),
        ascii(50708, "Compositor Parity Camera"),
        srational(50721, &m.concat()),
        rational(50728, &capture.as_shot_neutral),
        short(50778, &[21]),
    ];
    let mut dng_tags = dng_tags;
    if t.rendering {
        dng_tags.extend([
            srational(50722, &m.concat()),
            rational(50727, &[1.0, 1.0, 1.0]),
            srational(50730, &[0.0]),
            rational(50731, &[1.0]),
            rational(50732, &[1.0]),
            rational(50734, &[1.0]),
            short(50779, &[17]),
        ]);
    }
    let ifd0 = match &capture.preview {
        Some((pw, ph, preview)) => {
            let raw_ifd = write_ifd(&mut out, raw);
            let mut entries = vec![
                long(254, &[1]),
                long(256, &[*pw]),
                long(257, &[*ph]),
                short(258, &[8, 8, 8]),
                short(259, &[1]),
                short(262, &[2]),
                long(273, &[preview_offset]),
                short(277, &[3]),
                long(278, &[*ph]),
                long(279, &[preview.len() as u32]),
                short(284, &[1]),
                long(330, &[raw_ifd]),
            ];
            entries.extend(dng_tags);
            write_ifd(&mut out, entries)
        }
        None => {
            raw.extend(dng_tags);
            write_ifd(&mut out, raw)
        }
    };
    out[4..8].copy_from_slice(&ifd0.to_le_bytes());
    out
}

// ---------------------------------------------------------------------------------------------
// GIF

/// The smallest GIF: one white pixel. The app doesn't import GIFs.
pub const GIF_1X1: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";

// ---------------------------------------------------------------------------------------------
// JPEG from DCT coefficients

/// Zigzag position to natural (row-major) index.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57,
    50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
const DC_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1,
    0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56,
    0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85,
    0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa,
    0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6,
    0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9,
    0xfa,
];

/// Canonical Huffman codes for a table: (code, length) per symbol.
fn huffman_codes(bits: &[u8; 16], values: &[u8]) -> Vec<(u16, u8)> {
    let mut codes = vec![(0u16, 0u8); 256];
    let (mut code, mut k) = (0u16, 0);
    for (i, &n) in bits.iter().enumerate() {
        for _ in 0..n {
            codes[values[k] as usize] = (code, i as u8 + 1);
            code += 1;
            k += 1;
        }
        code <<= 1;
    }
    codes
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, code: u32, len: u32) {
        for i in (0..len).rev() {
            self.acc = (self.acc << 1) | ((code >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                if self.acc == 0xff {
                    self.out.push(0);
                }
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
        self.out
    }
}

fn magnitude(v: i32) -> (u32, u32) {
    let size = 32 - v.unsigned_abs().leading_zeros();
    let bits = if v < 0 { (v - 1) as u32 & ((1 << size) - 1) } else { v as u32 };
    (size, bits)
}

/// A baseline grayscale JPEG whose 8x8 blocks hold exactly `blocks` (quantized coefficients in
/// natural order, row by row of blocks) under a quantization table of all ones, with the standard
/// Huffman tables: a probe of the decoder's inverse DCT, free of any encoder's rounding.
pub fn jpeg_gray_coefficients(blocks_wide: u32, blocks_high: u32, blocks: &[[i16; 64]]) -> Vec<u8> {
    let (width, height) = (blocks_wide * 8, blocks_high * 8);
    let mut out = vec![0xff, 0xd8];
    let segment = |out: &mut Vec<u8>, marker: u8, body: &[u8]| {
        out.extend([0xff, marker]);
        out.extend(((body.len() + 2) as u16).to_be_bytes());
        out.extend(body);
    };
    segment(&mut out, 0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    let mut dqt = vec![0u8];
    dqt.extend([1u8; 64]);
    segment(&mut out, 0xdb, &dqt);
    let mut sof = vec![8];
    sof.extend((height as u16).to_be_bytes());
    sof.extend((width as u16).to_be_bytes());
    sof.extend([1, 1, 0x11, 0]);
    segment(&mut out, 0xc0, &sof);
    let mut dht = vec![0x00];
    dht.extend(DC_BITS);
    dht.extend(DC_VALUES);
    dht.push(0x10);
    dht.extend(AC_BITS);
    dht.extend(AC_VALUES);
    segment(&mut out, 0xc4, &dht);
    segment(&mut out, 0xda, &[1, 1, 0x00, 0, 63, 0]);
    let (dc_codes, ac_codes) = (huffman_codes(&DC_BITS, &DC_VALUES), huffman_codes(&AC_BITS, &AC_VALUES));
    let mut w = BitWriter { out: Vec::new(), acc: 0, n: 0 };
    let mut previous = 0i32;
    for block in blocks {
        let dc = block[0] as i32;
        let (size, bits) = magnitude(dc - previous);
        previous = dc;
        let (code, len) = dc_codes[size as usize];
        w.put(code as u32, len as u32);
        w.put(bits, size);
        let mut run = 0;
        for k in 1..64 {
            let v = block[ZIGZAG[k]] as i32;
            if v == 0 {
                run += 1;
                continue;
            }
            while run > 15 {
                let (code, len) = ac_codes[0xf0];
                w.put(code as u32, len as u32);
                run -= 16;
            }
            let (size, bits) = magnitude(v);
            let (code, len) = ac_codes[(run << 4) | size as usize];
            w.put(code as u32, len as u32);
            w.put(bits, size);
            run = 0;
        }
        if run > 0 {
            let (code, len) = ac_codes[0x00];
            w.put(code as u32, len as u32);
        }
    }
    out.extend(w.finish());
    out.extend([0xff, 0xd9]);
    out
}
