//! `Rendering/WandPixels.c`, ported line for line: the Magic Wand's match and flood fill, Color
//! Range's match, and the tracer that turns a mask into pixel-edge loops. These pick pixels and
//! trace outlines rather than draw anything, so they run on the CPU as the Mac's C does.

const EAST: u8 = 1;
const SOUTH: u8 = 2;
const WEST: u8 = 4;
const NORTH: u8 = 8;
/// Outlines with more pixel edges than this are refused: the path would be too slow to draw.
const EDGE_LIMIT: usize = 8_000_000;

fn matches(p: &[u8], reference: &[i32; 4], tolerance: i32) -> bool {
    (0..4).all(|c| {
        let d = p[c] as i32 - reference[c];
        d >= -tolerance && d <= tolerance
    })
}

/// `wand_mask` over premultiplied RGBA rows packed `width * 4` bytes apart: 255 where selected.
pub fn wand_mask(rgba: &[u8], width: usize, height: usize, seed: (usize, usize), radius: usize, tolerance: i32, contiguous: bool) -> Vec<u8> {
    let mut mask = vec![0u8; width * height];
    let (seed_x, seed_y) = seed;
    if width == 0 || height == 0 || seed_x >= width || seed_y >= height {
        return mask;
    }
    let stride = width * 4;
    let x0 = seed_x.saturating_sub(radius);
    let x1 = (seed_x + radius).min(width - 1);
    let y0 = seed_y.saturating_sub(radius);
    let y1 = (seed_y + radius).min(height - 1);
    let mut sums = [0u64; 4];
    let mut samples = 0u64;
    for y in y0..=y1 {
        for x in x0..=x1 {
            for c in 0..4 {
                sums[c] += rgba[y * stride + x * 4 + c] as u64;
            }
            samples += 1;
        }
    }
    let reference = sums.map(|s| ((s + samples / 2) / samples) as i32);
    let at = |x: usize, y: usize| &rgba[y * stride + x * 4..y * stride + x * 4 + 4];

    if !contiguous {
        for y in 0..height {
            for x in 0..width {
                if matches(at(x, y), &reference, tolerance) {
                    mask[y * width + x] = 255;
                }
            }
        }
        return mask;
    }

    // Scanline flood fill: each popped seed fills its whole horizontal run, then pushes one seed
    // per matching run in the rows directly above and below it.
    let mut stack = vec![(seed_x, seed_y)];
    while let Some((x, y)) = stack.pop() {
        let row = y * width;
        if mask[row + x] != 0 || !matches(at(x, y), &reference, tolerance) {
            continue;
        }
        let (mut left, mut right) = (x, x);
        while left > 0 && mask[row + left - 1] == 0 && matches(at(left - 1, y), &reference, tolerance) {
            left -= 1;
        }
        while right + 1 < width && mask[row + right + 1] == 0 && matches(at(right + 1, y), &reference, tolerance) {
            right += 1;
        }
        mask[row + left..=row + right].fill(255);
        for side in 0..2 {
            if if side == 0 { y == 0 } else { y + 1 >= height } {
                continue;
            }
            let ny = if side == 0 { y - 1 } else { y + 1 };
            let mut in_run = false;
            for nx in left..=right {
                let candidate = mask[ny * width + nx] == 0 && matches(at(nx, ny), &reference, tolerance);
                if candidate && !in_run {
                    stack.push((nx, ny));
                }
                in_run = candidate;
            }
        }
    }
    mask
}

fn color_near(rgb: &[i32; 3], colors: &[[u8; 3]], fuzziness: i32) -> bool {
    colors.iter().any(|c| (0..3).all(|i| (rgb[i] - c[i] as i32).abs() <= fuzziness))
}

/// `color_range_mask`: 255 where a pixel's straight color is within `fuzziness` of an included
/// color and of no excluded one (the reverse with `invert`). Transparent pixels never match.
pub fn color_range_mask(rgba: &[u8], width: usize, height: usize, include: &[[u8; 3]], exclude: &[[u8; 3]], fuzziness: i32, invert: bool) -> Vec<u8> {
    let mut mask = vec![0u8; width * height];
    for (i, px) in rgba.chunks_exact(4).take(width * height).enumerate() {
        let mut hit = false;
        if px[3] != 0 {
            let a = px[3] as i32;
            let rgb = [0, 1, 2].map(|c| (px[c] as i32 * 255 + a / 2) / a);
            hit = color_near(&rgb, include, fuzziness) && !color_near(&rgb, exclude, fuzziness);
        }
        if invert {
            hit = !hit;
        }
        if hit {
            mask[i] = 255;
        }
    }
    mask
}

// Headings, clockwise on screen (y grows downward): east, south, west, north.
fn turn_right(d: u8) -> u8 {
    if d == NORTH { EAST } else { d << 1 }
}

fn turn_left(d: u8) -> u8 {
    if d == EAST { NORTH } else { d >> 1 }
}

/// Why `wand_trace` gave up.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceError {
    /// More pixel edges than the Mac will outline.
    TooDetailed,
}

/// `wand_trace`: the outline of `mask`'s nonzero pixels as closed loops of corner points in
/// pixel-edge coordinates, outer boundaries clockwise and holes counterclockwise (top-left
/// origin), so the winding rule fills exactly those pixels.
pub fn wand_trace(mask: &[u8], width: usize, height: usize) -> Result<Vec<Vec<(i32, i32)>>, TraceError> {
    let mut loops = Vec::new();
    if width == 0 || height == 0 {
        return Ok(loops);
    }
    // Each vertex of the (width + 1) × (height + 1) grid records the directed boundary edges
    // leaving it: a selected pixel's unselected sides, walked clockwise around the pixel.
    let stride = width + 1;
    let vertices = stride * (height + 1);
    let mut out = vec![0u8; vertices];
    let mut edges = 0usize;
    for y in 0..height {
        let row = &mask[y * width..(y + 1) * width];
        for x in 0..width {
            if row[x] == 0 {
                continue;
            }
            if y == 0 || mask[(y - 1) * width + x] == 0 {
                out[y * stride + x] |= EAST;
                edges += 1;
            }
            if x + 1 == width || row[x + 1] == 0 {
                out[y * stride + x + 1] |= SOUTH;
                edges += 1;
            }
            if y + 1 == height || mask[(y + 1) * width + x] == 0 {
                out[(y + 1) * stride + x + 1] |= WEST;
                edges += 1;
            }
            if x == 0 || row[x - 1] == 0 {
                out[(y + 1) * stride + x] |= NORTH;
                edges += 1;
            }
        }
        if edges > EDGE_LIMIT {
            return Err(TraceError::TooDetailed);
        }
    }

    for start in 0..vertices {
        while out[start] != 0 {
            let mut points: Vec<(i32, i32)> = Vec::new();
            let mut v = start;
            let (mut heading, mut initial) = (0u8, 0u8);
            loop {
                let bits = out[v];
                // Where two loops meet at a corner, turning right keeps them apart.
                let lowest = bits & bits.wrapping_neg();
                let d = if heading == 0 {
                    lowest
                } else if bits & turn_right(heading) != 0 {
                    turn_right(heading)
                } else if bits & heading != 0 {
                    heading
                } else if bits & turn_left(heading) != 0 {
                    turn_left(heading)
                } else {
                    lowest
                };
                if d == 0 {
                    break;
                }
                out[v] &= !d;
                if d != heading {
                    points.push(((v % stride) as i32, (v / stride) as i32));
                }
                if heading == 0 {
                    initial = d;
                }
                heading = d;
                v = match d {
                    EAST => v + 1,
                    WEST => v - 1,
                    SOUTH => v + stride,
                    _ => v - stride,
                };
                if v == start {
                    break;
                }
            }
            // The start is a corner unless the loop arrives on the heading it left with.
            if heading == initial && !points.is_empty() {
                points.remove(0);
            }
            loops.push(points);
        }
    }
    Ok(loops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_a_square_clockwise() {
        let mask = [0, 0, 0, 0, 255, 255, 0, 255, 255];
        let loops = wand_trace(&mask, 3, 3).unwrap();
        assert_eq!(loops, vec![vec![(1, 1), (3, 1), (3, 3), (1, 3)]]);
    }

    #[test]
    fn corner_touching_cells_stay_apart() {
        // Two pixels meeting only at a corner are two loops.
        let mask = [255, 0, 0, 255];
        let loops = wand_trace(&mask, 2, 2).unwrap();
        assert_eq!(loops.len(), 2);
        assert!(loops.iter().all(|l| l.len() == 4));
    }

    #[test]
    fn a_hole_runs_counterclockwise() {
        let mut mask = [255u8; 9];
        mask[4] = 0;
        let loops = wand_trace(&mask, 3, 3).unwrap();
        assert_eq!(loops.len(), 2);
        let area = |l: &Vec<(i32, i32)>| {
            (0..l.len()).map(|i| {
                let (a, b) = (l[i], l[(i + 1) % l.len()]);
                (a.0 * b.1 - b.0 * a.1) as i64
            }).sum::<i64>()
        };
        assert_eq!(area(&loops[0]), 18);
        assert_eq!(area(&loops[1]), -2);
    }

    #[test]
    fn contiguous_fill_stops_at_other_colors() {
        let px = |v: u8| [v, v, v, 255];
        let rgba: Vec<u8> = [0u8, 0, 200, 0, 200, 0, 200, 0, 0].iter().flat_map(|&v| px(v)).collect();
        let mask = wand_mask(&rgba, 3, 3, (0, 0), 0, 10, true);
        assert_eq!(mask, [255, 255, 0, 255, 0, 0, 0, 0, 0]);
        let all = wand_mask(&rgba, 3, 3, (0, 0), 0, 10, false);
        assert_eq!(all.iter().filter(|&&m| m != 0).count(), 6);
    }
}
