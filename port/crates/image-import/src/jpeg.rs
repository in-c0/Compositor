//! JPEG, as ImageIO reads it: YCbCr, grayscale or Adobe CMYK, with the EXIF orientation and an
//! embedded ICC profile.

use crate::color::{Curve, Space};
use crate::develop::{Samples, Source};
use crate::{ImportError, Result, exif};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

pub(crate) fn decode(data: &[u8]) -> Result<Source> {
    let mut probe = JpegDecoder::new(std::io::Cursor::new(data));
    probe.decode_headers().map_err(|_| ImportError::Unreadable)?;
    let input = probe.input_colorspace().ok_or(ImportError::Unreadable)?;
    let (out, colors) = match input {
        ColorSpace::Luma => (ColorSpace::Luma, 1),
        ColorSpace::CMYK | ColorSpace::YCCK => {
            return Err(ImportError::NotPorted("CMYK JPEG (ColorSync converts it with Apple's Generic CMYK profile)".into()));
        }
        _ => (ColorSpace::RGB, 3),
    };
    let mut decoder = JpegDecoder::new_with_options(std::io::Cursor::new(data), DecoderOptions::default().jpeg_set_out_colorspace(out));
    let pixels = decoder.decode().map_err(|_| ImportError::Unreadable)?;
    let info = decoder.info().ok_or(ImportError::Unreadable)?;
    let space = match decoder.icc_profile() {
        Some(icc) => Space::from_icc(&icc, colors)?,
        None if colors == 1 => Space::Gray(Curve::Srgb),
        None => Space::Srgb,
    };
    let orientation = decoder.exif().and_then(|e| exif::orientation(e)).unwrap_or(1);
    Ok(Source {
        width: info.width as u32,
        height: info.height as u32,
        colors,
        alpha: false,
        premultiplied: false,
        samples: Samples::U8(pixels),
        space,
        orientation,
        approximation: Some("Apple's JPEG decoder: its inverse DCT and chroma upsampling aren't reproduced yet".into()),
    })
}
