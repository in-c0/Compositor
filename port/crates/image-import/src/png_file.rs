//! PNG, as ImageIO reads it: palette and tRNS expanded, 16-bit kept, and the color space from
//! iCCP, then sRGB, then gAMA (with cHRM when present); untagged means sRGB.

use crate::color::{Curve, Space};
use crate::develop::{Samples, Source};
use crate::{ImportError, Result, exif};

pub(crate) fn decode(data: &[u8]) -> Result<Source> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|_| ImportError::Unreadable)?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or(ImportError::TooLarge)?];
    let frame = reader.next_frame(&mut buf).map_err(|_| ImportError::Unreadable)?;
    buf.truncate(frame.buffer_size());
    let info = reader.info();
    let (colors, alpha) = match frame.color_type {
        png::ColorType::Grayscale => (1, false),
        png::ColorType::GrayscaleAlpha => (1, true),
        png::ColorType::Rgb => (3, false),
        png::ColorType::Rgba => (3, true),
        png::ColorType::Indexed => return Err(ImportError::Unreadable),
    };
    let samples = match frame.bit_depth {
        png::BitDepth::Sixteen => Samples::U16(buf.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect()),
        png::BitDepth::Eight => Samples::U8(buf),
        _ => return Err(ImportError::Unreadable),
    };
    let space = if let Some(icc) = &info.icc_profile {
        Space::from_icc(icc, colors)?
    } else if info.srgb.is_some() {
        if colors == 1 { Space::Gray(Curve::Srgb) } else { Space::Srgb }
    } else if let Some(gamma) = info.gama_chunk {
        if colors == 1 {
            Space::Gray(Curve::Gamma(1.0 / gamma.into_value() as f64))
        } else {
            let chromaticities = info.chrm_chunk.map(|c| {
                let xy = |p: (png::ScaledFloat, png::ScaledFloat)| [p.0.into_value() as f64, p.1.into_value() as f64];
                [xy(c.white), xy(c.red), xy(c.green), xy(c.blue)]
            });
            Space::from_gamma(gamma.into_value() as f64, chromaticities)
        }
    } else if colors == 1 {
        Space::Gray(Curve::Srgb)
    } else {
        Space::Srgb
    };
    let orientation = info.exif_metadata.as_deref().and_then(exif::orientation).unwrap_or(1);
    Ok(Source {
        width: frame.width,
        height: frame.height,
        colors,
        alpha,
        premultiplied: false,
        samples,
        space,
        orientation,
        approximation: None,
    })
}
