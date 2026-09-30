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
    Ok(())
}
