//! TIFF, as ImageIO reads it: 8- and 16-bit gray and RGB, with or without alpha, and the
//! Orientation tag.

use crate::color::{Curve, Space};
use crate::develop::{Samples, Source};
use crate::{ImportError, Result};
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

pub(crate) fn decode(data: &[u8]) -> Result<Source> {
    let mut decoder = Decoder::new(std::io::Cursor::new(data)).map_err(|_| ImportError::Unreadable)?;
    let (width, height) = decoder.dimensions().map_err(|_| ImportError::Unreadable)?;
    crate::check_size(width as u64, height as u64)?;
    let color = decoder.colortype().map_err(|_| ImportError::Unreadable)?;
    let extra: Vec<u16> = decoder.get_tag_u16_vec(Tag::ExtraSamples).unwrap_or_default();
    let orientation = decoder.get_tag_u32(Tag::Orientation).ok().filter(|o| (1..=8).contains(o)).unwrap_or(1) as u16;
    let icc = decoder.get_tag_u8_vec(Tag::Unknown(34675)).ok();
    let (colors, alpha) = match color {
        tiff::ColorType::Gray(_) => (1, false),
        tiff::ColorType::GrayA(_) => (1, true),
        tiff::ColorType::RGB(_) => (3, false),
        tiff::ColorType::RGBA(_) => (3, true),
        _ => return Err(ImportError::NotPorted(format!("TIFF color type {color:?}"))),
    };
    let premultiplied = alpha && extra.first() == Some(&1);
    let samples = match decoder.read_image().map_err(|_| ImportError::Unreadable)? {
        DecodingResult::U8(v) => Samples::U8(v),
        DecodingResult::U16(v) => Samples::U16(v),
        _ => return Err(ImportError::NotPorted("TIFF sample format other than 8 or 16-bit unsigned".into())),
    };
    let space = match icc {
        Some(icc) => Space::from_icc(&icc, colors)?,
        None if colors == 1 => Space::Gray(Curve::Srgb),
        None => Space::Srgb,
    };
    Ok(Source { width, height, colors, alpha, premultiplied, samples, space, orientation, approximation: None })
}
