//! HEIC: the HEVC picture decoded by heic-rs (a conforming decoder, so the YCbCr planes are the
//! ones Apple's decoder makes), then converted to RGB here.

use crate::develop::Source;
use crate::{ImportError, Result};

/// An ISO base media file whose brand is an HEIF image brand.
pub(crate) fn matches(data: &[u8]) -> bool {
    data.len() >= 12 && &data[4..8] == b"ftyp" && matches!(&data[8..12], b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" | b"hevc" | b"hevx")
}

pub(crate) fn decode(_data: &[u8]) -> Result<Source> {
    Err(ImportError::NotPorted("HEIC".into()))
}
