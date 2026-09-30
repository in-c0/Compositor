use crate::{Score, load_cases};
use anyhow::{Result, bail};
use std::path::Path;

pub fn run(name: &str, corpus: &Path, refs: &Path, rest: &[String]) -> Result<()> {
    match name {
        "premultiply" => premultiply(corpus, refs),
        "normal" => normal(corpus, refs, rest.first().map(String::as_str).unwrap_or("blend/normal-*")),
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
