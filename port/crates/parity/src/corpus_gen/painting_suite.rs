//! Painting and retouching: Brush and Eraser on pixels and masks, Clone Stamp, Spot Healing,
//! Blur, Smudge and Liquify, each as `stroke` ops the harness feeds to the app's mouse handlers.

use super::builder::{CaseWriter, LayerSpec};
use super::images::{self, Alpha};
use anyhow::Result;
use comp_format::Transform;
use image::{Rgba, RgbaImage};
use serde_json::{Value, json};

const N: u32 = 64;

fn spec() -> LayerSpec {
    LayerSpec::default()
}

fn at(x: f64, y: f64, w: f64, h: f64) -> LayerSpec {
    LayerSpec { transform: Some(Transform::at(x, y, w, h)), ..spec() }
}

/// A stroke op. `settings` holds only the options-bar fields to change.
fn stroke(tool: &str, layer: &str, points: &[[f64; 2]], settings: Value) -> Value {
    json!({ "op": "stroke", "tool": tool, "layer": layer, "points": points, "settings": settings })
}

fn on_mask(mut op: Value) -> Value {
    op["target"] = json!("mask");
    op
}

/// A gentle S across the canvas, sampled every `step` pixels, as a mouse reports a drag.
fn wave(x0: f64, x1: f64, y: f64, amplitude: f64, step: f64) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    let mut x = x0;
    while x <= x1 + 1e-9 {
        let t = (x - x0) / (x1 - x0);
        // Quarter-pixel positions, so the JSON holds exact binary fractions.
        let yy = ((y + amplitude * (t * std::f64::consts::TAU).sin()) * 4.0).round() / 4.0;
        points.push([x, yy]);
        x += step;
    }
    points
}

/// Points around a circle, as a drawn loop.
fn circle(cx: f64, cy: f64, r: f64, count: usize, turns: f64) -> Vec<[f64; 2]> {
    (0..=count)
        .map(|i| {
            let a = i as f64 / count as f64 * turns * std::f64::consts::TAU;
            [((cx + r * a.cos()) * 4.0).round() / 4.0, ((cy + r * a.sin()) * 4.0).round() / 4.0]
        })
        .collect()
}

/// A photo with a dark blotch and a thin scratch on it: something for Spot Healing to remove.
fn blemished(w: u32, h: u32) -> RgbaImage {
    let mut img = images::checker(w, h, 8);
    let photo = images::photo(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let q = photo.get_pixel(x, y);
        for c in 0..3 {
            p[c] = ((p[c] as u32 + q[c] as u32 * 3) / 4) as u8;
        }
        let d = ((x as f64 - 30.5).powi(2) + (y as f64 - 26.5).powi(2)).sqrt();
        if d < 5.0 || (x == 44 && (36..50).contains(&y)) {
            *p = Rgba([25, 20, 18, 255]);
        }
    }
    img
}

pub fn painting(w: &mut CaseWriter) -> Result<()> {
    const F: &str = "painting";
    let red = json!([0.9, 0.2, 0.1]);
    let blue = json!([0.1, 0.3, 0.9]);

    // Brush on pixels.
    let brush_cases: Vec<(&str, &str, Value, Vec<[f64; 2]>)> = vec![
        ("brush-hard", "Hard 12 px brush", json!({ "size": 12, "hardness": 1, "opacity": 1, "color": red }), wave(6.0, 58.0, 32.0, 14.0, 4.0)),
        ("brush-hard-half", "Hard 12 px brush at 50%", json!({ "size": 12, "hardness": 1, "opacity": 0.5, "color": red }), wave(6.0, 58.0, 32.0, 14.0, 4.0)),
        ("brush-soft", "Soft 24 px brush", json!({ "size": 24, "hardness": 0, "opacity": 1, "color": blue }), wave(8.0, 56.0, 30.0, 10.0, 6.0)),
        ("brush-soft-partial", "30 px brush, 50% hardness, 37% opacity", json!({ "size": 30, "hardness": 0.5, "opacity": 0.37, "color": blue }), wave(8.0, 56.0, 34.0, 12.0, 8.0)),
        ("brush-curve", "Hard 8 px brush around a loop", json!({ "size": 8, "hardness": 1, "opacity": 1, "color": [0.2, 0.8, 0.3] }), circle(32.0, 32.0, 20.0, 18, 1.0)),
        ("brush-self-cross", "Soft 16 px brush crossing itself", json!({ "size": 16, "hardness": 0.25, "opacity": 0.9, "color": [1, 1, 1] }), circle(30.0, 34.0, 14.0, 24, 1.6)),
        ("brush-single-dab", "One click of a soft 20 px brush", json!({ "size": 20, "hardness": 0, "opacity": 1, "color": red }), vec![[31.25, 29.75]]),
        ("brush-tiny-hard", "Hard 1 px brush", json!({ "size": 1, "hardness": 1, "opacity": 1, "color": [0, 0, 0] }), wave(4.0, 60.0, 20.0, 8.0, 3.0)),
        ("brush-tiny-soft", "Soft 3 px brush", json!({ "size": 3, "hardness": 0, "opacity": 1, "color": [1, 1, 0] }), wave(4.0, 60.0, 44.0, 8.0, 3.0)),
        ("brush-off-canvas", "A stroke that leaves the canvas and comes back", json!({ "size": 14, "hardness": 0.75, "opacity": 1, "color": red }), vec![[10.0, 20.0], [-12.0, 30.0], [-6.0, 50.0], [30.0, 70.0], [50.0, 58.0], [75.0, 40.0]]),
        ("brush-smoothing", "Smoothing 10 over a shaky drag", json!({ "size": 10, "hardness": 1, "opacity": 1, "smoothing": 10, "color": blue }), (0..30).map(|i| [6.0 + i as f64 * 1.75, 32.0 + if i % 2 == 0 { 3.0 } else { -3.0 }]).collect()),
    ];
    for (case, label, settings, points) in brush_cases {
        let mut d = w.doc(F, case, N, N);
        let id = d.image("Photo", images::photo(N, N), spec());
        w.write(F, case, label, d, vec![stroke("brush", &id, &points, settings)])?;
    }

    // Brush over translucent pixels: many (backdrop, coverage) pairs.
    let mut d = w.doc(F, "brush-over-alpha", N, N);
    let id = d.image("Noise", images::noise(N, N, 71, Alpha::Varied), spec());
    let settings = json!({ "size": 40, "hardness": 0.25, "opacity": 0.8, "color": [0.3, 0.6, 0.2] });
    w.write(F, "brush-over-alpha", "40 px brush at 80% over translucent noise", d, vec![stroke("brush", &id, &wave(4.0, 60.0, 32.0, 6.0, 8.0), settings)])?;

    // A small layer that the stroke runs past: the layer grows to hold the paint.
    let mut d = w.doc(F, "brush-grow", N, N);
    d.image("Ground", images::photo(N, N), spec());
    let id = d.image("Disc", images::disc(24, 24, [240, 240, 240]), at(20.0, 20.0, 24.0, 24.0));
    let settings = json!({ "size": 10, "hardness": 0.5, "opacity": 1, "color": red });
    w.write(F, "brush-grow", "A stroke past a small layer's edge grows it", d, vec![stroke("brush", &id, &[[4.0, 30.0], [30.0, 34.0], [58.0, 36.0], [70.0, 38.0]], settings)])?;

    // The same on a masked layer: the mask is carried over to the grown layer.
    let mut d = w.doc(F, "brush-grow-masked", N, N);
    d.image("Ground", images::photo(N, N), spec());
    let id = d.image("Disc", images::noise(24, 24, 72, Alpha::Opaque), at(20.0, 20.0, 24.0, 24.0));
    d.mask(&id, images::gray_ramp(24, 24, true));
    let settings = json!({ "size": 12, "hardness": 1, "opacity": 1, "color": blue });
    w.write(F, "brush-grow-masked", "Painting past a masked layer's edge", d, vec![stroke("brush", &id, &[[30.0, 4.0], [32.0, 30.0], [34.0, 60.0]], settings)])?;

    // Two strokes: the second paints over the first's committed tiles.
    let mut d = w.doc(F, "brush-two-strokes", N, N);
    let id = d.image("Photo", images::photo(N, N), spec());
    let ops = vec![
        stroke("brush", &id, &wave(6.0, 58.0, 24.0, 8.0, 5.0), json!({ "size": 16, "hardness": 0, "opacity": 0.6, "color": red })),
        stroke("brush", &id, &[[32.0, 4.0], [30.0, 32.0], [34.0, 60.0]], json!({ "size": 16, "hardness": 0, "opacity": 0.6, "color": blue })),
    ];
    w.write(F, "brush-two-strokes", "Two overlapping soft strokes", d, ops)?;

    // Shift-click: a straight line on from the end of the last stroke.
    let mut d = w.doc(F, "brush-shift-line", N, N);
    let id = d.image("Photo", images::photo(N, N), spec());
    let settings = json!({ "size": 6, "hardness": 1, "opacity": 1, "color": [0, 0, 0] });
    let mut line = stroke("brush", &id, &[[50.0, 52.0]], settings.clone());
    line["shift"] = json!(true);
    let ops = vec![stroke("brush", &id, &[[8.0, 8.0], [20.0, 12.0]], settings), line];
    w.write(F, "brush-shift-line", "A stroke, then a Shift-click line on from its end", d, ops)?;

    // Eraser.
    let erase_cases: Vec<(&str, &str, Value, Alpha)> = vec![
        ("erase-hard", "Hard 14 px eraser", json!({ "size": 14, "hardness": 1, "opacity": 1 }), Alpha::Opaque),
        ("erase-soft-half", "Soft 24 px eraser at 50%", json!({ "size": 24, "hardness": 0, "opacity": 0.5 }), Alpha::Opaque),
        ("erase-over-alpha", "30 px eraser, 50% hardness, over translucent noise", json!({ "size": 30, "hardness": 0.5, "opacity": 0.85 }), Alpha::Varied),
    ];
    for (case, label, settings, alpha) in erase_cases {
        let mut d = w.doc(F, case, N, N);
        d.image("Ground", images::checker(N, N, 8), spec());
        let id = d.image("Noise", images::noise(N, N, 73, alpha), spec());
        w.write(F, case, label, d, vec![stroke("eraser", &id, &wave(6.0, 58.0, 32.0, 12.0, 6.0), settings)])?;
    }

    // Masks: hide, reveal, and a brush that grows the mask past its layer.
    let mut d = w.doc(F, "mask-hide-hard", N, N);
    d.image("Ground", images::checker(N, N, 8), spec());
    let id = d.image("Photo", images::photo(N, N), spec());
    d.mask(&id, images::gray_ramp(N, N, true));
    let op = on_mask(stroke("brush", &id, &wave(6.0, 58.0, 32.0, 14.0, 5.0), json!({ "size": 12, "hardness": 1, "opacity": 1, "white": false })));
    w.write(F, "mask-hide-hard", "Hard black on a mask", d, vec![op])?;

    let mut d = w.doc(F, "mask-reveal-soft", N, N);
    d.image("Ground", images::checker(N, N, 8), spec());
    let id = d.image("Photo", images::photo(N, N), spec());
    d.mask(&id, images::gray_solid(N, N, 0));
    let op = on_mask(stroke("brush", &id, &circle(32.0, 32.0, 16.0, 16, 1.0), json!({ "size": 22, "hardness": 0, "opacity": 0.7, "white": true })));
    w.write(F, "mask-reveal-soft", "Soft white at 70% on a black mask", d, vec![op])?;

    let mut d = w.doc(F, "mask-grow", N, N);
    d.image("Ground", images::checker(N, N, 8), spec());
    let id = d.image("Photo", images::photo(32, 32), at(16.0, 16.0, 32.0, 32.0));
    d.mask(&id, images::gray_ramp(32, 32, false));
    let op = on_mask(stroke("brush", &id, &[[2.0, 20.0], [30.0, 26.0], [62.0, 40.0]], json!({ "size": 10, "hardness": 0.5, "opacity": 1, "white": false })));
    w.write(F, "mask-grow", "Black on a mask, past its layer's edge", d, vec![op])?;

    // Gray arithmetic: white and black at several opacities over every mask value.
    let probes: [(&str, &str, bool, f64, f64); 4] = [
        ("mask-white-half", "Soft white at 50% over a ramp mask", true, 0.5, 0.0),
        ("mask-black-37", "Soft black at 37% over a ramp mask", false, 0.37, 0.0),
        ("mask-white-full", "Soft white at 100% over a ramp mask", true, 1.0, 0.0),
        ("mask-white-hard-60", "Hard white at 60% over a ramp mask", true, 0.6, 1.0),
    ];
    for (case, label, white, opacity, hardness) in probes {
        let mut d = w.doc(F, case, N, N);
        d.image("Ground", images::checker(N, N, 8), spec());
        let id = d.image("Photo", images::photo(N, N), spec());
        d.mask(&id, images::gray_ramp(N, N, true));
        let settings = json!({ "size": 36, "hardness": hardness, "opacity": opacity, "white": white });
        let op = on_mask(stroke("brush", &id, &[[2.0, 12.0], [62.0, 16.0], [60.0, 44.0], [4.0, 50.0]], settings));
        w.write(F, case, label, d, vec![op])?;
    }

    // Clone Stamp.
    let clone = |w: &mut CaseWriter, case: &str, label: &str, ops: &dyn Fn(&str) -> Vec<Value>| -> Result<()> {
        let mut d = w.doc(F, case, N, N);
        let id = d.image("Pattern", images::checker(N, N, 6), spec());
        let ops = ops(&id);
        w.write(F, case, label, d, ops)
    };
    clone(w, "clone-hard", "Clone Stamp, hard 12 px, from 20 px away", &|id| {
        let mut op = stroke("clone", id, &[[14.0, 40.0], [30.0, 44.0], [48.0, 42.0]], json!({ "size": 12, "hardness": 1, "opacity": 1 }));
        op["source"] = json!([34.0, 20.0]);
        vec![op]
    })?;
    clone(w, "clone-soft", "Clone Stamp, soft 20 px at 60%", &|id| {
        let mut op = stroke("clone", id, &wave(10.0, 54.0, 40.0, 6.0, 6.0), json!({ "size": 20, "hardness": 0, "opacity": 0.6 }));
        op["source"] = json!([20.5, 12.25]);
        vec![op]
    })?;
    clone(w, "clone-past-edge", "Clone Stamp copying from past the layer's edge", &|id| {
        let mut op = stroke("clone", id, &[[40.0, 10.0], [44.0, 30.0], [40.0, 54.0]], json!({ "size": 16, "hardness": 0.5, "opacity": 1 }));
        op["source"] = json!([4.0, 40.0]);
        vec![op]
    })?;
    clone(w, "clone-aligned", "Two aligned Clone Stamp strokes keep one offset", &|id| {
        let settings = json!({ "size": 10, "hardness": 0.75, "opacity": 1, "aligned": true });
        let mut first = stroke("clone", id, &[[10.0, 40.0], [24.0, 42.0]], settings.clone());
        first["source"] = json!([12.0, 12.0]);
        vec![first, stroke("clone", id, &[[40.0, 50.0], [54.0, 46.0]], settings)]
    })?;
    clone(w, "clone-unaligned", "Two Clone Stamp strokes, not aligned, each from the source", &|id| {
        let settings = json!({ "size": 10, "hardness": 0.75, "opacity": 1, "aligned": false });
        let mut first = stroke("clone", id, &[[10.0, 40.0], [24.0, 42.0]], settings.clone());
        first["source"] = json!([12.0, 12.0]);
        vec![first, stroke("clone", id, &[[40.0, 50.0], [54.0, 46.0]], settings)]
    })?;
    let mut d = w.doc(F, "clone-sample-all", N, N);
    d.image("Ground", images::photo(N, N), spec());
    d.image("Veil", images::noise(N, N, 74, Alpha::Varied), LayerSpec { opacity: Some(0.5), ..spec() });
    let id = d.image("Target", images::disc(40, 40, [60, 140, 230]), at(12.0, 12.0, 40.0, 40.0));
    let mut op = stroke("clone", &id, &[[8.0, 50.0], [32.0, 54.0], [56.0, 50.0]], json!({ "size": 14, "hardness": 0.5, "opacity": 1, "sampleAll": true }));
    op["source"] = json!([32.0, 12.0]);
    w.write(F, "clone-sample-all", "Clone Stamp sampling all layers onto a small layer", d, vec![op])?;

    // Spot Healing.
    let heal_cases: Vec<(&str, &str, Value)> = vec![
        ("heal-content-aware", "Spot Healing, Content-Aware, over a blotch", json!({ "size": 14, "hardness": 1, "opacity": 1, "healingMode": "Content-Aware" })),
        ("heal-proximity", "Spot Healing, Proximity Match, over a blotch", json!({ "size": 14, "hardness": 1, "opacity": 1, "healingMode": "Proximity Match" })),
        ("heal-soft", "Spot Healing with a soft tip at 80%", json!({ "size": 16, "hardness": 0.3, "opacity": 0.8, "healingMode": "Content-Aware" })),
    ];
    for (case, label, settings) in heal_cases {
        let mut d = w.doc(F, case, N, N);
        let id = d.image("Photo", blemished(N, N), spec());
        w.write(F, case, label, d, vec![stroke("heal", &id, &[[28.0, 25.0], [32.0, 27.0]], settings)])?;
    }
    let mut d = w.doc(F, "heal-scratch", N, N);
    let id = d.image("Photo", blemished(N, N), spec());
    let settings = json!({ "size": 6, "hardness": 1, "opacity": 1, "healingMode": "Content-Aware" });
    w.write(F, "heal-scratch", "Spot Healing along a thin scratch", d, vec![stroke("heal", &id, &[[44.5, 35.0], [44.5, 42.0], [44.5, 50.5]], settings)])?;

    // Blur.
    let mut d = w.doc(F, "blur-pixels", N, N);
    let id = d.image("Pattern", images::checker(N, N, 4), spec());
    let settings = json!({ "size": 24, "hardness": 0, "opacity": 1, "blurRadius": 3 });
    w.write(F, "blur-pixels", "Blur tool, radius 3, soft 24 px", d, vec![stroke("blur", &id, &wave(6.0, 58.0, 32.0, 10.0, 6.0), settings)])?;
    let mut d = w.doc(F, "blur-strength", N, N);
    let id = d.image("Noise", images::noise(N, N, 75, Alpha::Varied), spec());
    let settings = json!({ "size": 30, "hardness": 0.5, "opacity": 0.5, "blurRadius": 8 });
    w.write(F, "blur-strength", "Blur tool, radius 8, strength 50%, over translucent noise", d, vec![stroke("blur", &id, &[[10.0, 20.0], [50.0, 44.0]], settings)])?;
    let mut d = w.doc(F, "blur-mask", N, N);
    d.image("Ground", images::checker(N, N, 8), spec());
    let id = d.image("Photo", images::photo(N, N), spec());
    d.mask(&id, images::mask_mixed(N, N));
    let op = on_mask(stroke("blur", &id, &[[4.0, 32.0], [60.0, 32.0]], json!({ "size": 28, "hardness": 1, "opacity": 1, "blurRadius": 4 })));
    w.write(F, "blur-mask", "Blur tool on a mask", d, vec![op])?;

    // Smudge and Liquify.
    let mut d = w.doc(F, "smudge", N, N);
    let id = d.image("Pattern", images::checker(N, N, 8), spec());
    let settings = json!({ "size": 16, "hardness": 0, "opacity": 0.8 });
    w.write(F, "smudge", "Smudge, 16 px at 80%", d, vec![stroke("smudge", &id, &[[12.0, 20.0], [30.0, 28.0], [52.0, 30.0]], settings)])?;
    let mut d = w.doc(F, "liquify", N, N);
    let id = d.image("Pattern", images::checker(N, N, 8), spec());
    let settings = json!({ "size": 20, "hardness": 0.5, "opacity": 1 });
    w.write(F, "liquify", "Liquify, 20 px", d, vec![stroke("liquify", &id, &[[16.0, 32.0], [28.0, 34.0], [44.0, 30.0]], settings)])?;
    Ok(())
}
