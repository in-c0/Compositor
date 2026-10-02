//! The Shape and Gradient tools (`shapes`): each kind and option through the app's own drag
//! functions, shape layers resized so they redraw, shape layers loaded from a manifest, and probes
//! that put many shapes and gradients through Core Graphics at once.

use super::builder::{CaseWriter, Doc, LayerSpec};
use super::images::{self, Alpha};
use anyhow::Result;
use comp_format::*;
use serde_json::{Value, json};

const F: &str = "shapes";
const N: u32 = 64;

/// Colors that put different bytes through the coverage arithmetic.
const COLORS: [[f64; 3]; 4] = [[0.8, 0.3, 0.55], [1.0, 1.0, 1.0], [0.1, 0.6, 0.9], [0.0, 0.0, 0.0]];

fn spec() -> LayerSpec {
    LayerSpec::default()
}

fn ground(w: &CaseWriter, case: &str, width: u32, height: u32) -> Doc {
    let mut d = w.doc(F, case, width, height);
    d.image("Ground", images::photo(width, height), spec());
    d
}

fn rect(from: [f64; 2], to: [f64; 2], radius: f64, color: [f64; 3]) -> Value {
    json!({ "op": "shape", "kind": "Rectangle", "from": from, "to": to, "cornerRadius": radius, "color": color })
}

fn ellipse(from: [f64; 2], to: [f64; 2], color: [f64; 3]) -> Value {
    json!({ "op": "shape", "kind": "Ellipse", "from": from, "to": to, "color": color })
}

fn line(from: [f64; 2], to: [f64; 2], width: f64, color: [f64; 3]) -> Value {
    json!({ "op": "shape", "kind": "Line", "from": from, "to": to, "lineWidth": width, "color": color })
}

fn style(kind: ShapeKind, color: [f64; 3], radius: f64) -> ShapeStyle {
    ShapeStyle { kind, red: color[0], green: color[1], blue: color[2], corner_radius: radius, line_width: None, start: None, end: None }
}

pub fn shapes(w: &mut CaseWriter) -> Result<()> {
    shape_cases(w)?;
    resize_cases(w)?;
    manifest_cases(w)?;
    gradient_cases(w)?;
    probes(w)?;
    Ok(())
}

fn shape_cases(w: &mut CaseWriter) -> Result<()> {
    let c = COLORS;
    let cases: Vec<(&str, &str, Value)> = vec![
        ("rectangle", "A rectangle", rect([8.0, 8.0], [56.0, 40.0], 0.0, c[0])),
        ("rectangle-rounded", "A rectangle with 8 px corners", rect([8.0, 10.0], [55.0, 47.0], 8.0, c[2])),
        ("rectangle-pill", "A rectangle whose corners make a pill", rect([6.0, 20.0], [58.0, 43.0], 100.0, c[0])),
        ("rectangle-fraction-radius", "A rectangle with 3.5 px corners", rect([9.0, 9.0], [30.0, 50.0], 3.5, c[1])),
        ("rectangle-square", "A rectangle dragged with Shift: a square", json!({ "op": "shape", "kind": "Rectangle", "from": [10, 10], "to": [50, 30], "square": true, "cornerRadius": 5, "color": c[0] })),
        ("ellipse-circle", "A circle 41 px across", ellipse([8.0, 8.0], [49.0, 49.0], c[0])),
        ("ellipse-odd", "An ellipse 53 by 21", ellipse([5.0, 9.0], [58.0, 30.0], c[2])),
        ("ellipse-from-center", "An ellipse dragged with Option, from its center", json!({ "op": "shape", "kind": "Ellipse", "from": [32, 32], "to": [45, 40], "fromCenter": true, "color": c[3] })),
        ("ellipse-tiny", "A circle 3 px across", ellipse([30.0, 30.0], [33.0, 33.0], c[0])),
        ("line-flat", "A level line at the default width", json!({ "op": "shape", "kind": "Line", "from": [6, 32], "to": [58, 32], "color": c[0] })),
        ("line-angled", "A line 3 px wide at an angle", line([5.0, 50.0], [57.0, 11.0], 3.0, c[2])),
        ("line-thick", "A line 12 px wide", line([10.0, 12.0], [50.0, 50.0], 12.0, c[0])),
        ("line-hairline", "A line 1 px wide", line([4.0, 60.0], [60.0, 41.0], 1.0, c[3])),
        ("line-fractional", "A line 2.5 px wide between fractional points", line([7.25, 40.5], [55.75, 20.1], 2.5, c[1])),
        ("line-snapped", "A line dragged with Shift, snapped to 45 degrees", json!({ "op": "shape", "kind": "Line", "from": [10, 50], "to": [52, 20], "square": true, "lineWidth": 5, "color": c[2] })),
        ("line-dot", "A click-length line: a round dot", line([30.0, 30.0], [30.0, 30.0], 9.0, c[0])),
    ];
    for (case, label, op) in cases {
        let d = ground(w, case, N, N);
        w.write(F, case, label, d, vec![op])?;
    }
    // Two shapes, the second drawn above the first, inside a folder with a translucent ground.
    let case = "stacked-in-folder";
    let mut d = w.doc(F, case, N, N);
    d.image("Ground", images::noise(N, N, 71, Alpha::Varied), spec());
    let folder = d.group("Folder", spec());
    d.image("Inside", images::disc(N, N, [30, 200, 90]), LayerSpec { parent: Some(folder), ..spec() });
    w.write(F, case, "Two shapes over a layer in a folder", d, vec![
        rect([4.0, 6.0], [40.0, 30.0], 6.0, c[0]),
        ellipse([20.0, 18.0], [60.0, 58.0], c[2]),
    ])?;
    Ok(())
}

fn resize_cases(w: &mut CaseWriter) -> Result<()> {
    let c = COLORS;
    let resize = |r: [f64; 4]| json!({ "op": "resizeLayer", "rect": r });
    let cases: Vec<(&str, &str, Vec<Value>)> = vec![
        ("resize-rounded-grow", "A rounded rectangle scaled up keeps its 6 px corners",
            vec![rect([10.0, 10.0], [30.0, 26.0], 6.0, c[0]), resize([6.0, 8.0, 50.0, 40.0])]),
        ("resize-rounded-squash", "A rounded rectangle squashed flat keeps its 9 px corners",
            vec![rect([8.0, 8.0], [56.0, 56.0], 9.0, c[2]), resize([4.0, 24.0, 57.0, 15.0])]),
        ("resize-ellipse-shrink", "An ellipse scaled down redraws at its new size",
            vec![ellipse([4.0, 4.0], [60.0, 44.0], c[0]), resize([20.0, 20.0, 17.0, 13.0])]),
        ("resize-line", "A line scaled keeps its width and its ends' places in the box",
            vec![line([10.0, 10.0], [40.0, 30.0], 3.0, c[2]), resize([5.0, 5.0, 55.0, 50.0])]),
        ("resize-move-only", "A shape moved without scaling isn't redrawn",
            vec![rect([10.0, 10.0], [30.0, 26.0], 6.0, c[0]), resize([25.0, 30.0, 20.0, 16.0])]),
        ("resize-named", "The lower of two shapes, found by name, scaled",
            vec![ellipse([4.0, 4.0], [24.0, 24.0], c[0]), rect([30.0, 30.0], [60.0, 60.0], 4.0, c[2]),
                 json!({ "op": "resizeLayer", "layer": "Ellipse 1", "rect": [2.0, 2.0, 41.0, 29.0] })]),
    ];
    for (case, label, ops) in cases {
        let d = ground(w, case, N, N);
        w.write(F, case, label, d, ops)?;
    }
    Ok(())
}

/// Shape layers as a saved project holds them, resized so the app draws them again from their
/// `shape` metadata; the pixels saved with them are deliberately not the shape.
fn manifest_cases(w: &mut CaseWriter) -> Result<()> {
    let c = COLORS;
    let placeholder = |w: u32, h: u32| images::solid(w, h, [200, 40, 40, 255]);
    let mut add = |case: &str, label: &str, size: (u32, u32), at: Transform, style: ShapeStyle, to: [f64; 4]| -> Result<()> {
        let mut d = ground(w, case, N, N);
        let id = d.image("Shape", placeholder(size.0, size.1), LayerSpec { transform: Some(at), ..spec() });
        d.layer(&id).shape = Some(style);
        w.write(F, case, label, d, vec![json!({ "op": "resizeLayer", "layer": id, "rect": to })])
    };
    add("manifest-rounded", "A saved rounded rectangle, resized", (24, 16), Transform::at(10.0, 10.0, 24.0, 16.0),
        style(ShapeKind::Rectangle, c[0], 5.0), [8.0, 8.0, 44.0, 30.0])?;
    add("manifest-scaled-ellipse", "A saved ellipse already stretched to twice its pixels, resized", (20, 10),
        Transform::at(10.0, 10.0, 40.0, 20.0), style(ShapeKind::Ellipse, c[2], 0.0), [12.0, 14.0, 37.0, 23.0])?;
    add("manifest-line", "A saved line with its ends, resized", (30, 20), Transform::at(10.0, 20.0, 30.0, 20.0),
        ShapeStyle { line_width: Some(4.0), start: Some([0.1, 0.9]), end: Some([0.9, 0.1]), ..style(ShapeKind::Line, c[0], 0.0) },
        [6.0, 10.0, 48.0, 33.0])?;
    add("manifest-line-legacy", "A saved line from before lines kept their ends, resized", (30, 20),
        Transform::at(10.0, 20.0, 30.0, 20.0), ShapeStyle { line_width: Some(5.0), ..style(ShapeKind::Line, c[2], 0.0) },
        [6.0, 10.0, 48.0, 33.0])?;
    Ok(())
}

fn gradient_cases(w: &mut CaseWriter) -> Result<()> {
    let fg = [0.9, 0.2, 0.1];
    let bg = [1.0, 0.85, 0.3];
    let g = |from: [f64; 2], to: [f64; 2], extra: Value| {
        let mut op = json!({ "op": "gradient", "from": from, "to": to, "foreground": fg, "background": bg });
        if let Value::Object(extra) = extra {
            for (k, v) in extra {
                op[k] = v;
            }
        }
        op
    };
    let both = json!({ "style": "Foreground to Background" });
    let cases: Vec<(&str, &str, Value)> = vec![
        ("gradient-linear", "Linear, foreground to transparent, left to right", g([8.0, 32.0], [56.0, 32.0], json!({}))),
        ("gradient-linear-colors", "Linear, foreground to background, corner to corner", g([4.0, 60.0], [60.0, 4.0], both.clone())),
        ("gradient-reversed", "Linear, foreground to background, reversed", g([10.0, 5.0], [30.0, 58.0], json!({ "style": "Foreground to Background", "reversed": true }))),
        ("gradient-opacity", "Linear at 45% opacity", g([0.0, 0.0], [64.0, 20.0], json!({ "style": "Foreground to Background", "opacity": 0.45 }))),
        ("gradient-radial", "Radial, foreground to background", g([32.0, 32.0], [52.0, 40.0], json!({ "type": "Radial", "style": "Foreground to Background" }))),
        ("gradient-radial-transparent", "Radial, foreground to transparent at 70%", g([20.5, 40.25], [44.0, 30.0], json!({ "type": "Radial", "opacity": 0.7 }))),
        ("gradient-short", "A gradient only 3 px long, between fractional points", g([30.5, 20.25], [33.75, 22.0], both.clone())),
        ("gradient-past-canvas", "A gradient whose ends lie off the canvas", g([-20.0, 10.0], [90.0, 50.0], both.clone())),
    ];
    for (case, label, op) in cases {
        let d = ground(w, case, N, N);
        w.write(F, case, label, d, vec![op])?;
    }
    // Targets other than an opaque layer covering the canvas.
    let case = "gradient-blank-layer";
    let mut d = ground(w, case, N, N);
    d.blank("Layer 1", N as f64, N as f64, spec());
    w.write(F, case, "Foreground to transparent on a new, empty layer", d, vec![g([16.0, 0.0], [48.0, 64.0], json!({}))])?;
    let case = "gradient-small-layer";
    let mut d = ground(w, case, N, N);
    d.image("Small", images::disc(24, 24, [40, 90, 200]), LayerSpec { transform: Some(Transform::at(20.0, 20.0, 24.0, 24.0)), ..spec() });
    w.write(F, case, "A gradient on a small layer grows it to the canvas", d, vec![g([0.0, 32.0], [64.0, 32.0], both.clone())])?;
    let case = "gradient-translucent";
    let mut d = w.doc(F, case, N, N);
    d.image("Translucent", images::noise(N, N, 72, Alpha::Varied), spec());
    w.write(F, case, "Foreground to transparent over translucent pixels", d, vec![g([60.0, 8.0], [8.0, 50.0], json!({ "opacity": 0.8 }))])?;
    let case = "gradient-mask";
    let mut d = ground(w, case, N, N);
    let id = d.image("Masked", images::checker(N, N, 8), spec());
    d.mask(&id, images::gray_solid(N, N, 255));
    w.write(F, case, "Black to transparent on a layer mask", d, vec![json!({
        "op": "gradient", "layer": id, "mask": true, "from": [8, 8], "to": [56, 40], "foreground": [0, 0, 0] })])?;
    Ok(())
}

/// Many shapes in one case, each on its own layer, so the saved project holds every raster Core
/// Graphics drew; and gradients across wide, short canvases, which show the ramp byte by byte.
fn probes(w: &mut CaseWriter) -> Result<()> {
    let c = |i: usize| COLORS[i % COLORS.len()];
    let rects: [(f64, f64, f64, f64, f64); 13] = [
        (2.0, 2.0, 9.0, 7.0, 0.0), (13.0, 2.0, 9.0, 7.0, 1.0), (24.0, 2.0, 9.0, 7.0, 2.0), (35.0, 2.0, 10.0, 8.0, 2.5),
        (47.0, 2.0, 15.0, 11.0, 3.3), (2.0, 14.0, 20.0, 13.0, 5.0), (24.0, 14.0, 17.0, 17.0, 8.5), (43.0, 14.0, 19.0, 9.0, 50.0),
        (2.0, 30.0, 31.0, 21.0, 7.0), (35.0, 30.0, 27.0, 27.0, 0.5), (2.0, 53.0, 12.0, 9.0, 4.0), (16.0, 53.0, 30.0, 9.0, 4.5),
        (48.0, 53.0, 14.0, 9.0, 1.5),
    ];
    let ops = rects.iter().enumerate().map(|(i, &(x, y, w, h, r))| rect([x, y], [x + w, y + h], r, c(i))).collect();
    w.write(F, "probe-rectangles", "Probe: rectangles with many corner radii", ground(w, "probe-rectangles", N, N), ops)?;
    let small: [(f64, f64, f64, f64); 16] = [
        (1.0, 1.0, 1.0, 1.0), (4.0, 1.0, 2.0, 2.0), (8.0, 1.0, 3.0, 3.0), (13.0, 1.0, 4.0, 4.0), (19.0, 1.0, 5.0, 5.0),
        (26.0, 1.0, 6.0, 6.0), (34.0, 1.0, 7.0, 7.0), (43.0, 1.0, 8.0, 8.0), (53.0, 1.0, 9.0, 9.0), (1.0, 12.0, 10.0, 10.0),
        (13.0, 12.0, 11.0, 11.0), (26.0, 12.0, 13.0, 13.0), (41.0, 12.0, 5.0, 3.0), (48.0, 12.0, 3.0, 9.0), (53.0, 12.0, 10.0, 2.0),
        (1.0, 26.0, 12.0, 7.0),
    ];
    let ops = small.iter().enumerate().map(|(i, &(x, y, w, h))| ellipse([x, y], [x + w, y + h], c(i))).collect();
    w.write(F, "probe-ellipses-small", "Probe: small circles and ellipses", ground(w, "probe-ellipses-small", N, N), ops)?;
    let large: [(f64, f64, f64, f64); 7] = [
        (0.0, 0.0, 16.0, 16.0), (17.0, 0.0, 21.0, 21.0), (39.0, 0.0, 25.0, 10.0), (0.0, 22.0, 32.0, 32.0),
        (33.0, 12.0, 17.0, 40.0), (0.0, 33.0, 63.0, 31.0), (51.0, 11.0, 12.0, 51.0),
    ];
    let ops = large.iter().enumerate().map(|(i, &(x, y, w, h))| ellipse([x, y], [x + w, y + h], c(i))).collect();
    w.write(F, "probe-ellipses-large", "Probe: larger circles and ellipses", ground(w, "probe-ellipses-large", N, N), ops)?;
    // More sizes, for the fill's edge pixels: each ellipse on its own layer, overlapping freely.
    for (case, first) in [("probe-ellipses-more", 2u32), ("probe-ellipses-more-2", 5)] {
        let ops = (0..18u32)
            .map(|i| {
                let (w, h) = (first + (i * 7 + 3) % 23, first + (i * 11 + 5) % 19);
                let (x, y) = ((i * 13) % (N - w) , (i * 17) % (N - h));
                ellipse([x as f64, y as f64], [(x + w) as f64, (y + h) as f64], c(i as usize))
            })
            .collect();
        w.write(F, case, "Probe: ellipses of many sizes", ground(w, case, N, N), ops)?;
    }
    let lines: [([f64; 2], [f64; 2], f64); 14] = [
        ([4.0, 5.0], [40.0, 5.0], 1.0), ([4.0, 9.0], [40.0, 9.0], 2.0), ([4.0, 14.0], [40.0, 14.0], 3.0),
        ([44.0, 4.0], [44.0, 40.0], 1.0), ([50.0, 4.0], [50.0, 40.0], 4.0), ([4.0, 20.0], [30.0, 46.0], 1.0),
        ([8.0, 20.0], [36.0, 44.0], 2.0), ([4.0, 60.0], [60.0, 50.0], 1.5), ([6.0, 52.0], [26.0, 22.0], 3.5),
        ([30.5, 30.5], [58.25, 44.75], 6.0), ([12.3, 40.7], [13.9, 41.2], 4.0), ([56.0, 56.0], [56.0, 56.0], 5.0),
        ([20.0, 30.0], [60.0, 22.0], 7.0), ([3.1, 3.1], [3.1, 30.9], 2.2),
    ];
    let ops = lines.iter().enumerate().map(|(i, &(a, b, width))| line(a, b, width, c(i))).collect();
    w.write(F, "probe-lines", "Probe: lines at many angles and widths", ground(w, "probe-lines", N, N), ops)?;
    // Gradient ramps on an empty layer, one pixel of the ramp per canvas pixel and finer.
    let ramps: [(&str, &str, u32, [f64; 2], [f64; 2], Value); 5] = [
        ("probe-gradient-256", "Probe: black to white over 256 px", 256, [0.0, 2.0], [256.0, 2.0],
            json!({ "foreground": [0, 0, 0], "background": [1, 1, 1], "style": "Foreground to Background" })),
        ("probe-gradient-40", "Probe: a color ramp over 40 px", 64, [12.0, 2.0], [52.0, 2.0],
            json!({ "foreground": [0.9, 0.2, 0.1], "background": [0.1, 0.4, 1], "style": "Foreground to Background" })),
        ("probe-gradient-transparent", "Probe: a color fading out over 256 px", 256, [0.0, 2.0], [256.0, 2.0],
            json!({ "foreground": [1, 0.5, 0.25] })),
        ("probe-gradient-opacity", "Probe: black to white over 256 px at 30%", 256, [0.0, 2.0], [256.0, 2.0],
            json!({ "foreground": [0, 0, 0], "background": [1, 1, 1], "style": "Foreground to Background", "opacity": 0.3 })),
        ("probe-gradient-1000", "Probe: black to white over 1000 px, past the canvas", 256, [-300.0, 2.0], [700.0, 2.0],
            json!({ "foreground": [0, 0, 0], "background": [1, 1, 1], "style": "Foreground to Background" })),
    ];
    for (case, label, width, from, to, extra) in ramps {
        let mut d = w.doc(F, case, width, 4);
        d.blank("Layer 1", width as f64, 4.0, spec());
        let mut op = json!({ "op": "gradient", "from": from, "to": to });
        for (k, v) in extra.as_object().expect("an object") {
            op[k] = v.clone();
        }
        w.write(F, case, label, d, vec![op])?;
    }
    gradient_probes(w)?;
    line_probes(w)?;
    selection_probes(w)
}

/// Core Graphics dithers gradients. Flat "gradients" (both ends one color) whose channels sit a
/// sixteenth apart between two bytes read the dither threshold at every pixel; tall ramps average
/// the dither out to show how the color follows the line.
fn gradient_probes(w: &mut CaseWriter) -> Result<()> {
    let gray = |from: [f64; 2], to: [f64; 2], fg: f64, bg: f64, extra: Value| {
        let mut op = json!({ "op": "gradient", "from": from, "to": to, "foreground": [fg, fg, fg], "background": [bg, bg, bg], "style": "Foreground to Background" });
        if let Value::Object(extra) = extra {
            for (k, v) in extra {
                op[k] = v;
            }
        }
        op
    };
    for k in 0..5 {
        let case = format!("probe-dither-{k}");
        let c: Vec<f64> = (1..=3).map(|i| (100.0 + (3 * k + i) as f64 / 16.0) / 255.0).collect();
        let mut d = w.doc(F, &case, 128, 128);
        d.blank("Layer 1", 128.0, 128.0, spec());
        let op = json!({ "op": "gradient", "from": [0, 0], "to": [128, 0], "foreground": c, "background": c, "style": "Foreground to Background" });
        w.write(F, &case, &format!("Probe: one color, channels {}/16 to {}/16 past a byte", 3 * k + 1, 3 * k + 3), d, vec![op])?;
    }
    // The same, on a layer whose grid starts off the canvas.
    let case = "probe-dither-offset";
    let c: Vec<f64> = (1..=3).map(|i| (100.0 + (3 * i) as f64 / 16.0) / 255.0).collect();
    let mut d = w.doc(F, case, 128, 128);
    d.blank("Layer 1", 200.0, 180.0, LayerSpec { transform: Some(Transform::at(-40.0, -24.0, 200.0, 180.0)), ..spec() });
    let op = json!({ "op": "gradient", "from": [0, 0], "to": [128, 0], "foreground": c, "background": c, "style": "Foreground to Background" });
    w.write(F, case, "Probe: one color on a layer that starts off the canvas", d, vec![op])?;
    let ramps: [(&str, &str, u32, u32, Value); 8] = [
        ("probe-ramp-256", "Probe: black to white over 256 px, 32 rows", 256, 32, gray([0.0, 0.0], [256.0, 0.0], 0.0, 1.0, json!({}))),
        ("probe-ramp-64", "Probe: black to white over 64 px", 64, 32, gray([0.0, 0.0], [64.0, 0.0], 0.0, 1.0, json!({}))),
        ("probe-ramp-16", "Probe: black to white over 16 px", 64, 32, gray([24.0, 0.0], [40.0, 0.0], 0.0, 1.0, json!({}))),
        ("probe-ramp-half", "Probe: black to middle gray over 256 px", 256, 32, gray([0.0, 0.0], [256.0, 0.0], 0.0, 0.5, json!({}))),
        ("probe-ramp-600", "Probe: dark to light gray over 600 px", 256, 32, gray([-100.0, 0.0], [500.0, 0.0], 0.2, 0.9, json!({}))),
        ("probe-ramp-vertical", "Probe: black to white over 128 px, downward", 32, 128, gray([0.0, 0.0], [0.0, 128.0], 0.0, 1.0, json!({}))),
        ("probe-ramp-diagonal", "Probe: black to white along a diagonal", 64, 64, gray([3.0, 5.0], [61.0, 50.0], 0.0, 1.0, json!({}))),
        ("probe-ramp-radial", "Probe: black to white, radial, 40 px", 96, 96, gray([48.0, 48.0], [88.0, 48.0], 0.0, 1.0, json!({ "type": "Radial" }))),
    ];
    for (case, label, width, height, op) in ramps {
        let mut d = w.doc(F, case, width, height);
        d.blank("Layer 1", width as f64, height as f64, spec());
        w.write(F, case, label, d, vec![op])?;
    }
    // How the ramp's pieces depend on its colors and length.
    let colored = |from: [f64; 2], to: [f64; 2], fg: [f64; 3], bg: [f64; 3]| {
        json!({ "op": "gradient", "from": from, "to": to, "foreground": fg, "background": bg, "style": "Foreground to Background" })
    };
    let pieces: [(&str, &str, u32, Value); 9] = [
        ("probe-piece-white-black", "Probe: white to black over 64 px", 64, colored([0.0, 0.0], [64.0, 0.0], [1.0; 3], [0.0; 3])),
        ("probe-piece-grays", "Probe: two grays over 64 px", 64, colored([0.0, 0.0], [64.0, 0.0], [0.2; 3], [0.9; 3])),
        ("probe-piece-black-gray", "Probe: black to 90% gray over 64 px", 64, colored([0.0, 0.0], [64.0, 0.0], [0.0; 3], [0.9; 3])),
        ("probe-piece-red-blue", "Probe: red to blue over 64 px", 64, colored([0.0, 0.0], [64.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0])),
        ("probe-piece-40", "Probe: black to white over 40 px", 64, colored([12.0, 0.0], [52.0, 0.0], [0.0; 3], [1.0; 3])),
        ("probe-piece-128", "Probe: black to white over 128 px", 128, colored([0.0, 0.0], [128.0, 0.0], [0.0; 3], [1.0; 3])),
        ("probe-piece-600", "Probe: black to white over 600 px", 256, colored([-100.0, 0.0], [500.0, 0.0], [0.0; 3], [1.0; 3])),
        ("probe-piece-inside", "Probe: black to white over 96 px inside a wider canvas", 256, colored([80.0, 0.0], [176.0, 0.0], [0.0; 3], [1.0; 3])),
        ("probe-piece-45", "Probe: black to white at 45 degrees", 64, colored([0.0, 0.0], [64.0, 64.0], [0.0; 3], [1.0; 3])),
    ];
    for (case, label, width, op) in pieces {
        let height = if case.ends_with("-45") { 64 } else { 16 };
        let mut d = w.doc(F, case, width, height);
        d.blank("Layer 1", width as f64, height as f64, spec());
        w.write(F, case, label, d, vec![op])?;
    }
    // How many pieces a ramp has, by its length and where its ends fall.
    let spans: [(&str, f64, f64); 12] = [
        ("probe-span-24", 0.0, 24.0), ("probe-span-32", 0.0, 32.0), ("probe-span-48", 0.0, 48.0), ("probe-span-56", 0.0, 56.0),
        ("probe-span-72", 0.0, 72.0), ("probe-span-100", 0.0, 100.0), ("probe-span-160", 0.0, 160.0), ("probe-span-200", 0.0, 200.0),
        ("probe-span-40-at-0", 0.0, 40.0), ("probe-span-40-at-8", 8.0, 48.0), ("probe-span-64-at-4", 4.0, 68.0), ("probe-span-64-at-half", 0.5, 64.5),
    ];
    for (case, from, to) in spans {
        let width = (to.ceil() as u32).max(64).next_multiple_of(16);
        let mut d = w.doc(F, case, width, 16);
        d.blank("Layer 1", width as f64, 16.0, spec());
        let label = format!("Probe: black to white from x = {from} to {to}");
        w.write(F, case, &label, d, vec![colored([from, 0.0], [to, 0.0], [0.0; 3], [1.0; 3])])?;
    }
    // The dither's thresholds to 1/256: one color per layer, each channel a quarter of a 256th
    // past 100 + k/256, for every k that isn't a multiple of 16.
    let case = "probe-dither-fine";
    let mut d = w.doc(F, case, 16, 16);
    let mut ops = Vec::new();
    let ks: Vec<u32> = (1..256).filter(|k| k % 16 != 0).collect();
    for (i, chunk) in ks.chunks(3).enumerate() {
        let id = d.blank(&format!("Layer {}", i + 1), 16.0, 16.0, spec());
        let c: Vec<f64> = chunk.iter().map(|&k| (100.0 + (k as f64 + 0.25) / 256.0) / 255.0).collect();
        ops.push(json!({ "op": "gradient", "layer": id, "from": [0, 0], "to": [16, 0], "foreground": c, "background": c, "style": "Foreground to Background" }));
    }
    w.write(F, case, "Probe: the dither's thresholds, one layer per three of them", d, ops)?;
    // The dither with the gradient's line turned: level, upright, slanted and radial.
    let c: Vec<f64> = [3.0, 8.0, 13.0].iter().map(|f| (100.0 + f / 16.0) / 255.0).collect();
    for (case, label, from, to, radial) in [
        ("probe-dither-upright", "Probe: one color along an upright line", [0.0, 0.0], [0.0, 64.0], false),
        ("probe-dither-slanted", "Probe: one color along a slanted line", [0.0, 0.0], [64.0, 40.0], false),
        ("probe-dither-radial", "Probe: one color, radial", [32.0, 32.0], [64.0, 32.0], true),
    ] {
        let mut d = w.doc(F, case, 64, 64);
        d.blank("Layer 1", 64.0, 64.0, spec());
        let mut op = json!({ "op": "gradient", "from": from, "to": to, "foreground": c, "background": c, "style": "Foreground to Background" });
        if radial {
            op["type"] = json!("Radial");
        }
        w.write(F, case, label, d, vec![op])?;
    }
    // Opacity and transparency on a tall ramp.
    let case = "probe-ramp-opacity";
    let mut d = w.doc(F, case, 256, 32);
    d.blank("Layer 1", 256.0, 32.0, spec());
    w.write(F, case, "Probe: black to white at 50%", d, vec![gray([0.0, 0.0], [256.0, 0.0], 0.0, 1.0, json!({ "opacity": 0.5 }))])?;
    let case = "probe-ramp-transparent";
    let mut d = w.doc(F, case, 256, 32);
    d.blank("Layer 1", 256.0, 32.0, spec());
    w.write(F, case, "Probe: a color fading out over 256 px, 32 rows", d, vec![json!({ "op": "gradient", "from": [0, 0], "to": [256, 0], "foreground": [0.4, 0.6, 0.8] })])?;
    Ok(())
}

/// The same ellipses the shape probes draw, as antialiased Marquee selections on a canvas exactly
/// their size: the Mac fills the same `CGPath(ellipseIn:)` into a gray bitmap, so comparing its
/// coverage with the shape layers' alpha tells a difference in the RGBA fill from one in the path.
fn selection_probes(w: &mut CaseWriter) -> Result<()> {
    for (width, height) in [(2u32, 2u32), (6, 6), (10, 10), (11, 11), (13, 13), (12, 7), (21, 21), (25, 10), (32, 32), (17, 40), (63, 31), (12, 51)] {
        let case = format!("probe-select-ellipse-{width}x{height}");
        let d = ground(w, &case, width, height);
        let op = json!({ "op": "marquee", "shape": "Ellipse", "from": [0, 0], "to": [width, height] });
        w.write(F, &case, &format!("Probe: a {width} by {height} ellipse selected with the Marquee"), d, vec![op])?;
    }
    // The same ellipses as Polygonal Lasso outlines through the points the port flattens them to,
    // in the path's own order, reversed, and starting a quarter round: if the Mac's coverage
    // matches the Marquee's, the flattening is the same and only the fill is left to explain.
    for (width, height) in [(2u32, 2u32), (6, 6), (10, 10), (13, 13), (25, 10), (32, 32)] {
        let (wf, hf) = (width as f64, height as f64);
        let points = engine::select::geom::flatten(&engine::select::geom::ellipse(0.0, 0.0, wf, hf));
        let spaced = (0..points.len()).all(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            (a[0] - b[0]).hypot(a[1] - b[1]) >= 0.25
        });
        assert!(spaced, "the Lasso would drop points of the {width}x{height} ellipse");
        let quarter = points.len() / 4;
        let reversed: Vec<[f64; 2]> = points.iter().rev().copied().collect();
        let rotated: Vec<[f64; 2]> = points[quarter..].iter().chain(&points[..quarter]).copied().collect();
        for (suffix, more, outline) in [("", "", points.clone()), ("-reversed", ", reversed", reversed), ("-rotated", ", starting a quarter round", rotated)] {
            let case = format!("probe-flat-ellipse-{width}x{height}{suffix}");
            let d = ground(w, &case, width, height);
            let op = json!({ "op": "lasso", "kind": "Polygonal", "points": outline });
            w.write(F, &case, &format!("Probe: a {width} by {height} ellipse's flattened points with the Polygonal Lasso{more}"), d, vec![op])?;
        }
    }
    Ok(())
}

/// Lines whose edges run level at fractional heights, thin lines either side of 1 px, and long
/// slanted lines, each on its own layer.
fn line_probes(w: &mut CaseWriter) -> Result<()> {
    let white = [1.0, 1.0, 1.0];
    let level: Vec<Value> = [1.3, 1.7, 2.5, 3.3, 1.01, 1.1].iter().enumerate()
        .map(|(i, &width)| line([4.0, 5.0 + 10.0 * i as f64], [60.0, 5.0 + 10.0 * i as f64], width, white)).collect();
    w.write(F, "probe-lines-level", "Probe: level lines of fractional widths", ground(w, "probe-lines-level", N, N), level)?;
    let thin: Vec<Value> = [(1.0, [4.0, 4.0], [60.0, 20.0]), (1.0, [4.0, 30.0], [30.0, 60.0]), (1.0, [34.0, 34.0], [60.0, 36.0]),
        (1.01, [4.0, 10.0], [60.0, 26.0]), (1.5, [4.0, 20.0], [60.0, 36.0]), (2.0, [4.0, 40.0], [60.0, 56.0]), (1.0, [40.0, 60.0], [60.0, 40.5])]
        .iter().map(|&(width, a, b)| line(a, b, width, white)).collect();
    w.write(F, "probe-lines-thin", "Probe: 1 px lines at angles, and slightly wider", ground(w, "probe-lines-thin", N, N), thin)?;
    let long: Vec<Value> = [(2.0, [2.0, 3.0], [62.0, 7.0]), (2.0, [2.0, 12.0], [62.0, 30.0]), (3.0, [2.0, 60.0], [40.0, 2.0]),
        (4.0, [10.0, 62.0], [62.0, 50.0]), (2.5, [30.0, 40.0], [31.0, 62.0])]
        .iter().map(|&(width, a, b)| line(a, b, width, white)).collect();
    w.write(F, "probe-lines-long", "Probe: long lines at shallow and steep slopes", ground(w, "probe-lines-long", N, N), long)?;
    // Lines fanned out from two centers at slants other than the eighths of a turn, on 1/64 px
    // ends (trigonometry differs in the last bits between platforms).
    let q = |v: f64| (v * 64.0).round() / 64.0;
    for (case, center, widths) in [("probe-lines-fan", [31.5, 32.25], [2.0, 3.0, 4.5, 6.0]), ("probe-lines-fan-thin", [32.3, 31.6], [1.25, 1.5, 2.5, 3.5])] {
        let fan: Vec<Value> = (0..8)
            .map(|i| {
                let angle = (11.0 + 43.0 * i as f64).to_radians();
                let (c, s) = (angle.cos(), angle.sin());
                let a = [q(center[0] + 5.0 * c), q(center[1] + 5.0 * s)];
                let b = [q(center[0] + 28.0 * c), q(center[1] + 28.0 * s)];
                line(a, b, widths[i % 4], white)
            })
            .collect();
        w.write(F, case, "Probe: lines fanned out at slants between the eighths of a turn", ground(w, case, N, N), fan)?;
    }
    Ok(())
}
