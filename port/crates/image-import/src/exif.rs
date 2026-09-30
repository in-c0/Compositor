//! The one EXIF field the import reads: Orientation (0x0112) in IFD 0 of a TIFF-structured block.

/// Orientation from a TIFF-structured EXIF block (starting at the byte-order mark), when it is
/// there and 1-8.
pub(crate) fn orientation(tiff: &[u8]) -> Option<u16> {
    let big = match tiff.get(0..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = tiff.get(at..at + 2)?;
        Some(if big { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = tiff.get(at..at + 4)?;
        Some(if big { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    for i in 0..count {
        let entry = ifd + 2 + i * 12;
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 {
            let value = u16_at(entry + 8)?;
            return (1..=8).contains(&value).then_some(value);
        }
    }
    None
}
