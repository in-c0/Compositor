//! File > Export PNG and Export JPEG, written the way the Mac's ImageIO writes them.

use anyhow::Result;
use image::RgbaImage;

/// A PNG as `ImageExporter.pngData` writes it: RGBA8, then the same ancillary chunks ImageIO
/// adds (sRGB, an EXIF block with the resolution, color space and size, and pHYs), then pixels.
pub fn png(image: &RgbaImage, resolution: f64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width(), image.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        // Rendering intent 0, perceptual.
        writer.write_chunk(png::chunk::ChunkType(*b"sRGB"), &[0])?;
        writer.write_chunk(png::chunk::ChunkType(*b"eXIf"), &exif(image.width(), image.height(), resolution))?;
        let per_meter = (resolution / 0.0254).round() as u32;
        let mut phys = Vec::with_capacity(9);
        phys.extend_from_slice(&per_meter.to_be_bytes());
        phys.extend_from_slice(&per_meter.to_be_bytes());
        phys.push(1);
        writer.write_chunk(png::chunk::ChunkType(*b"pHYs"), &phys)?;
        writer.write_image_data(image.as_raw())?;
    }
    Ok(out)
}

/// The TIFF-structured EXIF block ImageIO writes: XResolution, YResolution, ResolutionUnit
/// (inches) and an EXIF sub-IFD with ColorSpace (sRGB) and the pixel dimensions. Big-endian.
fn exif(width: u32, height: u32, resolution: f64) -> Vec<u8> {
    let (num, den) = rational(resolution);
    let mut b = Vec::with_capacity(120);
    b.extend_from_slice(b"MM\0*");
    b.extend_from_slice(&8u32.to_be_bytes());
    let entry = |b: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32| {
        b.extend_from_slice(&tag.to_be_bytes());
        b.extend_from_slice(&kind.to_be_bytes());
        b.extend_from_slice(&count.to_be_bytes());
        if kind == 3 {
            b.extend_from_slice(&(value as u16).to_be_bytes());
            b.extend_from_slice(&[0, 0]);
        } else {
            b.extend_from_slice(&value.to_be_bytes());
        }
    };
    // IFD0 at 8: four entries, then the two rationals at 62 and 70, then the EXIF IFD at 78.
    b.extend_from_slice(&4u16.to_be_bytes());
    entry(&mut b, 0x011A, 5, 1, 62);
    entry(&mut b, 0x011B, 5, 1, 70);
    entry(&mut b, 0x0128, 3, 1, 2);
    entry(&mut b, 0x8769, 4, 1, 78);
    b.extend_from_slice(&0u32.to_be_bytes());
    for _ in 0..2 {
        b.extend_from_slice(&num.to_be_bytes());
        b.extend_from_slice(&den.to_be_bytes());
    }
    b.extend_from_slice(&3u16.to_be_bytes());
    entry(&mut b, 0xA001, 3, 1, 1);
    entry(&mut b, 0xA002, 4, 1, width);
    entry(&mut b, 0xA003, 4, 1, height);
    b.extend_from_slice(&0u32.to_be_bytes());
    b
}

/// A resolution as the rational ImageIO stores. Whole values are n/1; the rest are fitted once
/// the references show ImageIO's choice (see the export cases).
fn rational(value: f64) -> (u32, u32) {
    if value.fract() == 0.0 {
        return (value as u32, 1);
    }
    let den = 1000u32;
    ((value * den as f64).round() as u32, den)
}

#[cfg(test)]
mod tests {
    #[test]
    fn exif_matches_the_macs_bytes_at_72_ppi() {
        let expected = "4d4d002a000000080004011a0005000000010000003e011b0005000000010000004601280003000000010002000087690004000000010000004e00000000000000480000000100000048000000010003a00100030000000100010000a00200040000000100000040a0030004000000010000004000000000";
        let got: String = super::exif(64, 64, 72.0).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, expected);
    }
}
