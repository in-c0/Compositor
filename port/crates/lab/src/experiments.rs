use crate::{Score, load_cases};
use anyhow::{Result, bail};
use std::path::Path;

pub fn run(name: &str, corpus: &Path, refs: &Path, rest: &[String]) -> Result<()> {
    match name {
        "premultiply" => premultiply(corpus, refs),
        "normal" => normal(corpus, refs, rest.first().map(String::as_str).unwrap_or("blend/normal-*")),
        "dump" => dump(corpus, refs, &rest[0], rest.get(1).map(|n| n.parse().unwrap()).unwrap_or(30)),
        "csv" => csv(corpus, refs, &rest[0]),
        "lum" => {
            for w in [[0.3, 0.59, 0.11], [77.0 / 256.0, 151.0 / 256.0, 28.0 / 256.0], [0.299, 0.587, 0.114], [0.2126, 0.7152, 0.0722]] {
                *crate::blend::LUM.write().unwrap() = w;
                println!("weights {w:?}");
                modes(corpus, refs, rest.first().map(String::as_str).unwrap_or("blend/*-opaque-100"))?;
            }
            Ok(())
        }
        "modes" => modes(corpus, refs, rest.first().map(String::as_str).unwrap_or("blend/*-*")),
        other => bail!("unknown experiment {other}"),
    }
}

type Round = fn(u32, u32) -> u32;

fn div_round(x: u32, d: u32) -> u32 {
    (x + d / 2) / d
}
fn div_floor(x: u32, d: u32) -> u32 {
    x / d
}
fn div_ceil(x: u32, d: u32) -> u32 {
    x.div_ceil(d)
}
/// The shift-based x/255 many blitters use: (x + 1 + (x >> 8)) >> 8.
fn div_shift(x: u32, _d: u32) -> u32 {
    (x + 1 + (x >> 8)) >> 8
}

pub const ROUNDINGS: [(&str, Round); 4] = [("round", div_round), ("floor", div_floor), ("ceil", div_ceil), ("shift", div_shift)];

fn unpremultiply(p: [u32; 4], un: Round) -> [u8; 4] {
    let a = p[3];
    if a == 0 {
        return [0, 0, 0, 0];
    }
    let c = |v: u32| un(v * 255, a).min(255) as u8;
    [c(p[0]), c(p[1]), c(p[2]), a as u8]
}

/// A lone layer drawn onto an empty canvas and exported: straight -> premultiplied (decode or draw)
/// -> straight (PNG export). Which roundings reproduce the references?
fn premultiply(corpus: &Path, refs: &Path) -> Result<()> {
    let cases = load_cases(corpus, refs, "blend/empty-canvas")?;
    for (pn, pre) in ROUNDINGS {
        for (un, un_round) in ROUNDINGS {
            let mut score = Score::default();
            for case in &cases {
                for (s, r) in case.layer(0).pixels().zip(case.reference.pixels()) {
                    let a = s[3] as u32;
                    let p = [pre(s[0] as u32 * a, 255), pre(s[1] as u32 * a, 255), pre(s[2] as u32 * a, 255), a];
                    score.add_pixel(unpremultiply(p, un_round), r.0);
                }
            }
            println!("premultiply {pn:>5}, unpremultiply {un:>5}: {score}");
        }
    }
    Ok(())
}

/// Two layers, both Normal: the backdrop at full opacity, the source at the case's opacity.
/// Candidates vary how opacity is quantized and how each product is rounded.
fn normal(corpus: &Path, refs: &Path, pattern: &str) -> Result<()> {
    let cases = load_cases(corpus, refs, pattern)?;
    let pre: Round = div_round;
    let un: Round = div_round;
    for (gn, global) in [("alpha8", 0u8), ("float", 1u8)] {
        for (sn, scale) in ROUNDINGS {
            for (bn, blend) in ROUNDINGS {
                let mut score = Score::default();
                for case in &cases {
                    let opacity = case.record(1).opacity();
                    let g8 = (opacity * 255.0).round() as u32;
                    let (back, src) = (case.layer(0), case.layer(1));
                    for ((b, s), r) in back.pixels().zip(src.pixels()).zip(case.reference.pixels()) {
                        let bp = [0, 1, 2].map(|c| pre(b[c] as u32 * b[3] as u32, 255));
                        let bp = [bp[0], bp[1], bp[2], b[3] as u32];
                        let sp = [0, 1, 2].map(|c| pre(s[c] as u32 * s[3] as u32, 255));
                        let sp = [sp[0], sp[1], sp[2], s[3] as u32];
                        let sg: [u32; 4] = if global == 0 {
                            sp.map(|v| scale(v * g8, 255))
                        } else {
                            sp.map(|v| (v as f64 * opacity).round() as u32)
                        };
                        let inv = 255 - sg[3];
                        let out = [0, 1, 2, 3].map(|c| (sg[c] + blend(bp[c] * inv, 255)).min(255));
                        score.add_pixel(unpremultiply(out, un), r.0);
                    }
                }
                println!("opacity {gn:>6}, scale {sn:>5}, over {bn:>5}: {score}");
            }
        }
    }
    Ok(())
}

/// Every two-layer blend case against a few families of the W3C model. For each mode, prints the
/// best family, so the ones that already match stand out from the ones that need a closer look.
fn modes(corpus: &Path, refs: &Path, pattern: &str) -> Result<()> {
    use std::collections::BTreeMap;
    let cases = load_cases(corpus, refs, pattern)?;
    // (quantize opacity to a byte, source premultiplied to bytes first, backdrop read back from bytes)
    let families: [(&str, bool, bool); 1] = [("quantized-src", false, true)];
    let mut table: BTreeMap<String, Vec<(String, Score)>> = BTreeMap::new();
    for case in &cases {
        if case.project.manifest.layers.len() != 2 {
            continue;
        }
        let mode = case.record(1).blend_mode();
        let opacity = case.record(1).opacity();
        let (back, src) = (case.layer(0), case.layer(1));
        for (name, alpha8, premul_src) in families {
            let mut score = Score::default();
            for ((b, s), r) in back.pixels().zip(src.pixels()).zip(case.reference.pixels()) {
                // The backdrop is on the canvas as premultiplied bytes.
                let ab8 = b[3] as u32;
                let bp = [0, 1, 2].map(|c| div_round(b[c] as u32 * ab8, 255));
                let ab = ab8 as f64 / 255.0;
                let bs = bp.map(|v| if ab8 == 0 { 0.0 } else { v as f64 / 255.0 / ab });
                let _ = (alpha8, premul_src);
                // Premultiplied source bytes, scaled by opacity and rounded back to bytes.
                let sp = [0, 1, 2].map(|c| div_round(s[c] as u32 * s[3] as u32, 255));
                let sq = [sp[0], sp[1], sp[2], s[3] as u32].map(|v| (v as f64 * opacity).round() as u32);
                let a = sq[3] as f64 / 255.0;
                let ss = [0, 1, 2].map(|c| if sq[3] == 0 { 0.0 } else { sq[c] as f64 / sq[3] as f64 });
                let mut out = crate::blend::composite(mode, [bs[0], bs[1], bs[2], ab], [ss[0], ss[1], ss[2], a]);
                // Non-separable: B on straight bytes in integers, then the general composite.
                let bi = [0, 1, 2].map(|c| if b[3] == 0 { 0 } else { ((bp[c] * 255 + b[3] as u32 / 2) / b[3] as u32).min(255) as i64 });
                let si = [0, 1, 2].map(|c| if sq[3] == 0 { 0 } else { ((sq[c] * 255 + sq[3] / 2) / sq[3]).min(255) as i64 });
                if let Some(m) = crate::blend::non_separable_int(mode, bi, si) {
                    for i in 0..3 {
                        out[i] = a * (1.0 - ab) * ss[i] + a * ab * (m[i] as f64 / 255.0) + (1.0 - a) * ab * bs[i];
                    }
                }
                let q = out.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u32);
                score.add_pixel(unpremultiply(q, div_round), r.0);
            }
            let entry = table.entry(case.id.clone()).or_default();
            entry.push((name.to_string(), score));
        }
    }
    for (key, scores) in table {
        let best = scores.iter().min_by_key(|(_, s)| (s.max, s.wrong)).unwrap();
        println!("{key:<32} best {:<18} {}", best.0, best.1);
    }
    Ok(())
}

/// Prints the pixels where the quantized-source W3C model misses one case, with the inputs.
fn dump(corpus: &Path, refs: &Path, id: &str, limit: usize) -> Result<()> {
    let cases = load_cases(corpus, refs, id)?;
    let case = &cases[0];
    let mode = case.record(1).blend_mode();
    let opacity = case.record(1).opacity();
    let (back, src) = (case.layer(0), case.layer(1));
    let mut shown = 0;
    for ((b, s), r) in back.pixels().zip(src.pixels()).zip(case.reference.pixels()) {
        let bp = [0, 1, 2].map(|c| div_round(b[c] as u32 * b[3] as u32, 255));
        let ab = b[3] as f64 / 255.0;
        let bs = bp.map(|v| if b[3] == 0 { 0.0 } else { v as f64 / b[3] as f64 });
        let sp = [0, 1, 2].map(|c| div_round(s[c] as u32 * s[3] as u32, 255));
        let sq = [sp[0], sp[1], sp[2], s[3] as u32].map(|v| (v as f64 * opacity).round() as u32);
        let ss = [0, 1, 2].map(|c| if sq[3] == 0 { 0.0 } else { sq[c] as f64 / sq[3] as f64 });
        let out = crate::blend::composite(mode, [bs[0], bs[1], bs[2], ab], [ss[0], ss[1], ss[2], sq[3] as f64 / 255.0]);
        let q = out.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u32);
        let got = unpremultiply(q, div_round);
        if got != r.0 && !(got[3] == 0 && r[3] == 0) {
            println!("back {:?} a{:>3} | src {:?} a{:>3} | want {:?} got {:?} | raw {:?}", bp, b[3], [sq[0], sq[1], sq[2]], sq[3], r.0, got, out.map(|v| (v * 255.0 * 1000.0).round() / 1000.0));
            shown += 1;
            if shown >= limit {
                break;
            }
        }
    }
    Ok(())
}

/// Every pixel of a two-layer case as CSV: premultiplied backdrop, opacity-scaled premultiplied
/// source, and the reference (straight), for fitting in a notebook.
fn csv(corpus: &Path, refs: &Path, id: &str) -> Result<()> {
    let cases = load_cases(corpus, refs, id)?;
    let case = &cases[0];
    let opacity = case.record(1).opacity();
    println!("br,bg,bb,ba,sr,sg,sb,sa,rr,rg,rb,ra");
    for ((b, s), r) in case.layer(0).pixels().zip(case.layer(1).pixels()).zip(case.reference.pixels()) {
        let bp = [0, 1, 2].map(|c| div_round(b[c] as u32 * b[3] as u32, 255));
        let sp = [0, 1, 2].map(|c| div_round(s[c] as u32 * s[3] as u32, 255));
        let sq = [sp[0], sp[1], sp[2], s[3] as u32].map(|v| (v as f64 * opacity).round() as u32);
        println!("{},{},{},{},{},{},{},{},{},{},{},{}", bp[0], bp[1], bp[2], b[3], sq[0], sq[1], sq[2], sq[3], r[0], r[1], r[2], r[3]);
    }
    Ok(())
}
