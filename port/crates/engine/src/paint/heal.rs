//! Spot Healing: `HealPixels.c`, ported line for line. The solver is Gauss–Seidel with
//! over-relaxation, where each pixel's update reads the ones just updated before it, so it runs
//! in order on the CPU, as the Mac runs it; a parallel version would converge to different bytes.
//!
//! Apple's clang contracts `a * b + c` inside one expression into a fused multiply-add on arm64
//! (`-ffp-contract=on`), so those sites use `mul_add` here.

const OUTSIDE: u8 = 0;
const RING: u8 = 1;
const HOLE: u8 = 2;

/// `heal_coverage_bounds`: half-open bounds of the nonzero bytes, all zero when there are none.
pub fn coverage_bounds(gray: &[u8], width: usize, height: usize, stride: usize) -> [i64; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (width as i64, height as i64, 0i64, 0i64);
    for y in 0..height {
        let row = &gray[y * stride..];
        for x in 0..width {
            if row[x] == 0 {
                continue;
            }
            let (x, y) = (x as i64, y as i64);
            x0 = x0.min(x);
            x1 = x1.max(x + 1);
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return [0; 4];
    }
    [x0, y0, x1, y1]
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn unit(key: u32) -> f64 {
    (hash(key) >> 8) as f64 / 16_777_216.0
}

/// C's `lround`: to nearest, halves away from zero.
fn lround(v: f64) -> i64 {
    v.round() as i64
}

#[allow(clippy::too_many_arguments)]
fn score(rgba: &[u8], stride: usize, role: &[u8], wx0: i64, wy0: i64, ww: i64, wh: i64, dx: i64, dy: i64, w: i64, h: i64) -> f64 {
    if dx.abs() < ww && dy.abs() < wh {
        return f64::INFINITY;
    }
    if wx0 + dx < 0 || wy0 + dy < 0 || wx0 + ww + dx > w || wy0 + wh + dy > h {
        return f64::INFINITY;
    }
    let mut sum = 0.0f64;
    let mut n = 0i64;
    for y in 0..wh {
        for x in 0..ww {
            if role[(y * ww + x) as usize] != RING {
                continue;
            }
            let t = (wy0 + y) as usize * stride + (wx0 + x) as usize * 4;
            let s = (wy0 + y + dy) as usize * stride + (wx0 + x + dx) as usize * 4;
            for c in 0..4 {
                let d = rgba[t + c] as f64 - rgba[s + c] as f64;
                sum = d.mul_add(d, sum);
            }
            n += 1;
        }
    }
    if n > 0 { sum / n as f64 } else { f64::INFINITY }
}

fn solve(value: &mut [f32], role: &[u8], w: i64, h: i64, depth: u32) {
    let mut iterations = 300;
    if w > 32 && h > 32 && depth < 16 {
        let (cw, ch) = ((w + 1) / 2, (h + 1) / 2);
        let mut coarse = vec![0f32; (cw * ch * 4) as usize];
        let mut coarse_role = vec![0u8; (cw * ch) as usize];
        for y in 0..ch {
            for x in 0..cw {
                let (mut known, mut hole) = (0i32, 0i32);
                let (mut known_sum, mut hole_sum) = ([0f32; 4], [0f32; 4]);
                for j in 0..2 {
                    for i in 0..2 {
                        let (fx, fy) = (x * 2 + i, y * 2 + j);
                        if fx >= w || fy >= h {
                            continue;
                        }
                        let p = (fy * w + fx) as usize;
                        if role[p] == RING {
                            known += 1;
                            for c in 0..4 {
                                known_sum[c] += value[p * 4 + c];
                            }
                        } else if role[p] == HOLE {
                            hole += 1;
                            for c in 0..4 {
                                hole_sum[c] += value[p * 4 + c];
                            }
                        }
                    }
                }
                let q = (y * cw + x) as usize;
                if known > 0 {
                    coarse_role[q] = RING;
                    for c in 0..4 {
                        coarse[q * 4 + c] = known_sum[c] / known as f32;
                    }
                } else if hole > 0 {
                    coarse_role[q] = HOLE;
                    for c in 0..4 {
                        coarse[q * 4 + c] = hole_sum[c] / hole as f32;
                    }
                }
            }
        }
        solve(&mut coarse, &coarse_role, cw, ch, depth + 1);
        for y in 0..h {
            for x in 0..w {
                let (p, q) = ((y * w + x) as usize, ((y / 2) * cw + x / 2) as usize);
                if role[p] == HOLE && coarse_role[q] == HOLE {
                    value[p * 4..p * 4 + 4].copy_from_slice(&coarse[q * 4..q * 4 + 4]);
                }
            }
        }
        iterations = 40;
    }
    let omega = 1.8f32;
    for _ in 0..iterations {
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) as usize;
                if role[p] != HOLE {
                    continue;
                }
                let mut sum = [0f32; 4];
                let mut n = 0i32;
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let q = (ny * w + nx) as usize;
                    if role[q] == OUTSIDE {
                        continue;
                    }
                    for c in 0..4 {
                        sum[c] += value[q * 4 + c];
                    }
                    n += 1;
                }
                if n == 0 {
                    continue;
                }
                for c in 0..4 {
                    let v = value[p * 4 + c];
                    value[p * 4 + c] = omega.mul_add(sum[c] / n as f32 - v, v);
                }
            }
        }
    }
}

/// `spot_heal`, in place over premultiplied RGBA. `mode` 0 Content-Aware, 1 Create Texture,
/// 2 Proximity Match.
#[allow(clippy::too_many_arguments)]
pub fn spot_heal(rgba: &mut [u8], coverage: &[u8], width: usize, height: usize, stride: usize, opacity: f32, mode: i32, seed: u32) {
    let (w, h) = (width as i64, height as i64);
    let bounds = coverage_bounds(coverage, width, height, width);
    if bounds[2] <= bounds[0] {
        return;
    }
    let (bw, bh) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let size = bw.max(bh);
    let ring = (size / 8).clamp(2, 16);
    let wx0 = (bounds[0] - ring).max(0);
    let wy0 = (bounds[1] - ring).max(0);
    let wx1 = (bounds[2] + ring).min(w);
    let wy1 = (bounds[3] + ring).min(h);
    let (ww, wh) = (wx1 - wx0, wy1 - wy0);
    let wn = (ww * wh) as usize;
    let mut role = vec![OUTSIDE; wn];
    let mut near = vec![0u8; wn];
    let mut prefix = vec![0i64; (ww.max(wh) + 1) as usize];
    let mut value = vec![0f32; wn * 4];
    for y in 0..wh {
        for x in 0..ww {
            let covered = coverage[(wy0 + y) as usize * width + (wx0 + x) as usize] != 0;
            role[(y * ww + x) as usize] = if covered { HOLE } else { OUTSIDE };
        }
    }
    // The ring: pixels within `ring` of the spot (a square dilation, row pass then column pass).
    for y in 0..wh {
        prefix[0] = 0;
        for x in 0..ww {
            prefix[(x + 1) as usize] = prefix[x as usize] + (role[(y * ww + x) as usize] == HOLE) as i64;
        }
        for x in 0..ww {
            let (lo, hi) = ((x - ring).max(0), (x + ring + 1).min(ww));
            near[(y * ww + x) as usize] = (prefix[hi as usize] - prefix[lo as usize] > 0) as u8;
        }
    }
    for x in 0..ww {
        prefix[0] = 0;
        for y in 0..wh {
            prefix[(y + 1) as usize] = prefix[y as usize] + near[(y * ww + x) as usize] as i64;
        }
        for y in 0..wh {
            let (lo, hi) = ((y - ring).max(0), (y + ring + 1).min(wh));
            let p = (y * ww + x) as usize;
            if role[p] == OUTSIDE && prefix[hi as usize] - prefix[lo as usize] > 0 {
                role[p] = RING;
            }
        }
    }
    let ring_count = role.iter().filter(|&&r| r == RING).count() as i64;
    if ring_count == 0 {
        return;
    }

    // Source patch for Content-Aware and Proximity Match.
    let (mut ox, mut oy) = (0i64, 0i64);
    let mut have_source = false;
    if mode != 1 {
        const FACTORS: [f64; 5] = [1.05, 1.35, 1.75, 2.25, 2.8];
        let count = if mode == 2 { 2 } else { 5 };
        let mut best = f64::INFINITY;
        for (f, factor) in FACTORS.iter().enumerate().take(count) {
            for a in 0..24 {
                let angle = a as f64 * std::f64::consts::PI / 12.0;
                let dx = lround(angle.cos() * factor * ww as f64);
                let dy = lround(angle.sin() * factor * wh as f64);
                let mut s = score(rgba, stride, &role, wx0, wy0, ww, wh, dx, dy, w, h);
                if !s.is_finite() {
                    continue;
                }
                // Nearer patches win ties.
                s *= if mode == 2 { 0.6f64.mul_add(f as f64, 1.0) } else { 0.1f64.mul_add(f as f64, 1.0) };
                if s < best {
                    best = s;
                    ox = dx;
                    oy = dy;
                }
            }
        }
        if best.is_finite() {
            // Fine-tune the alignment so repeating texture lines up.
            let (cx, cy) = (ox, oy);
            let mut refined = score(rgba, stride, &role, wx0, wy0, ww, wh, cx, cy, w, h);
            for j in -3..=3 {
                for i in -3..=3 {
                    let s = score(rgba, stride, &role, wx0, wy0, ww, wh, cx + i, cy + j, w, h);
                    if s < refined {
                        refined = s;
                        ox = cx + i;
                        oy = cy + j;
                    }
                }
            }
            have_source = true;
        }
    }

    // Membrane: the edge difference between the original and the patch (or the original itself
    // for a smooth fill), spread across the spot.
    let mut mean = [0f64; 4];
    let mut detail = [0f64; 3];
    let pixel = |x: i64, y: i64| y as usize * stride + x as usize * 4;
    for y in 0..wh {
        for x in 0..ww {
            let p = (y * ww + x) as usize;
            if role[p] != RING {
                value[p * 4..p * 4 + 4].fill(0.0);
                continue;
            }
            let (ix, iy) = (wx0 + x, wy0 + y);
            let t = pixel(ix, iy);
            let s = have_source.then(|| pixel(ix + ox, iy + oy));
            for c in 0..4 {
                value[p * 4 + c] = rgba[t + c] as f32 - s.map_or(0.0, |s| rgba[s + c] as f32);
                mean[c] += value[p * 4 + c] as f64;
            }
            if !have_source {
                // Fine detail around the spot: each pixel against the average of its neighbours.
                for c in 0..3 {
                    let mut around = 0f64;
                    let mut n = 0i32;
                    for (nx, ny) in [(ix - 1, iy), (ix + 1, iy), (ix, iy - 1), (ix, iy + 1)] {
                        if nx < 0 || ny < 0 || nx >= w || ny >= h {
                            continue;
                        }
                        around += rgba[pixel(nx, ny) + c] as f64;
                        n += 1;
                    }
                    if n > 0 {
                        let d = rgba[t + c] as f64 - around / n as f64;
                        detail[c] = d.mul_add(d, detail[c]);
                    }
                }
            }
        }
    }
    for m in &mut mean {
        *m /= ring_count as f64;
    }
    for p in 0..wn {
        if role[p] == HOLE {
            for c in 0..4 {
                value[p * 4 + c] = mean[c] as f32;
            }
        }
    }
    solve(&mut value, &role, ww, wh, 0);
    for d in &mut detail {
        *d = (*d / ring_count as f64).sqrt() * 0.9;
    }

    for y in 0..wh {
        for x in 0..ww {
            let p = (y * ww + x) as usize;
            if role[p] != HOLE {
                continue;
            }
            let (ix, iy) = (wx0 + x, wy0 + y);
            let t = pixel(ix, iy);
            let s = have_source.then(|| pixel(ix + ox, iy + oy));
            let amount = coverage[iy as usize * width + ix as usize] as f64 / 255.0 * opacity as f64;
            let mut grain = 0f64;
            if !have_source {
                let key = hash(seed ^ hash((iy * w + ix) as u32));
                let (u1, u2) = (unit(key), unit(key ^ 0x68e3_1da4));
                grain = (-2.0 * (1.0 - u1).ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            }
            let mut out = [0f64; 4];
            for c in 0..4 {
                let base = s.map_or(0.0, |s| rgba[s + c] as f64) + value[p * 4 + c] as f64;
                let healed = if c < 3 { grain.mul_add(detail[c], base) } else { base + 0.0 };
                let original = rgba[t + c] as f64;
                out[c] = (healed - original).mul_add(amount, original);
            }
            let alpha = out[3].clamp(0.0, 255.0);
            rgba[t + 3] = lround(alpha) as u8;
            for c in 0..3 {
                let v = if out[c] < 0.0 { 0.0 } else if out[c] > rgba[t + 3] as f64 { rgba[t + 3] as f64 } else { out[c] };
                rgba[t + c] = lround(v) as u8;
            }
        }
    }
}
