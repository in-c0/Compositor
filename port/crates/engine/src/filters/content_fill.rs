//! Content-Aware Fill: `ContentFill.run` (Document/ContentFill.swift) and `content_fill`
//! (Rendering/ContentFill.c), ported line for line.
//!
//! The kernel fills the selected pixels one at a time in the order a queue reaches them, each
//! copied from the unselected pixel whose 5 × 5 neighborhood matches best at that moment, so every
//! pixel depends on the ones filled before it. It runs in order on the CPU, as the Mac runs it.
//! The only arithmetic in floating point is the match score, a sum of integer squares divided by
//! a count in `double`, which `f64` reproduces exactly. The random probes come from the C's own
//! linear congruential generator with its fixed seed, so the result is the same every time.

/// The generator's state: `seed=0x6d2b79f5` in `content_fill`.
const SEED: u32 = 0x6d2b_79f5;

fn next_random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}

/// `match`: the mean squared difference, over the pixels of the two windows that are inside the
/// image and known around `p`, or `f64::MAX` (`DBL_MAX`) when there are none.
#[allow(clippy::too_many_arguments)]
fn score(pixels: &[u8], stride: usize, known: &[u8], w: i32, h: i32, p: i32, q: i32, radius: i32) -> f64 {
    let (px, py, qx, qy) = (p % w, p / w, q % w, q / w);
    let mut count = 0i32;
    let mut sum = 0.0f64;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (x, y, sx, sy) = (px + dx, py + dy, qx + dx, qy + dy);
            if x < 0 || y < 0 || x >= w || y >= h || sx < 0 || sy < 0 || sx >= w || sy >= h || known[(y * w + x) as usize] == 0 {
                continue;
            }
            let a = y as usize * stride + x as usize * 4;
            let b = sy as usize * stride + sx as usize * 4;
            for c in 0..4 {
                let d = pixels[a + c] as i32 - pixels[b + c] as i32;
                sum += (d * d) as f64;
            }
            count += 1;
        }
    }
    if count != 0 { sum / count as f64 } else { f64::MAX }
}

/// `content_fill`: fills the pixels whose `mask` byte isn't zero in `pixels` (premultiplied RGBA,
/// `stride` bytes a row) from the unselected opaque ones. Returns 1 when it filled (or there was
/// nothing to fill) and 0 when no pixel had a whole known neighborhood to copy from.
pub fn content_fill(pixels: &mut [u8], stride: usize, mask: &[u8], ms: usize, w: i32, h: i32) -> i32 {
    let n = w as usize * h as usize;
    let mut known = vec![0u8; n];
    let mut target = vec![0u8; n];
    let mut valid = vec![0u8; n];
    let mut queued = vec![0u8; n];
    let mut donors = vec![0i32; n];
    let mut queue = vec![0i32; n];
    let mut chosen = vec![0i32; n];
    let radius = if w >= 5 && h >= 5 { 2 } else { 0 };
    let (mut missing, mut donor_count, mut head, mut tail, mut scan) = (0usize, 0usize, 0usize, 0usize, 0usize);
    // Selected pixels are filled. Unselected opaque pixels are the image to match and copy from;
    // unselected transparent ones are neither.
    for y in 0..h {
        for x in 0..w {
            let p = (y * w + x) as usize;
            target[p] = (mask[y as usize * ms + x as usize] != 0) as u8;
            known[p] = (target[p] == 0 && pixels[y as usize * stride + x as usize * 4 + 3] == 255) as u8;
            chosen[p] = -1;
            if target[p] != 0 {
                missing += 1;
            }
        }
    }
    if missing == 0 {
        return 1;
    }
    for y in 0..h {
        for x in 0..w {
            let p = (y * w + x) as usize;
            if known[p] == 0 {
                continue;
            }
            let mut ok = true;
            'window: for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (sx, sy) = (x + dx, y + dy);
                    if sx < 0 || sy < 0 || sx >= w || sy >= h || known[(sy * w + sx) as usize] == 0 {
                        ok = false;
                        break 'window;
                    }
                }
            }
            if ok {
                valid[p] = 1;
                donors[donor_count] = p as i32;
                donor_count += 1;
            }
        }
    }
    if donor_count == 0 {
        return 0;
    }
    for y in 0..h {
        for x in 0..w {
            let p = (y * w + x) as usize;
            let touches = (x != 0 && known[p - 1] != 0)
                || (x + 1 < w && known[p + 1] != 0)
                || (y != 0 && known[p - w as usize] != 0)
                || (y + 1 < h && known[p + w as usize] != 0);
            if target[p] != 0 && touches {
                queue[tail] = p as i32;
                tail += 1;
                queued[p] = 1;
            }
        }
    }
    let mut seed = SEED;
    loop {
        while head < tail {
            let p = queue[head];
            head += 1;
            let (x, y) = (p % w, p / w);
            let mut best = -1i32;
            let mut best_score = f64::MAX;
            let neighbors = [
                if x != 0 { p - 1 } else { -1 },
                if x + 1 < w { p + 1 } else { -1 },
                if y != 0 { p - w } else { -1 },
                if y + 1 < h { p + w } else { -1 },
            ];
            // Propagate coherent source offsets, then refine with randomized patch search.
            for k in 0..28 {
                let mut q = -1i32;
                if k < 4 {
                    let t = neighbors[k];
                    if t >= 0 {
                        let from = chosen[t as usize];
                        q = (if from >= 0 { from } else { t }) + (p - t);
                    }
                } else {
                    q = donors[(next_random(&mut seed) % donor_count as u32) as usize];
                }
                if q < 0 || q as usize >= n || valid[q as usize] == 0 {
                    continue;
                }
                let s = score(pixels, stride, &known, w, h, p, q, radius);
                if best < 0 || s < best_score {
                    best_score = s;
                    best = q;
                }
            }
            if best < 0 {
                best = donors[0];
            }
            let mut r = 64i32;
            while r >= 1 {
                let span = (2 * r + 1) as u32;
                let qx = best % w + (next_random(&mut seed) % span) as i32 - r;
                let qy = best / w + (next_random(&mut seed) % span) as i32 - r;
                if !(qx < 0 || qy < 0 || qx >= w || qy >= h || valid[(qy * w + qx) as usize] == 0) {
                    let q = qy * w + qx;
                    let s = score(pixels, stride, &known, w, h, p, q, radius);
                    if s < best_score {
                        best_score = s;
                        best = q;
                    }
                }
                r /= 2;
            }
            let to = y as usize * stride + x as usize * 4;
            let from = (best / w) as usize * stride + (best % w) as usize * 4;
            pixels.copy_within(from..from + 4, to);
            known[p as usize] = 1;
            chosen[p as usize] = best;
            for &q in &neighbors {
                if q >= 0 && target[q as usize] != 0 && known[q as usize] == 0 && queued[q as usize] == 0 {
                    queued[q as usize] = 1;
                    queue[tail] = q;
                    tail += 1;
                }
            }
        }
        // A selected area that only transparency touches starts from the best random donor, then
        // spreads.
        while scan < n && (target[scan] == 0 || known[scan] != 0) {
            scan += 1;
        }
        if scan >= n {
            break;
        }
        queue[tail] = scan as i32;
        tail += 1;
        queued[scan] = 1;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vertical stripe pattern, 4 pixels a stripe, with a red block over it.
    fn stripes(w: usize, h: usize) -> Vec<u8> {
        let mut pixels = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if (x / 4) % 2 == 0 { 51 } else { 204 };
                pixels[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        pixels
    }

    #[test]
    fn continues_a_repeating_texture() {
        // SmartEditTests.fillContinuesRepeatingTexture: at least 114 of the 120 pixels continue
        // the stripes.
        let (w, h) = (80usize, 64usize);
        let mut pixels = stripes(w, h);
        let mut mask = vec![0u8; w * h];
        for y in 26..36 {
            for x in 32..44 {
                pixels[(y * w + x) * 4..][..4].copy_from_slice(&[255, 0, 0, 255]);
                mask[y * w + x] = 255;
            }
        }
        assert_eq!(content_fill(&mut pixels, w * 4, &mask, w, w as i32, h as i32), 1);
        let mut matching = 0;
        for y in 26..36 {
            for x in 32..44 {
                let want = if (x / 4) % 2 == 0 { 51 } else { 204 };
                matching += (pixels[(y * w + x) * 4] as i32 - want).abs() <= 1 && pixels[(y * w + x) * 4 + 1] == want as u8;
            }
        }
        assert!(matching >= 114, "matched {matching} of 120");
    }

    #[test]
    fn leaves_unselected_pixels_and_reports_no_source() {
        let (w, h) = (16usize, 12usize);
        let original = stripes(w, h);
        let mut pixels = original.clone();
        let mut mask = vec![0u8; w * h];
        mask[5 * w + 6] = 1;
        assert_eq!(content_fill(&mut pixels, w * 4, &mask, w, w as i32, h as i32), 1);
        for p in 0..w * h {
            if p != 5 * w + 6 {
                assert_eq!(pixels[p * 4..][..4], original[p * 4..][..4]);
            }
        }
        // Nothing unselected: no donor.
        let mut all = original.clone();
        assert_eq!(content_fill(&mut all, w * 4, &vec![255; w * h], w, w as i32, h as i32), 0);
        assert_eq!(all, original);
        // Nothing selected: done, untouched.
        let mut none = original.clone();
        assert_eq!(content_fill(&mut none, w * 4, &vec![0; w * h], w, w as i32, h as i32), 1);
        assert_eq!(none, original);
    }

    #[test]
    fn fills_an_area_only_transparency_touches() {
        // A selected pixel with no known neighbor is seeded from the scan and still filled.
        let (w, h) = (12usize, 12usize);
        let mut pixels = stripes(w, h);
        for y in 0..3 {
            for x in 0..3 {
                pixels[(y * w + x) * 4..][..4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
        let mut mask = vec![0u8; w * h];
        mask[w + 1] = 255;
        assert_eq!(content_fill(&mut pixels, w * 4, &mask, w, w as i32, h as i32), 1);
        assert_eq!(pixels[(w + 1) * 4 + 3], 255);
    }
}
