//! Image import cases: a JPEG, PNG, HEIC, TIFF, SVG or camera RAW file opened into an empty
//! session, as File > Open and dropping a file do (`EditorSession.importImages`). The harness saves
//! the project the Mac made, so each case checks the layer's pixels, the canvas size and the
//! resolution as well as the flattened image.
//!
//! Every input is written here, except the HEIC files: HEVC has no encoder in this workspace, so
//! `fixtures/make_heic.py` made them once (libheif 1.23.4 with x265 4.3, through pillow-heif 1.8.0,
//! from pictures that script draws) and they are copied in as committed. The DNG files are
//! synthetic: a made-up camera whose raw samples come from a drawn scene. No input has third-party
//! content.

use super::builder::CaseWriter;
use super::images::{self, hash};
use super::import_files::{self as files, DngTags, JpegOptions, PngOptions, RawCapture, TiffPixels, Trc};
use anyhow::Result;
use jpeg_encoder::{ColorType as JpegColor, SamplingFactor};
use png::{BitDepth, ColorType as PngColor};
use serde_json::{Value, json};

const FEATURE: &str = "import";

pub fn import(w: &mut CaseWriter) -> Result<()> {
    png_cases(w)?;
    jpeg_cases(w)?;
    tiff_cases(w)?;
    heic_cases(w)?;
    svg_cases(w)?;
    raw_cases(w)?;
    put(w, "gif-refused", "A GIF, which the app doesn't import: both refuse it with the same message", "input.gif", files::GIF_1X1)?;
    Ok(())
}

fn put(w: &mut CaseWriter, case: &str, label: &str, name: &str, bytes: &[u8]) -> Result<()> {
    w.write_file(FEATURE, case, label, name, bytes, vec![])
}

/// A RAW case that opens `other`'s input.dng with its own develop settings.
fn shared(w: &mut CaseWriter, case: &str, label: &str, other: &str, raw: Value) -> Result<()> {
    w.write_shared(FEATURE, case, label, other, "input.dng", raw)
}

fn put_raw(w: &mut CaseWriter, case: &str, label: &str, bytes: &[u8], raw: Value) -> Result<()> {
    w.write_file_with(FEATURE, case, label, "input.dng", bytes, raw)
}

/// 16 x 16 opaque pixels in which each channel takes every value 0-255 once, in a different order.
fn all_values_rgb() -> Vec<u8> {
    (0..256u32).flat_map(|i| [i as u8, (i * 167 + 13) as u8, (i * 73 + 101) as u8]).collect()
}

/// 256 x 256: red runs 0-255 left to right and alpha 0-255 top to bottom, so every (color, alpha)
/// pair is there once; green and blue vary too.
fn all_values_rgba() -> Vec<u8> {
    let mut v = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256u32 {
        for x in 0..256u32 {
            v.extend([x as u8, (255 - x) as u8, (x * 97 + y * 13) as u8, y as u8]);
        }
    }
    v
}

fn rgb_of(img: &image::RgbaImage) -> Vec<u8> {
    img.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect()
}

fn photo_rgb(w: u32, h: u32) -> Vec<u8> {
    rgb_of(&images::photo(w, h))
}

fn be16(samples: &[u16]) -> Vec<u8> {
    samples.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn png_cases(w: &mut CaseWriter) -> Result<()> {
    let rgb = all_values_rgb();
    let png = |width, height, color, depth, data: &[u8], options| files::png(width, height, color, depth, data, options);
    put(w, "png-rgb8", "8-bit RGB PNG with no color information, every value in every channel", "input.png",
        &png(16, 16, PngColor::Rgb, BitDepth::Eight, &rgb, PngOptions::default())?)?;
    put(w, "png-rgba8", "8-bit RGBA PNG holding every (color, alpha) pair: premultiplication and the saved layer", "input.png",
        &png(256, 256, PngColor::Rgba, BitDepth::Eight, &all_values_rgba(), PngOptions::default())?)?;
    put(w, "png-srgb", "8-bit RGB PNG with an sRGB chunk", "input.png",
        &png(48, 32, PngColor::Rgb, BitDepth::Eight, &photo_rgb(48, 32), PngOptions { srgb: true, ..Default::default() })?)?;
    let gray: Vec<u8> = (0..256u32).map(|i| i as u8).collect();
    put(w, "png-gray8", "8-bit grayscale PNG, every value", "input.png", &png(16, 16, PngColor::Grayscale, BitDepth::Eight, &gray, PngOptions::default())?)?;
    let gray_alpha: Vec<u8> = (0..16u32).flat_map(|y| (0..64u32).flat_map(move |x| [(x * 4 + y / 4) as u8, (y * 17) as u8])).collect();
    put(w, "png-gray-alpha8", "8-bit grayscale PNG with alpha", "input.png",
        &png(64, 16, PngColor::GrayscaleAlpha, BitDepth::Eight, &gray_alpha, PngOptions::default())?)?;
    let palette: Vec<u8> = (0..256u32).flat_map(|i| [(i * 29) as u8, (i * 71 + 5) as u8, (255 - i) as u8]).collect();
    let trns: Vec<u8> = (0..200u32).map(|i| if i < 8 { 0 } else { (i * 37) as u8 | 1 }).collect();
    let indices: Vec<u8> = (0..32 * 16u32).map(|i| (hash(i) >> 24) as u8).collect();
    put(w, "png-palette", "Indexed PNG with a partly transparent palette (tRNS)", "input.png",
        &png(32, 16, PngColor::Indexed, BitDepth::Eight, &indices, PngOptions { palette: Some((palette, trns)), ..Default::default() })?)?;
    let deep: Vec<u16> = (0..64 * 64u32).flat_map(|i| [(i * 16) as u16, (i * 16 + 7) as u16, (hash(i ^ 0x16) >> 16) as u16]).collect();
    put(w, "png-rgb16", "16-bit RGB PNG: reduced to 8 bits on import", "input.png",
        &png(64, 64, PngColor::Rgb, BitDepth::Sixteen, &be16(&deep), PngOptions::default())?)?;
    let deep_alpha: Vec<u16> = (0..64 * 64u32)
        .flat_map(|i| {
            let h = hash(i ^ 0x1616);
            [(i * 16) as u16, (h >> 16) as u16, h as u16, (hash(i ^ 0xa1) >> 16) as u16]
        })
        .collect();
    put(w, "png-rgba16", "16-bit RGBA PNG", "input.png", &png(64, 64, PngColor::Rgba, BitDepth::Sixteen, &be16(&deep_alpha), PngOptions::default())?)?;
    put(w, "png-gamma18", "8-bit RGB PNG with only a gAMA chunk, gamma 1/1.8", "input.png",
        &png(16, 16, PngColor::Rgb, BitDepth::Eight, &rgb, PngOptions { gamma: Some(55556), ..Default::default() })?)?;
    let adobe_chrm = [[0.3127, 0.3290], files::ADOBE_RGB[0], files::ADOBE_RGB[1], files::ADOBE_RGB[2]];
    put(w, "png-gamma-chrm", "8-bit RGB PNG with gAMA 1/2.2 and cHRM with Adobe RGB's primaries", "input.png",
        &png(16, 16, PngColor::Rgb, BitDepth::Eight, &rgb, PngOptions { gamma: Some(45455), chromaticities: Some(adobe_chrm), ..Default::default() })?)?;
    let p3 = files::icc_profile("Compositor test Display P3", files::DISPLAY_P3, Trc::Srgb, true);
    let adobe = files::icc_profile("Compositor test Adobe RGB (1998) compatible", files::ADOBE_RGB, Trc::Gamma(563), false);
    put(w, "png-p3", "8-bit RGB PNG with an embedded Display P3 profile (v4, parametric curve), every value", "input.png",
        &png(16, 16, PngColor::Rgb, BitDepth::Eight, &rgb, PngOptions { icc: Some(p3.clone()), ..Default::default() })?)?;
    put(w, "png-adobe-rgb", "8-bit RGB PNG with an embedded Adobe RGB (1998)-compatible profile (v2, gamma 563/256), every value", "input.png",
        &png(16, 16, PngColor::Rgb, BitDepth::Eight, &rgb, PngOptions { icc: Some(adobe), ..Default::default() })?)?;
    let noise = images::noise(64, 64, 7, images::Alpha::Varied);
    put(w, "png-p3-rgba", "8-bit RGBA PNG with a Display P3 profile: color conversion and alpha together", "input.png",
        &png(64, 64, PngColor::Rgba, BitDepth::Eight, noise.as_raw(), PngOptions { icc: Some(p3.clone()), ..Default::default() })?)?;
    put(w, "png-p3-16", "16-bit RGB PNG with a Display P3 profile", "input.png",
        &png(64, 64, PngColor::Rgb, BitDepth::Sixteen, &be16(&deep), PngOptions { icc: Some(p3), ..Default::default() })?)?;
    put(w, "png-exif-orientation-6", "8-bit RGB PNG with an eXIf chunk giving orientation 6", "input.png",
        &png(48, 32, PngColor::Rgb, BitDepth::Eight, &photo_rgb(48, 32), PngOptions { exif: Some(files::exif_orientation(6)), ..Default::default() })?)?;
    Ok(())
}

fn jpeg_cases(w: &mut CaseWriter) -> Result<()> {
    let photo = photo_rgb(48, 32);
    let jpeg = |data: &[u8], color, options| files::jpeg(48, 32, color, data, options);
    put(w, "jpeg-444", "Baseline JPEG, quality 90, no chroma subsampling", "input.jpg", &jpeg(&photo, JpegColor::Rgb, JpegOptions::default())?)?;
    put(w, "jpeg-420", "Baseline JPEG, quality 90, 4:2:0", "input.jpg",
        &jpeg(&photo, JpegColor::Rgb, JpegOptions { sampling: SamplingFactor::F_2_2, ..Default::default() })?)?;
    put(w, "jpeg-422", "Baseline JPEG, quality 90, 4:2:2", "input.jpg",
        &jpeg(&photo, JpegColor::Rgb, JpegOptions { sampling: SamplingFactor::F_2_1, ..Default::default() })?)?;
    put(w, "jpeg-progressive", "Progressive JPEG, quality 90, 4:2:0", "input.jpg",
        &jpeg(&photo, JpegColor::Rgb, JpegOptions { sampling: SamplingFactor::F_2_2, progressive: true, ..Default::default() })?)?;
    let noise = rgb_of(&images::noise(48, 32, 11, images::Alpha::Opaque));
    put(w, "jpeg-noise-444", "Baseline JPEG of per-pixel noise, quality 75: the most IDCT rounding", "input.jpg",
        &jpeg(&noise, JpegColor::Rgb, JpegOptions { quality: 75, ..Default::default() })?)?;
    let odd = photo_rgb(47, 31);
    put(w, "jpeg-420-odd", "Baseline JPEG, 4:2:0, odd width and height", "input.jpg",
        &files::jpeg(47, 31, JpegColor::Rgb, &odd, JpegOptions { sampling: SamplingFactor::F_2_2, ..Default::default() })?)?;
    let gray: Vec<u8> = photo.chunks(3).map(|p| ((p[0] as u32 * 3 + p[1] as u32 * 4 + p[2] as u32) / 8) as u8).collect();
    put(w, "jpeg-gray", "Grayscale JPEG", "input.jpg", &jpeg(&gray, JpegColor::Luma, JpegOptions::default())?)?;
    for orientation in 1..=8u16 {
        put(w, &format!("jpeg-orientation-{orientation}"), &format!("JPEG with EXIF orientation {orientation}: turned upright on import"), "input.jpg",
            &jpeg(&photo, JpegColor::Rgb, JpegOptions { exif: Some(files::exif_orientation(orientation)), ..Default::default() })?)?;
    }
    let p3 = files::icc_profile("Compositor test Display P3", files::DISPLAY_P3, Trc::Srgb, true);
    put(w, "jpeg-p3", "JPEG with an embedded Display P3 profile", "input.jpg",
        &jpeg(&photo, JpegColor::Rgb, JpegOptions { icc: Some(p3), ..Default::default() })?)?;
    jpeg_probes(w)?;
    let cmyk: Vec<u8> = photo
        .chunks(3)
        .flat_map(|p| {
            let k = 255 - p[0].max(p[1]).max(p[2]);
            let f = |c: u8| if k == 255 { 0 } else { ((255 - c - k) as u32 * 255 / (255 - k) as u32) as u8 };
            [f(p[0]), f(p[1]), f(p[2]), k]
        })
        .collect();
    put(w, "jpeg-cmyk", "Adobe CMYK JPEG with no profile: converted from the system's generic CMYK", "input.jpg",
        &jpeg(&cmyk, JpegColor::Cmyk, JpegOptions::default())?)?;
    Ok(())
}

/// Probes of Apple's JPEG decoder. The coefficient files pin down its inverse DCT (one basis
/// function per block at four amplitudes, then random sparse blocks); the quality-100 files of
/// per-pixel noise show its chroma upsampling and YCbCr conversion.
fn jpeg_probes(w: &mut CaseWriter) -> Result<()> {
    let mut basis = Vec::new();
    for amplitude in [37i16, -61, 113, -150] {
        for position in 0..64 {
            let mut block = [0i16; 64];
            block[position] = amplitude;
            basis.push(block);
        }
    }
    put(w, "jpeg-idct-basis", "Grayscale JPEG, all-ones quantization: each block holds one coefficient (every position, four amplitudes)", "input.jpg",
        &files::jpeg_gray_coefficients(16, 16, &basis))?;
    let random: Vec<[i16; 64]> = (0..256u32)
        .map(|b| {
            let mut block = [0i16; 64];
            block[0] = ((hash(b * 31 + 7) % 801) as i32 - 400) as i16;
            for k in 0..8u32 {
                let h = hash(b * 977 + k * 13 + 1);
                block[1 + (h % 63) as usize] = (((h >> 8) % 241) as i32 - 120) as i16;
            }
            block
        })
        .collect();
    put(w, "jpeg-idct-random", "Grayscale JPEG, all-ones quantization: random sparse blocks", "input.jpg", &files::jpeg_gray_coefficients(16, 16, &random))?;
    for (case, sampling, (fx, fy)) in [("jpeg-ycc-444", SamplingFactor::F_1_1, (1, 1)), ("jpeg-ycc-420", SamplingFactor::F_2_2, (2, 2)), ("jpeg-ycc-422", SamplingFactor::F_2_1, (2, 1))] {
        let (width, height) = (64u32, 64u32);
        let ycc: Vec<u8> = (0..height)
            .flat_map(|y| {
                (0..width).flat_map(move |x| {
                    let cell = (y / fy) * width + x / fx;
                    [(hash(y * width + x) >> 24) as u8, 40 + (hash(cell ^ 0xcb) >> 24) as u8 % 176, 40 + (hash(cell ^ 0xc7) >> 24) as u8 % 176]
                })
            })
            .collect();
        put(w, case, "Quality-100 JPEG of YCbCr noise, chroma constant over each subsampling cell", "input.jpg",
            &files::jpeg(width, height, JpegColor::Ycbcr, &ycc, JpegOptions { quality: 100, sampling, ..Default::default() })?)?;
    }
    Ok(())
}

fn tiff_cases(w: &mut CaseWriter) -> Result<()> {
    use tiff::encoder::Compression;
    let photo = photo_rgb(48, 32);
    let tiff = |pixels, compression, orientation| files::tiff(48, 32, pixels, compression, orientation);
    put(w, "tiff-rgb8", "Uncompressed 8-bit RGB TIFF", "input.tif", &tiff(TiffPixels::Rgb8(&photo), Compression::Uncompressed, None)?)?;
    put(w, "tiff-rgb8-lzw", "LZW-compressed 8-bit RGB TIFF", "input.tiff", &tiff(TiffPixels::Rgb8(&photo), Compression::Lzw, None)?)?;
    let noise = images::noise(48, 32, 5, images::Alpha::Varied);
    put(w, "tiff-rgba8", "8-bit RGBA TIFF with unassociated alpha", "input.tif", &tiff(TiffPixels::Rgba8(noise.as_raw()), Compression::Lzw, None)?)?;
    let deep: Vec<u16> = (0..48 * 32u32).flat_map(|i| [(i * 42) as u16, (i * 42 + 21) as u16, (hash(i ^ 0x77) >> 16) as u16]).collect();
    put(w, "tiff-rgb16-lzw", "LZW-compressed 16-bit RGB TIFF", "input.tif", &tiff(TiffPixels::Rgb16(&deep), Compression::Lzw, None)?)?;
    let gray: Vec<u8> = (0..48 * 32u32).map(|i| (i % 256) as u8).collect();
    put(w, "tiff-gray8", "8-bit grayscale TIFF", "input.tif", &tiff(TiffPixels::Gray8(&gray), Compression::Uncompressed, None)?)?;
    put(w, "tiff-orientation-8", "8-bit RGB TIFF with Orientation 8", "input.tif", &tiff(TiffPixels::Rgb8(&photo), Compression::Uncompressed, Some(8))?)?;
    Ok(())
}

fn heic_cases(w: &mut CaseWriter) -> Result<()> {
    put(w, "heic-420", "HEIC, 8-bit 4:2:0 (x265), sRGB nclx", "input.heic", include_bytes!("fixtures/heic-420.heic"))?;
    put(w, "heic-444", "HEIC, 8-bit 4:4:4 (x265), sRGB nclx", "input.heic", include_bytes!("fixtures/heic-444.heic"))?;
    put(w, "heic-rgba", "HEIC with an alpha plane", "input.heic", include_bytes!("fixtures/heic-rgba.heic"))?;
    put(w, "heic-odd-420", "HEIC, 4:2:0, odd width and height", "input.heic", include_bytes!("fixtures/heic-odd-420.heic"))?;
    put(w, "heic-noise-420", "HEIC of per-pixel noise, 4:2:0, quality 100: chroma upsampling", "input.heic", include_bytes!("fixtures/heic-noise-420.heic"))?;
    put(w, "heic-noise-444", "HEIC of per-pixel noise, 4:4:4, quality 100: YCbCr conversion", "input.heic", include_bytes!("fixtures/heic-noise-444.heic"))?;
    put(w, "heic-noise-rgba", "HEIC of per-pixel noise with a noise alpha plane, 4:2:0, quality 100", "input.heic", include_bytes!("fixtures/heic-noise-rgba.heic"))?;
    Ok(())
}

fn svg_cases(w: &mut CaseWriter) -> Result<()> {
    const HEAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg""#;
    let rects = format!(
        r##"{HEAD} width="48" height="32">
  <rect x="0" y="0" width="48" height="32" fill="#f4f1e8"/>
  <rect x="4" y="4" width="16" height="12" fill="#d0342c"/>
  <rect x="24" y="4" width="20" height="12" fill="rgb(40,120,220)" fill-opacity="0.5"/>
  <rect x="4" y="20" width="40" height="8" fill="#1f8a4c"/>
</svg>
"##
    );
    put(w, "svg-rects", "SVG of pixel-aligned rectangles, one half transparent, at its declared size", "input.svg", rects.as_bytes())?;
    let transparent = format!(
        r##"{HEAD} width="40" height="24">
  <rect x="2" y="2" width="16" height="20" fill="#e0a020"/>
  <rect x="22" y="2" width="16" height="20" fill="#3050c0" opacity="0.25"/>
</svg>
"##
    );
    put(w, "svg-transparent", "SVG with no background: transparent where nothing is drawn", "input.svg", transparent.as_bytes())?;
    let shapes = format!(
        r##"{HEAD} width="64" height="48">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#ff6040"/>
      <stop offset="1" stop-color="#4060ff"/>
    </linearGradient>
  </defs>
  <rect x="0" y="0" width="64" height="48" fill="url(#g)"/>
  <circle cx="18" cy="18" r="11" fill="#ffffff" stroke="#202020" stroke-width="3"/>
  <ellipse cx="46" cy="30" rx="14" ry="8" fill="#20a060" fill-opacity="0.7"/>
  <path d="M4 44 C 16 28, 30 52, 44 40 S 60 30, 62 44" fill="none" stroke="#ffe040" stroke-width="2.5"/>
  <rect x="36" y="4" width="14" height="10" fill="#8030a0" transform="rotate(20 43 9)"/>
</svg>
"##
    );
    put(w, "svg-shapes", "SVG with a gradient, circle, ellipse, curve and rotated rectangle: antialiased edges", "input.svg", shapes.as_bytes())?;
    let viewbox = format!(
        r##"{HEAD} width="48" height="32" viewBox="0 0 24 16">
  <rect x="0" y="0" width="24" height="16" fill="#202840"/>
  <rect x="2" y="2" width="9" height="5" fill="#f0c030"/>
  <rect x="13" y="9" width="9" height="5" fill="#30c0f0"/>
</svg>
"##
    );
    put(w, "svg-viewbox", "SVG whose viewBox is scaled 2x to its width and height", "input.svg", viewbox.as_bytes())?;
    Ok(())
}

/// A made-up camera: XYZ (D65) to camera RGB.
const CAMERA_MATRIX: [[f64; 3]; 3] = [[0.70, -0.10, -0.05], [-0.40, 1.25, 0.15], [-0.05, 0.20, 0.60]];

/// The scene the synthetic camera photographs, in linear sRGB: a grid of colored patches, a gray
/// ramp and a color ramp, in blocks big enough that demosaicing leaves their centers flat.
fn scene(width: u32, height: u32, x: u32, y: u32) -> [f64; 3] {
    let lin = |c: u8| {
        let v = c as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    if y < height / 2 {
        let (col, row) = (x / 8, y / 8);
        let (r, g, b) = images::hsv(col as f64 * 45.0, 0.3 + 0.35 * row as f64, 0.95 - 0.25 * row as f64);
        return [lin(r), lin(g), lin(b)];
    }
    let t = x as f64 / (width - 1) as f64;
    if y < height * 5 / 6 {
        let v = t * 0.9;
        [v, v, v]
    } else {
        [0.8 * (1.0 - t) + 0.05, 0.1, 0.05 + 0.7 * t]
    }
}

fn capture(width: u32, height: u32, cfa: Option<[u8; 4]>, with_preview: bool, tags: DngTags) -> RawCapture {
    let srgb_to_xyz = files::rgb_to_xyz([[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]], [0.3127 / 0.3290, 1.0, (1.0 - 0.3127 - 0.3290) / 0.3290]);
    let to_camera = files::mat_mul(&CAMERA_MATRIX, &srgb_to_xyz);
    let white = files::mul(&to_camera, [1.0, 1.0, 1.0]);
    let peak = white.iter().cloned().fold(0.0, f64::max);
    let neutral = white.map(|c| c / peak);
    let (black, white_level) = (64u16, 4095u16);
    // The scene's white lands at 80% of the sensor's range in the camera's strongest channel.
    let sample = |v: f64| (black as f64 + (v / peak * 0.8).clamp(0.0, 1.0) * (white_level - black) as f64).round() as u16;
    let mut samples = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let camera = files::mul(&to_camera, scene(width, height, x, y));
            match cfa {
                Some(pattern) => samples.push(sample(camera[pattern[((y % 2) * 2 + x % 2) as usize] as usize])),
                None => samples.extend(camera.map(sample)),
            }
        }
    }
    let (pw, ph) = (width / 4, height / 4);
    let preview: Vec<u8> = (0..ph)
        .flat_map(|y| {
            (0..pw).flat_map(move |x| {
                scene(width, height, x * 4 + 2, y * 4 + 2).map(|v| (v.powf(1.0 / 2.2) * 255.0).round().clamp(0.0, 255.0) as u8)
            })
        })
        .collect();
    RawCapture {
        width,
        height,
        cfa,
        samples,
        black,
        white: white_level,
        color_matrix: CAMERA_MATRIX,
        as_shot_neutral: neutral,
        preview: with_preview.then_some((pw, ph, preview)),
        tags,
    }
}

fn raw_cases(w: &mut CaseWriter) -> Result<()> {
    const BAYER: Option<[u8; 4]> = Some([0, 1, 1, 2]);
    let bare = DngTags::default();
    let all = DngTags { cfa_layout: true, crop: true, rendering: true, full_levels: true };
    // A 64 x 48 Bayer DNG with only the required tags and a preview in IFD 0: the Mac can't
    // develop the raw image and imports the 16 x 12 preview instead.
    let small = files::dng(&capture(64, 48, BAYER, true, bare));
    put_raw(w, "dng-small-preview", "64 x 48 Bayer DNG, required tags only, 16 x 12 preview in IFD 0: the Mac imports the preview", &small, Value::Null)?;
    // The same without a preview: the Mac refuses it.
    let bayer = files::dng(&capture(192, 128, BAYER, false, bare));
    put_raw(w, "dng-bayer", "192 x 128 Bayer (RGGB) DNG, 12-bit, required tags only: the Mac can't read it", &bayer, Value::Null)?;
    // Which optional tags make a Bayer DNG developable.
    for (case, label, tags) in [
        ("dng-bayer-layout", "Bayer DNG with CFAPlaneColor and CFALayout", DngTags { cfa_layout: true, ..bare }),
        ("dng-bayer-crop", "Bayer DNG with CFAPlaneColor, CFALayout, the default crop and ActiveArea", DngTags { cfa_layout: true, crop: true, ..bare }),
        ("dng-bayer-levels", "Bayer DNG with CFAPlaneColor, CFALayout and a 2 x 2 black level", DngTags { cfa_layout: true, full_levels: true, ..bare }),
        ("dng-bayer-full", "Bayer DNG with every optional tag the writer knows, as shot", all),
    ] {
        put_raw(w, case, label, &files::dng(&capture(192, 128, BAYER, false, tags)), Value::Null)?;
    }
    shared(w, "dng-bayer-boost-0", "The dng-bayer-full file with Boost 0: Apple's tone curve off", "dng-bayer-full", json!({ "boost": 0 }))?;
    shared(w, "dng-bayer-settings", "The dng-bayer-full file with exposure +0.7, 4200 K, tint +12, boost 0.5", "dng-bayer-full",
        json!({ "exposure": 0.7, "temperature": 4200, "tint": 12, "boost": 0.5 }))?;
    let linear = files::dng(&capture(192, 128, None, false, bare));
    put_raw(w, "dng-linear", "192 x 128 linear (demosaiced) DNG, one black and white level for three samples, as shot", &linear, Value::Null)?;
    shared(w, "dng-linear-boost-0", "The dng-linear file with Boost 0", "dng-linear", json!({ "boost": 0 }))?;
    let levels = files::dng(&capture(96, 64, None, false, DngTags { full_levels: true, ..bare }));
    put_raw(w, "dng-linear-levels", "96 x 64 linear DNG with a black and white level per sample, as shot", &levels, Value::Null)?;
    shared(w, "dng-linear-levels-boost-0", "The dng-linear-levels file with Boost 0", "dng-linear-levels", json!({ "boost": 0 }))?;
    shared(w, "dng-linear-levels-settings", "The dng-linear-levels file with exposure +0.7, 4200 K, tint +12, boost 0.5", "dng-linear-levels",
        json!({ "exposure": 0.7, "temperature": 4200, "tint": 12, "boost": 0.5 }))?;
    Ok(())
}
