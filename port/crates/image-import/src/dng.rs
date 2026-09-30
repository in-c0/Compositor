//! Reading a DNG: the TIFF directories, the raw image's samples (uncompressed, one strip or
//! several) and the tags the develop needs.

use crate::{ImportError, Result};
use std::collections::HashMap;

/// One directory entry: field type, count and the value's bytes.
struct Field {
    kind: u16,
    count: usize,
    bytes: Vec<u8>,
}

struct Tiff<'a> {
    data: &'a [u8],
    big: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.data.get(at..at + 2)?;
        Some(if self.big { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    }
    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.data.get(at..at + 4)?;
        Some(if self.big { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) })
    }

    fn directory(&self, offset: usize) -> Option<(HashMap<u16, Field>, usize)> {
        let count = self.u16(offset)? as usize;
        let mut fields = HashMap::new();
        for i in 0..count {
            let entry = offset + 2 + i * 12;
            let (tag, kind, n) = (self.u16(entry)?, self.u16(entry + 2)?, self.u32(entry + 4)? as usize);
            let size: usize = match kind {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 => 8,
                _ => continue,
            };
            let length = size.checked_mul(n)?;
            let at = if length <= 4 { entry + 8 } else { self.u32(entry + 8)? as usize };
            let bytes = self.data.get(at..at.checked_add(length)?)?.to_vec();
            fields.insert(tag, Field { kind, count: n, bytes });
        }
        let next = self.u32(offset + 2 + count * 12).unwrap_or(0) as usize;
        Some((fields, next))
    }

    /// A field's values as numbers.
    fn values(&self, field: &Field) -> Vec<f64> {
        let read = |i: usize, size: usize| -> u64 {
            let b = &field.bytes[i * size..(i + 1) * size];
            let mut v = 0u64;
            for k in 0..size {
                let byte = if self.big { b[k] } else { b[size - 1 - k] };
                v = (v << 8) | byte as u64;
            }
            v
        };
        (0..field.count)
            .map(|i| match field.kind {
                1 | 7 => field.bytes[i] as f64,
                6 => field.bytes[i] as i8 as f64,
                3 => read(i, 2) as f64,
                8 => read(i, 2) as i16 as f64,
                4 | 13 => read(i, 4) as f64,
                9 => read(i, 4) as i32 as f64,
                5 => {
                    let (n, d) = (read(2 * i, 4) as f64, read(2 * i + 1, 4) as f64);
                    if d == 0.0 { 0.0 } else { n / d }
                }
                10 => {
                    let (n, d) = (read(2 * i, 4) as i32 as f64, read(2 * i + 1, 4) as i32 as f64);
                    if d == 0.0 { 0.0 } else { n / d }
                }
                11 => f32::from_bits(read(i, 4) as u32) as f64,
                12 => f64::from_bits(read(i, 8)),
                _ => 0.0,
            })
            .collect()
    }
}

/// The raw image and what the develop needs to know about it.
pub(crate) struct Raw {
    pub width: usize,
    pub height: usize,
    /// 1 for a color filter array, 3 for LinearRaw.
    pub samples_per_pixel: usize,
    /// The 2 x 2 color filter pattern (0 red, 1 green, 2 blue), for a CFA image.
    pub cfa: Option<[u8; 4]>,
    pub samples: Vec<u16>,
    /// Per sample, or per CFA position, as the file gives them; empty when missing or miscounted.
    pub black: Vec<f64>,
    pub white: Vec<f64>,
    pub color_matrix: Option<[[f64; 3]; 3]>,
    pub as_shot_neutral: Option<[f64; 3]>,
    pub baseline_exposure: f64,
    pub orientation: u16,
}

const NEW_SUBFILE_TYPE: u16 = 254;
const SUB_IFDS: u16 = 330;

pub(crate) fn read(data: &[u8]) -> Result<Raw> {
    let big = match data.get(0..4) {
        Some(b"II*\0") => false,
        Some(b"MM\0*") => true,
        _ => return Err(ImportError::Unreadable),
    };
    let tiff = Tiff { data, big };
    let first = tiff.u32(4).ok_or(ImportError::Unreadable)? as usize;
    let (ifd0, _) = tiff.directory(first).ok_or(ImportError::Unreadable)?;
    if !ifd0.contains_key(&50706) {
        return Err(ImportError::NotPorted("camera RAW formats other than DNG".into()));
    }
    // The main image: IFD 0 itself, or the SubIFD whose NewSubFileType is 0.
    let mut candidates = vec![];
    if let Some(sub) = ifd0.get(&SUB_IFDS) {
        for offset in tiff.values(sub) {
            if let Some((ifd, _)) = tiff.directory(offset as usize) {
                candidates.push(ifd);
            }
        }
    }
    let is_main = |ifd: &HashMap<u16, Field>| ifd.get(&NEW_SUBFILE_TYPE).map_or(true, |f| tiff.values(f).first() == Some(&0.0));
    let raw_ifd = if is_main(&ifd0) { &ifd0 } else { candidates.iter().find(|ifd| is_main(ifd)).ok_or(ImportError::Unreadable)? };
    let number = |ifd: &HashMap<u16, Field>, tag: u16| ifd.get(&tag).and_then(|f| tiff.values(f).first().copied());
    let numbers = |tag: u16| raw_ifd.get(&tag).or_else(|| ifd0.get(&tag)).map(|f| tiff.values(f)).unwrap_or_default();
    let width = number(raw_ifd, 256).ok_or(ImportError::Unreadable)? as usize;
    let height = number(raw_ifd, 257).ok_or(ImportError::Unreadable)? as usize;
    crate::check_size(width as u64, height as u64)?;
    let photometric = number(raw_ifd, 262).unwrap_or(0.0) as u32;
    let samples_per_pixel = number(raw_ifd, 277).unwrap_or(1.0) as usize;
    let bits = number(raw_ifd, 258).unwrap_or(16.0) as u32;
    let compression = number(raw_ifd, 259).unwrap_or(1.0) as u32;
    if compression != 1 {
        return Err(ImportError::NotPorted(format!("DNG compression {compression}")));
    }
    if bits != 16 {
        return Err(ImportError::NotPorted(format!("DNG with {bits}-bit samples")));
    }
    let cfa = match (photometric, samples_per_pixel) {
        (32803, 1) => {
            if numbers(33421) != [2.0, 2.0] {
                return Err(ImportError::NotPorted("a color filter pattern other than 2 x 2".into()));
            }
            let p = numbers(33422);
            if p.len() != 4 {
                return Err(ImportError::Unreadable);
            }
            Some([p[0] as u8, p[1] as u8, p[2] as u8, p[3] as u8])
        }
        (34892, 3) => None,
        _ => return Err(ImportError::NotPorted(format!("DNG photometric interpretation {photometric}"))),
    };
    let offsets = numbers(273);
    let counts = numbers(279);
    let mut bytes = Vec::with_capacity(width * height * samples_per_pixel * 2);
    for (offset, count) in offsets.iter().zip(&counts) {
        let (o, c) = (*offset as usize, *count as usize);
        bytes.extend_from_slice(data.get(o..o + c).ok_or(ImportError::Unreadable)?);
    }
    let needed = width * height * samples_per_pixel;
    if bytes.len() < needed * 2 {
        return Err(ImportError::Unreadable);
    }
    let samples: Vec<u16> = bytes[..needed * 2]
        .chunks_exact(2)
        .map(|b| if big { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
        .collect();
    // The specification counts BlackLevel per repeat cell and sample, and WhiteLevel per sample.
    let repeat = numbers(50713);
    let cells = if repeat.len() == 2 { (repeat[0] * repeat[1]) as usize } else { 1 };
    let black = numbers(50714);
    let black = if black.len() == cells * samples_per_pixel { black } else { Vec::new() };
    let white = numbers(50717);
    let matrix = numbers(50721);
    let color_matrix = (matrix.len() == 9).then(|| [[matrix[0], matrix[1], matrix[2]], [matrix[3], matrix[4], matrix[5]], [matrix[6], matrix[7], matrix[8]]]);
    let neutral = numbers(50728);
    let as_shot_neutral = (neutral.len() == 3).then(|| [neutral[0], neutral[1], neutral[2]]);
    Ok(Raw {
        width,
        height,
        samples_per_pixel,
        cfa,
        samples,
        black,
        white,
        color_matrix,
        as_shot_neutral,
        baseline_exposure: numbers(50730).first().copied().unwrap_or(0.0),
        orientation: number(&ifd0, 274).unwrap_or(1.0) as u16,
    })
}
