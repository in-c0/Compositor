//! A JPEG entropy decoder that stops at the DCT coefficients: baseline and progressive Huffman
//! JPEG, 8-bit, any sampling factors, restart intervals. The inverse DCT and the color conversion
//! are Apple's and live in `jpeg.rs`; this part is fixed by the standard.

use crate::{ImportError, Result};

/// Zigzag position to natural (row-major) index.
pub(crate) const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57,
    50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

pub(crate) struct Component {
    pub id: u8,
    pub h: usize,
    pub v: usize,
    pub quant: usize,
    /// Blocks across and down, padded to whole MCUs.
    pub blocks_wide: usize,
    pub blocks_high: usize,
    /// Quantized coefficients, natural order, block by block row by row.
    pub coefficients: Vec<[i32; 64]>,
    dc_table: usize,
    ac_table: usize,
    dc_predictor: i32,
}

pub(crate) struct Coefficients {
    pub width: usize,
    pub height: usize,
    pub components: Vec<Component>,
    /// Quantization tables, natural order.
    pub quant: [[u16; 64]; 4],
    pub max_h: usize,
    pub max_v: usize,
    /// The Adobe APP14 transform flag, when there is one.
    pub adobe_transform: Option<u8>,
    pub progressive: bool,
}

#[derive(Clone, Default)]
struct Huffman {
    /// (length, code) to symbol, as a lookup by length.
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn new(bits: &[u8; 16], values: Vec<u8>) -> Huffman {
        let mut h = Huffman { values, ..Default::default() };
        let (mut code, mut k) = (0i32, 0i32);
        for l in 1..=16 {
            let n = bits[l - 1] as i32;
            if n > 0 {
                h.valptr[l] = k;
                h.mincode[l] = code;
                code += n;
                k += n;
                h.maxcode[l] = code - 1;
            } else {
                h.maxcode[l] = -1;
            }
            code <<= 1;
        }
        h.maxcode[17] = i32::MAX;
        h
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    count: u32,
    /// A marker was reached: the rest reads as zeros.
    marker: bool,
}

impl<'a> Bits<'a> {
    fn fill(&mut self) {
        while self.count <= 24 {
            let mut byte = 0u32;
            if !self.marker && self.pos < self.data.len() {
                let b = self.data[self.pos];
                if b == 0xff {
                    let next = self.data.get(self.pos + 1).copied().unwrap_or(0);
                    if next == 0 {
                        self.pos += 2;
                        byte = 0xff;
                    } else {
                        self.marker = true;
                    }
                } else {
                    self.pos += 1;
                    byte = b as u32;
                }
            }
            self.acc |= byte << (24 - self.count);
            self.count += 8;
        }
    }
    fn bit(&mut self) -> u32 {
        self.bits(1)
    }
    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        self.fill();
        let v = self.acc >> (32 - n);
        self.acc <<= n;
        self.count -= n;
        v
    }
    fn decode(&mut self, h: &Huffman) -> Result<u8> {
        let mut code = 0i32;
        for l in 1..=16 {
            code = (code << 1) | self.bit() as i32;
            if code <= h.maxcode[l] {
                return h.values.get((h.valptr[l] + code - h.mincode[l]) as usize).copied().ok_or(ImportError::Unreadable);
            }
        }
        Err(ImportError::Unreadable)
    }
    fn receive_extend(&mut self, s: u32) -> i32 {
        if s == 0 {
            return 0;
        }
        let v = self.bits(s) as i32;
        if v < 1 << (s - 1) { v - (1 << s) + 1 } else { v }
    }
    /// Skips to the byte after the next RSTn marker.
    fn restart(&mut self) {
        self.acc = 0;
        self.count = 0;
        self.marker = false;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xff && (0xd0..=0xd7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }
}

fn u16_at(data: &[u8], at: usize) -> Result<usize> {
    data.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize).ok_or(ImportError::Unreadable)
}

/// Every coefficient of the file's first frame.
pub(crate) fn read(data: &[u8]) -> Result<Coefficients> {
    if !data.starts_with(&[0xff, 0xd8]) {
        return Err(ImportError::Unreadable);
    }
    let mut pos = 2;
    let mut quant = [[1u16; 64]; 4];
    let mut dc: Vec<Huffman> = vec![Huffman::default(); 4];
    let mut ac: Vec<Huffman> = vec![Huffman::default(); 4];
    let mut frame: Option<Coefficients> = None;
    let mut restart_interval = 0usize;
    let mut adobe_transform = None;
    loop {
        while data.get(pos) == Some(&0xff) && data.get(pos + 1) == Some(&0xff) {
            pos += 1;
        }
        if data.get(pos) != Some(&0xff) {
            return Err(ImportError::Unreadable);
        }
        let marker = *data.get(pos + 1).ok_or(ImportError::Unreadable)?;
        pos += 2;
        if marker == 0xd9 {
            break;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            continue;
        }
        let length = u16_at(data, pos)?;
        let body = data.get(pos + 2..pos + length).ok_or(ImportError::Unreadable)?;
        pos += length;
        match marker {
            0xdb => {
                let mut i = 0;
                while i < body.len() {
                    let (precision, id) = (body[i] >> 4, (body[i] & 3) as usize);
                    i += 1;
                    for k in 0..64 {
                        let v = if precision == 0 {
                            *body.get(i + k).ok_or(ImportError::Unreadable)? as u16
                        } else {
                            u16_at(body, i + 2 * k)? as u16
                        };
                        quant[id][ZIGZAG[k]] = v;
                    }
                    i += 64 * (1 + precision as usize);
                }
            }
            0xc4 => {
                let mut i = 0;
                while i < body.len() {
                    let (class, id) = (body[i] >> 4, (body[i] & 3) as usize);
                    let bits: [u8; 16] = body.get(i + 1..i + 17).ok_or(ImportError::Unreadable)?.try_into().unwrap();
                    let n: usize = bits.iter().map(|&b| b as usize).sum();
                    let values = body.get(i + 17..i + 17 + n).ok_or(ImportError::Unreadable)?.to_vec();
                    let table = Huffman::new(&bits, values);
                    if class == 0 { dc[id] = table } else { ac[id] = table }
                    i += 17 + n;
                }
            }
            0xdd => restart_interval = u16_at(body, 0)?,
            0xee if body.starts_with(b"Adobe") && body.len() >= 12 => adobe_transform = Some(body[11]),
            0xc0 | 0xc1 | 0xc2 => {
                if frame.is_some() || body.first() != Some(&8) {
                    return Err(ImportError::NotPorted("JPEG with more than 8 bits per sample".into()));
                }
                let height = u16_at(body, 1)?;
                let width = u16_at(body, 3)?;
                let count = *body.get(5).ok_or(ImportError::Unreadable)? as usize;
                let mut components = Vec::new();
                for c in 0..count {
                    let b = body.get(6 + 3 * c..9 + 3 * c).ok_or(ImportError::Unreadable)?;
                    let (h, v) = ((b[1] >> 4) as usize, (b[1] & 15) as usize);
                    if !(1..=4).contains(&h) || !(1..=4).contains(&v) {
                        return Err(ImportError::Unreadable);
                    }
                    components.push(Component {
                        id: b[0],
                        h,
                        v,
                        quant: (b[2] & 3) as usize,
                        blocks_wide: 0,
                        blocks_high: 0,
                        coefficients: Vec::new(),
                        dc_table: 0,
                        ac_table: 0,
                        dc_predictor: 0,
                    });
                }
                if width == 0 || height == 0 || components.is_empty() {
                    return Err(ImportError::Unreadable);
                }
                crate::check_size(width as u64, height as u64)?;
                let max_h = components.iter().map(|c| c.h).max().unwrap();
                let max_v = components.iter().map(|c| c.v).max().unwrap();
                let (mcus_wide, mcus_high) = (width.div_ceil(8 * max_h), height.div_ceil(8 * max_v));
                for c in &mut components {
                    c.blocks_wide = mcus_wide * c.h;
                    c.blocks_high = mcus_high * c.v;
                    c.coefficients = vec![[0; 64]; c.blocks_wide * c.blocks_high];
                }
                frame = Some(Coefficients {
                    width,
                    height,
                    components,
                    quant,
                    max_h,
                    max_v,
                    adobe_transform: None,
                    progressive: marker == 0xc2,
                });
            }
            0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                return Err(ImportError::NotPorted("lossless, hierarchical or arithmetic-coded JPEG".into()));
            }
            0xda => {
                let f = frame.as_mut().ok_or(ImportError::Unreadable)?;
                let count = body[0] as usize;
                let mut scan = Vec::new();
                for i in 0..count {
                    let (id, tables) = (body[1 + 2 * i], body[2 + 2 * i]);
                    let index = f.components.iter().position(|c| c.id == id).ok_or(ImportError::Unreadable)?;
                    f.components[index].dc_table = (tables >> 4) as usize & 3;
                    f.components[index].ac_table = (tables & 15) as usize & 3;
                    scan.push(index);
                }
                let tail = &body[1 + 2 * count..];
                let (ss, se, ah, al) = (tail[0] as usize, tail[1] as usize, (tail[2] >> 4) as u32, (tail[2] & 15) as u32);
                // The entropy-coded data runs to the next marker that isn't RSTn.
                let start = pos;
                let mut end = pos;
                while end + 1 < data.len() && !(data[end] == 0xff && data[end + 1] != 0 && !(0xd0..=0xd7).contains(&data[end + 1])) {
                    end += 1;
                }
                let mut bits = Bits { data: &data[start..end], pos: 0, acc: 0, count: 0, marker: false };
                for &c in &scan {
                    f.components[c].dc_predictor = 0;
                }
                let mut eob_run = 0;
                f.quant = quant;
                decode_scan(f, &scan, &dc, &ac, &mut bits, (ss, se, ah, al), restart_interval, &mut eob_run)?;
                pos = end;
            }
            _ => {}
        }
    }
    let mut f = frame.ok_or(ImportError::Unreadable)?;
    f.adobe_transform = adobe_transform;
    Ok(f)
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    f: &mut Coefficients,
    scan: &[usize],
    dc: &[Huffman],
    ac: &[Huffman],
    bits: &mut Bits<'_>,
    (ss, se, ah, al): (usize, usize, u32, u32),
    restart_interval: usize,
    eob_run: &mut u32,
) -> Result<()> {
    let single = scan.len() == 1;
    // A scan of one component covers only its own blocks, not whole MCUs.
    let (units_wide, units_high) = if single {
        let c = &f.components[scan[0]];
        ((f.width * c.h).div_ceil(8 * f.max_h), (f.height * c.v).div_ceil(8 * f.max_v))
    } else {
        (f.width.div_ceil(8 * f.max_h), f.height.div_ceil(8 * f.max_v))
    };
    let mut restarts_left = restart_interval;
    for uy in 0..units_high {
        for ux in 0..units_wide {
            if restart_interval > 0 {
                if restarts_left == 0 {
                    bits.restart();
                    for &c in scan {
                        f.components[c].dc_predictor = 0;
                    }
                    *eob_run = 0;
                    restarts_left = restart_interval;
                }
                restarts_left -= 1;
            }
            for &ci in scan {
                let (h, v) = if single { (1, 1) } else { (f.components[ci].h, f.components[ci].v) };
                for by in 0..v {
                    for bx in 0..h {
                        let c = &mut f.components[ci];
                        let (row, col) = (uy * v + by, ux * h + bx);
                        let index = row * c.blocks_wide + col;
                        let (dct, act) = (&dc[c.dc_table], &ac[c.ac_table]);
                        let mut predictor = c.dc_predictor;
                        let block = &mut c.coefficients[index];
                        decode_block(block, bits, dct, act, &mut predictor, (ss, se, ah, al), eob_run, !f.progressive)?;
                        f.components[ci].dc_predictor = predictor;
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_block(
    block: &mut [i32; 64],
    bits: &mut Bits<'_>,
    dc: &Huffman,
    ac: &Huffman,
    predictor: &mut i32,
    (ss, se, ah, al): (usize, usize, u32, u32),
    eob_run: &mut u32,
    baseline: bool,
) -> Result<()> {
    if baseline {
        let s = bits.decode(dc)? as u32;
        *predictor += bits.receive_extend(s);
        block[0] = *predictor;
        let mut k = 1;
        while k < 64 {
            let rs = bits.decode(ac)?;
            let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
            if s == 0 {
                if r == 15 {
                    k += 16;
                    continue;
                }
                break;
            }
            k += r;
            if k > 63 {
                return Err(ImportError::Unreadable);
            }
            block[ZIGZAG[k]] = bits.receive_extend(s);
            k += 1;
        }
        return Ok(());
    }
    if ss == 0 {
        // DC scan.
        if ah == 0 {
            let s = bits.decode(dc)? as u32;
            *predictor += bits.receive_extend(s);
            block[0] = *predictor << al;
        } else if bits.bit() == 1 {
            block[0] |= 1 << al;
        }
        return Ok(());
    }
    if ah == 0 {
        // AC first pass.
        if *eob_run > 0 {
            *eob_run -= 1;
            return Ok(());
        }
        let mut k = ss;
        while k <= se {
            let rs = bits.decode(ac)?;
            let (r, s) = ((rs >> 4) as u32, (rs & 15) as u32);
            if s == 0 {
                if r < 15 {
                    *eob_run = (1 << r) - 1;
                    if r > 0 {
                        *eob_run += bits.bits(r);
                    }
                    break;
                }
                k += 16;
                continue;
            }
            k += r as usize;
            if k > 63 {
                return Err(ImportError::Unreadable);
            }
            block[ZIGZAG[k]] = bits.receive_extend(s) * (1 << al);
            k += 1;
        }
        return Ok(());
    }
    // AC refinement.
    let p1 = 1i32 << al;
    let m1 = -1i32 << al;
    let mut k = ss;
    if *eob_run == 0 {
        while k <= se {
            let rs = bits.decode(ac)?;
            let (mut r, s) = ((rs >> 4) as i32, (rs & 15) as u32);
            let mut value = 0;
            if s != 0 {
                value = if bits.bit() == 1 { p1 } else { m1 };
            } else if r != 15 {
                *eob_run = 1 << r;
                if r > 0 {
                    *eob_run += bits.bits(r as u32);
                }
                break;
            }
            // Past `r` zero coefficients (correcting the nonzero ones on the way) to the one
            // that takes the new value; ZRL passes 16 zeros.
            loop {
                let z = ZIGZAG[k];
                if block[z] != 0 {
                    if bits.bit() == 1 && (block[z] & p1) == 0 {
                        block[z] += if block[z] >= 0 { p1 } else { m1 };
                    }
                } else {
                    if r == 0 {
                        break;
                    }
                    r -= 1;
                }
                k += 1;
                if k > se {
                    break;
                }
            }
            if value != 0 && k <= se {
                block[ZIGZAG[k]] = value;
            }
            k += 1;
        }
    }
    if *eob_run > 0 {
        while k <= se {
            let z = ZIGZAG[k];
            if block[z] != 0 && bits.bit() == 1 && (block[z] & p1) == 0 {
                block[z] += if block[z] >= 0 { p1 } else { m1 };
            }
            k += 1;
        }
        *eob_run -= 1;
    }
    Ok(())
}
