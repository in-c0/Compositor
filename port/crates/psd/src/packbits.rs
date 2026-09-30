//! PackBits, the run-length scheme Photoshop calls RLE (compression 1). Each row is coded on its
//! own: a header byte n in 0..=127 copies the next n + 1 bytes, n in -127..=-1 repeats the next
//! byte 1 - n times, and -128 is a no-op.

use anyhow::{Result, bail};

/// Packs one row. Runs of three or more equal bytes (or a pair ending the row) become repeats,
/// everything else literals, neither longer than 128 bytes; so a row never grows by more than
/// one byte in 128.
pub fn pack_bits(row: &[u8]) -> Vec<u8> {
    let n = row.len();
    let run_at = |i: usize| {
        let mut run = 1;
        while i + run < n && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        run
    };
    let mut out = Vec::with_capacity(n + n.div_ceil(128));
    let mut i = 0;
    while i < n {
        let run = run_at(i);
        if run >= 3 || (run == 2 && i + 2 == n) {
            out.push((1 - run as i16) as i8 as u8);
            out.push(row[i]);
            i += run;
        } else {
            let start = i;
            i += 1;
            while i < n && i - start < 128 && run_at(i) < 3 {
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend_from_slice(&row[start..i]);
        }
    }
    out
}

/// Unpacks exactly `len` bytes from `data`, which must hold nothing else.
pub fn unpack_bits(data: &[u8], len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    while out.len() < len {
        let Some(&header) = data.get(i) else { bail!("PackBits data ends after {} of {len} bytes", out.len()) };
        i += 1;
        let n = header as i8;
        if n >= 0 {
            let count = n as usize + 1;
            if out.len() + count > len || i + count > data.len() {
                bail!("PackBits literal overruns the row");
            }
            out.extend_from_slice(&data[i..i + count]);
            i += count;
        } else if n != -128 {
            let count = (1 - n as i16) as usize;
            let Some(&value) = data.get(i) else { bail!("PackBits repeat has no value") };
            if out.len() + count > len {
                bail!("PackBits repeat overruns the row");
            }
            i += 1;
            out.extend(std::iter::repeat_n(value, count));
        }
    }
    if i != data.len() {
        bail!("{} bytes left over after unpacking a PackBits row", data.len() - i);
    }
    Ok(out)
}
