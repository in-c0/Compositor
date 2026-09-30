//! The corpus, one function per feature. Case folders are `<feature>/<case>`, where the feature
//! matches a `key` in `parity/features.toml`.

use super::builder::{CaseWriter, LayerSpec};
use super::images::{self, Alpha};
use anyhow::Result;
use comp_format::*;
use serde_json::json;

pub fn all(w: &mut CaseWriter) -> Result<()> {
    blend(w)?;
    masks(w)?;
    clipping(w)?;
    folders(w)?;
    transform(w)?;
    adjust(w)?;
    effects(w)?;
    document(w)?;
    filters(w)?;
    export(w)?;
    Ok(())
}

fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    s.trim_matches('-').to_string()
}

fn spec() -> LayerSpec {
    LayerSpec::default()
}

const N: u32 = 64;

fn blend(w: &mut CaseWriter) -> Result<()> {
    let variants: [(&str, Alpha, f64); 5] = [
        ("opaque-100", Alpha::Opaque, 1.0),
        ("opaque-60", Alpha::Opaque, 0.6),
        ("opaque-25", Alpha::Opaque, 0.25),
        ("alpha-100", Alpha::Varied, 1.0),
        ("alpha-60", Alpha::Varied, 0.6),
    ];
    for &mode in BlendMode::ALL {
        for (variant, alpha, opacity) in variants {
            let case = format!("{}-{variant}", slug(mode.name()));
            let mut d = w.doc("blend", &case, N, N);
            d.image("Backdrop", images::noise(N, N, 1, alpha), spec());
            d.image("Source", images::noise(N, N, 2, alpha), LayerSpec { opacity: Some(opacity), blend: Some(mode), ..spec() });
            let label = format!("{} at {}% over a {} backdrop", mode.name(), (opacity * 100.0) as u32, if matches!(alpha, Alpha::Opaque) { "opaque" } else { "translucent" });
            w.write("blend", &case, &label, d, vec![])?;
        }
    }
    for opacity in [0.0, 0.004, 0.333, 0.999] {
        let case = format!("normal-opacity-{}", (opacity * 1000.0) as u32);
        let mut d = w.doc("blend", &case, N, N);
        d.image("Backdrop", images::photo(N, N), spec());
        d.image("Source", images::noise(N, N, 3, Alpha::Varied), LayerSpec { opacity: Some(opacity), ..spec() });
        w.write("blend", &case, &format!("Normal at opacity {opacity}"), d, vec![])?;
    }
    let mut d = w.doc("blend", "stack", N, N);
    d.image("Photo", images::photo(N, N), spec());
    d.image("Screen", images::noise(N, N, 4, Alpha::Varied), LayerSpec { blend: Some(BlendMode::Screen), opacity: Some(0.7), ..spec() });
    d.image("Color", images::noise(N, N, 5, Alpha::Opaque), LayerSpec { blend: Some(BlendMode::Color), opacity: Some(0.5), ..spec() });
    d.image("Hard Mix", images::noise(N, N, 6, Alpha::Varied), LayerSpec { blend: Some(BlendMode::HardMix), opacity: Some(0.3), ..spec() });
    d.image("Hidden", images::solid(N, N, [255, 0, 0, 255]), LayerSpec { visible: Some(false), ..spec() });
    w.write("blend", "stack", "Several modes stacked, with a hidden layer on top", d, vec![])?;
    let mut d = w.doc("blend", "empty-canvas", N, N);
    d.image("Only", images::noise(N, N, 7, Alpha::Varied), LayerSpec { blend: Some(BlendMode::Multiply), ..spec() });
    w.write("blend", "empty-canvas", "Multiply over nothing", d, vec![])?;
    Ok(())
}

fn masks(w: &mut CaseWriter) -> Result<()> {
    let cases: Vec<(&str, &str)> = vec![
        ("ramp", "A horizontal gradient mask"),
        ("mixed", "Hard black, a ramp, and hard white"),
        ("disabled", "A disabled mask has no effect"),
        ("one-pixel", "A uniform 1x1 mask at 50%"),
        ("low-res", "A 16x16 mask stretched over a 64x64 layer"),
        ("scaled-layer", "A masked layer placed at half size"),
        ("rotated-layer", "A masked layer rotated 30 degrees"),
        ("unlinked", "An unlinked mask moved apart from its layer"),
        ("with-opacity-blend", "A mask with 60% opacity and Multiply"),
        ("translucent-layer", "A mask over a layer with its own varied alpha"),
    ];
    for (case, label) in cases {
        let mut d = w.doc("masks", case, N, N);
        d.image("Backdrop", images::photo(N, N), spec());
        let mut s = spec();
        match case {
            "scaled-layer" => s.transform = Some(Transform::at(16.0, 8.0, 32.0, 32.0)),
            "rotated-layer" => s.transform = Some(Transform { rotation: 30.0, ..Transform::at(8.0, 8.0, 48.0, 48.0) }),
            "with-opacity-blend" => {
                s.opacity = Some(0.6);
                s.blend = Some(BlendMode::Multiply);
            }
            _ => {}
        }
        let alpha = if case == "translucent-layer" { Alpha::Varied } else { Alpha::Opaque };
        let id = d.image("Masked", images::noise(N, N, 11, alpha), s);
        match case {
            "mixed" => d.mask(&id, images::mask_mixed(N, N)),
            "one-pixel" => d.mask(&id, images::gray_solid(1, 1, 128)),
            "low-res" => d.mask(&id, images::gray_ramp(16, 16, false)),
            _ => d.mask(&id, images::gray_ramp(N, N, true)),
        }
        match case {
            "disabled" => d.layer(&id).mask_enabled = Some(false),
            "unlinked" => {
                let l = d.layer(&id);
                l.mask_linked = Some(false);
                l.mask_placement = Some(Transform::at(12.0, -6.0, 48.0, 64.0));
            }
            _ => {}
        }
        w.write("masks", case, label, d, vec![])?;
    }
    let mut d = w.doc("masks", "folder-mask", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let folder = d.group("Folder", spec());
    d.mask(&folder, images::gray_ramp(N, N, false));
    let inner = d.image("Inner", images::noise(N, N, 12, Alpha::Opaque), LayerSpec { parent: Some(folder.clone()), ..spec() });
    d.mask(&inner, images::gray_ramp(N, N, true));
    d.image("Inner 2", images::disc(N, N, [250, 200, 20]), LayerSpec { parent: Some(folder), blend: Some(BlendMode::Screen), ..spec() });
    w.write("masks", "folder-mask", "A folder mask combined with a child's own mask", d, vec![])?;
    Ok(())
}

fn clipping(w: &mut CaseWriter) -> Result<()> {
    let cases: Vec<(&str, &str)> = vec![
        ("normal", "A photo clipped to a disc"),
        ("multiply", "A clipped layer in Multiply"),
        ("stack", "Two layers clipped to one base"),
        ("adjustment", "An Invert adjustment clipped to a base"),
        ("base-opacity", "A base at 50% opacity"),
        ("base-screen", "A base in Screen mode"),
        ("clipped-mask", "A clipped layer with its own mask"),
        ("base-mask", "A base with a mask"),
        ("base-hidden", "A hidden base"),
        ("lone", "A clipped layer separated from its base by another layer"),
        ("translucent", "A clipped layer with varied alpha at 70%"),
    ];
    for (case, label) in cases {
        let mut d = w.doc("clipping", case, N, N);
        d.image("Backdrop", images::noise(N, N, 21, Alpha::Opaque), spec());
        let mut base_spec = spec();
        match case {
            "base-opacity" => base_spec.opacity = Some(0.5),
            "base-screen" => base_spec.blend = Some(BlendMode::Screen),
            "base-hidden" => base_spec.visible = Some(false),
            _ => {}
        }
        let base = d.image("Base", images::disc(N, N, [240, 240, 240]), base_spec);
        if case == "base-mask" {
            d.mask(&base, images::gray_ramp(N, N, true));
        }
        if case == "lone" {
            d.image("Between", images::corners(N, N, [20, 200, 90]), spec());
        }
        let clipped = if case == "adjustment" {
            d.adjustment("Invert", Adjustment::new(AdjustmentKind::Invert), spec())
        } else {
            let mut s = spec();
            if case == "multiply" {
                s.blend = Some(BlendMode::Multiply);
            }
            if case == "translucent" {
                s.opacity = Some(0.7);
            }
            let alpha = if case == "translucent" { Alpha::Varied } else { Alpha::Opaque };
            let pixels = if case == "translucent" { images::noise(N, N, 22, alpha) } else { images::photo(N, N) };
            d.image("Clipped", pixels, s)
        };
        d.layer(&clipped).mask_source_id = Some(base.clone());
        if case == "clipped-mask" {
            d.mask(&clipped, images::mask_mixed(N, N));
        }
        if case == "stack" {
            let second = d.image("Clipped 2", images::noise(N, N, 23, Alpha::Varied), LayerSpec { blend: Some(BlendMode::Overlay), ..spec() });
            d.layer(&second).mask_source_id = Some(base.clone());
        }
        w.write("clipping", case, label, d, vec![])?;
    }
    Ok(())
}

fn folders(w: &mut CaseWriter) -> Result<()> {
    let mut d = w.doc("folders", "opacity", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let f = d.group("Folder", LayerSpec { opacity: Some(0.5), ..spec() });
    d.image("A", images::noise(N, N, 31, Alpha::Opaque), LayerSpec { parent: Some(f.clone()), ..spec() });
    d.image("B", images::disc(N, N, [20, 40, 250]), LayerSpec { parent: Some(f), blend: Some(BlendMode::Multiply), opacity: Some(0.8), ..spec() });
    w.write("folders", "opacity", "A folder at 50% multiplies into each child", d, vec![])?;

    let mut d = w.doc("folders", "nested", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let outer = d.group("Outer", LayerSpec { opacity: Some(0.7), ..spec() });
    let inner = d.group("Inner", LayerSpec { opacity: Some(0.6), parent: Some(outer.clone()), ..spec() });
    d.image("Deep", images::noise(N, N, 32, Alpha::Varied), LayerSpec { parent: Some(inner), blend: Some(BlendMode::Overlay), ..spec() });
    d.image("Shallow", images::disc(N, N, [250, 30, 30]), LayerSpec { parent: Some(outer), ..spec() });
    w.write("folders", "nested", "Nested folders at 70% and 60%", d, vec![])?;

    // Probes that take "nested" apart, one step each.
    let mut d = w.doc("folders", "probe-deep-direct", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    d.image("Deep", images::noise(N, N, 32, Alpha::Varied), LayerSpec { blend: Some(BlendMode::Overlay), opacity: Some(0.7 * 0.6), ..spec() });
    w.write("folders", "probe-deep-direct", "The nested case's Overlay layer at 0.42 with no folders", d, vec![])?;

    let mut d = w.doc("folders", "probe-deep-nested", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let outer = d.group("Outer", LayerSpec { opacity: Some(0.7), ..spec() });
    let inner = d.group("Inner", LayerSpec { opacity: Some(0.6), parent: Some(outer), ..spec() });
    d.image("Deep", images::noise(N, N, 32, Alpha::Varied), LayerSpec { parent: Some(inner), blend: Some(BlendMode::Overlay), ..spec() });
    w.write("folders", "probe-deep-nested", "The nested case's Overlay layer inside both folders, alone", d, vec![])?;

    let mut d = w.doc("folders", "probe-shallow", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let outer = d.group("Outer", LayerSpec { opacity: Some(0.7), ..spec() });
    d.image("Shallow", images::disc(N, N, [250, 30, 30]), LayerSpec { parent: Some(outer), ..spec() });
    w.write("folders", "probe-shallow", "The nested case's disc inside a 70% folder, alone", d, vec![])?;

    let mut d = w.doc("folders", "hidden", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let f = d.group("Hidden folder", LayerSpec { visible: Some(false), ..spec() });
    d.image("Inside", images::solid(N, N, [255, 0, 0, 255]), LayerSpec { parent: Some(f), ..spec() });
    w.write("folders", "hidden", "A hidden folder hides its visible children", d, vec![])?;

    let mut d = w.doc("folders", "adjustment-inside", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let f = d.group("Folder", spec());
    d.image("Inside", images::disc(N, N, [30, 200, 60]), LayerSpec { parent: Some(f.clone()), ..spec() });
    d.adjustment("Invert", Adjustment::new(AdjustmentKind::Invert), LayerSpec { parent: Some(f), ..spec() });
    w.write("folders", "adjustment-inside", "An adjustment inside a pass-through folder", d, vec![])?;

    let mut d = w.doc("folders", "empty", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    d.group("Empty", LayerSpec { opacity: Some(0.3), ..spec() });
    w.write("folders", "empty", "An empty folder draws nothing", d, vec![])?;
    Ok(())
}

fn transform(w: &mut CaseWriter) -> Result<()> {
    let samplings = [(Sampling::Nearest, "nearest"), (Sampling::Smooth, "smooth"), (Sampling::High, "high")];
    let add = |w: &mut CaseWriter, case: String, label: String, src: RgbaImage, t: Transform| -> Result<()> {
        let mut d = w.doc("transform", &case, N, N);
        d.image("Backdrop", images::solid(N, N, [128, 128, 128, 255]), spec());
        d.image("Moved", src, LayerSpec { transform: Some(t), ..spec() });
        w.write("transform", &case, &label, d, vec![])
    };
    for (s, name) in samplings {
        add(w, format!("upscale-2x-{name}"), format!("16x16 drawn at 32x32, {name}"), images::checker(16, 16, 2), Transform { sampling: s, ..Transform::at(16.0, 16.0, 32.0, 32.0) })?;
        add(w, format!("upscale-3.7x-{name}"), format!("12x12 drawn at 44.4, {name}"), images::noise(12, 12, 41, Alpha::Varied), Transform { sampling: s, ..Transform::at(10.0, 10.0, 44.4, 44.4) })?;
        add(w, format!("downscale-half-{name}"), format!("128x128 drawn at 64x64, {name}"), images::checker(128, 128, 3), Transform { sampling: s, ..Transform::at(0.0, 0.0, 64.0, 64.0) })?;
        add(w, format!("downscale-0.37-{name}"), format!("128x128 drawn at 47.4, {name}"), images::noise(128, 128, 42, Alpha::Opaque), Transform { sampling: s, ..Transform::at(8.0, 8.0, 47.4, 47.4) })?;
        add(w, format!("rotate-30-{name}"), format!("Rotated 30 degrees, {name}"), images::checker(40, 40, 4), Transform { sampling: s, rotation: 30.0, ..Transform::at(12.0, 12.0, 40.0, 40.0) })?;
    }
    for angle in [45.0, 90.0, 180.0, -15.0, 359.0] {
        add(w, format!("rotate-{angle}"), format!("Rotated {angle} degrees"), images::checker(40, 24, 4), Transform { rotation: angle, ..Transform::at(12.0, 20.0, 40.0, 24.0) })?;
    }
    for (fx, fy, name) in [(true, false, "flip-x"), (false, true, "flip-y"), (true, true, "flip-xy")] {
        add(w, name.into(), format!("Flipped: {name}"), images::corners(48, 48, [200, 40, 40]), Transform { flip_x: fx, flip_y: fy, ..Transform::at(8.0, 8.0, 48.0, 48.0) })?;
    }
    add(w, "flip-rotate".into(), "Flipped horizontally and rotated 20 degrees".into(), images::corners(48, 48, [200, 40, 40]), Transform { flip_x: true, rotation: 20.0, ..Transform::at(8.0, 8.0, 48.0, 48.0) })?;
    add(w, "fractional-origin".into(), "Placed at a fractional position".into(), images::checker(32, 32, 4), Transform::at(10.5, 7.25, 32.0, 32.0))?;
    add(w, "off-canvas".into(), "Partly outside the canvas".into(), images::noise(48, 48, 43, Alpha::Opaque), Transform::at(-20.0, 40.0, 48.0, 48.0))?;
    add(w, "non-uniform".into(), "Stretched to 60x20".into(), images::checker(32, 32, 4), Transform::at(2.0, 22.0, 60.0, 20.0))?;
    transform_probes(w)
}

/// Probes of Core Graphics' resampling, on a transparent canvas so the output holds the drawn
/// pixels (and edge coverage) directly: Low interpolation at scales whose sample phases avoid its
/// phase-rounding ties, edge antialiasing at several angles and fractional positions, vImage's
/// Lanczos halvings, and High interpolation resampling a mask (through an unlinked mask).
fn transform_probes(w: &mut CaseWriter) -> Result<()> {
    let smooth = |x: f64, y: f64, sw: f64, sh: f64, rotation: f64| Transform { sampling: Sampling::Smooth, rotation, ..Transform::at(x, y, sw, sh) };
    let gray_noise = |w: u32, h: u32, seed: u32| GrayImage::from_fn(w, h, |x, y| image::Luma([(images::hash(seed.wrapping_mul(0x9e37_79b9) ^ (y * w + x)) >> 11) as u8]));
    let white = |w: u32, h: u32| images::solid(w, h, [255, 255, 255, 255]);
    let layer = |w: &mut CaseWriter, case: &str, label: &str, src: RgbaImage, t: Transform| -> Result<()> {
        let mut d = w.doc("transform", case, N, N);
        d.image("Probe", src, LayerSpec { transform: Some(t), ..spec() });
        w.write("transform", case, label, d, vec![])
    };
    let gray = gray_noise(37, 37, 0x51);
    let gray_rgba = RgbaImage::from_fn(37, 37, |x, y| {
        let v = gray.get_pixel(x, y)[0];
        image::Rgba([v, v, v, 255])
    });
    // 37 source pixels over 64 put every sample phase at an odd multiple of 1/128.
    layer(w, "probe-low-h", "Low, horizontal only: 37x64 at 64x64", images::noise(37, 64, 60, Alpha::Opaque), smooth(0.0, 0.0, 64.0, 64.0, 0.0))?;
    layer(w, "probe-low-2d", "Low: 37x37 at 64x64", images::noise(37, 37, 61, Alpha::Opaque), smooth(0.0, 0.0, 64.0, 64.0, 0.0))?;
    layer(w, "probe-low-gray", "Low on gray pixels: 37x37 at 64x64", gray_rgba, smooth(0.0, 0.0, 64.0, 64.0, 0.0))?;
    layer(w, "probe-low-alpha", "Low on translucent pixels: 37x37 at 64x64", images::noise(37, 37, 62, Alpha::Varied), smooth(0.0, 0.0, 64.0, 64.0, 0.0))?;
    layer(w, "probe-low-2x", "Low: 32x32 at 64x64", images::noise(32, 32, 63, Alpha::Opaque), smooth(0.0, 0.0, 64.0, 64.0, 0.0))?;
    layer(w, "probe-low-shrink", "Low shrinking: 64x64 at 47x47", images::noise(64, 64, 64, Alpha::Varied), smooth(8.0, 8.0, 47.0, 47.0, 0.0))?;
    for angle in [7.0, 30.0, 45.0, 70.0] {
        layer(w, &format!("probe-edge-rotate-{angle}"), &format!("A white square rotated {angle} degrees"), white(32, 32), smooth(16.0, 16.0, 32.0, 32.0, angle))?;
    }
    layer(w, "probe-edge-fraction", "A white square at a fractional position", white(20, 20), smooth(10.3, 7.7, 20.0, 20.0, 0.0))?;
    layer(w, "probe-edge-scaled", "A white square scaled to a fractional size", white(16, 16), smooth(5.2, 9.9, 30.6, 30.6, 0.0))?;
    layer(w, "probe-edge-scaled-rotate", "A white square scaled 2x and rotated 30 degrees", white(16, 16), smooth(16.0, 16.0, 32.0, 32.0, 30.0))?;
    layer(w, "probe-halve-2", "Two Lanczos halvings: 64x64 at 16x16", images::noise(64, 64, 65, Alpha::Varied), smooth(8.0, 8.0, 16.0, 16.0, 0.0))?;
    let impulses = RgbaImage::from_fn(64, 64, |x, y| {
        let v = if x % 16 == 5 + y / 16 && y % 16 == 7 + x / 16 { 255 } else { 0 };
        image::Rgba([v, v, v, 255])
    });
    layer(w, "probe-halve-impulse", "Two Lanczos halvings of single white pixels", impulses, smooth(8.0, 8.0, 16.0, 16.0, 0.0))?;
    layer(w, "probe-halve-odd", "Two halvings of an odd size: 62x62 at 15.5x15.5", images::noise(62, 62, 66, Alpha::Varied), smooth(3.0, 3.0, 15.5, 15.5, 0.0))?;
    layer(w, "probe-halve-3", "Three Lanczos halvings: 128x128 at 16x16", images::noise(128, 128, 67, Alpha::Opaque), smooth(8.0, 8.0, 16.0, 16.0, 0.0))?;
    // A white layer through an unlinked mask shows the mask as placed, resampled with High.
    let masked = |w: &mut CaseWriter, case: &str, label: &str, mask: GrayImage, placement: Transform| -> Result<()> {
        let mut d = w.doc("transform", case, N, N);
        let id = d.image("Probe", white(N, N), spec());
        d.mask(&id, mask);
        let l = d.layer(&id);
        l.mask_linked = Some(false);
        l.mask_placement = Some(placement);
        w.write("transform", case, label, d, vec![])
    };
    masked(w, "probe-high-shrink", "High shrinking a mask: 64x64 at 48x48", gray_noise(64, 64, 70), Transform::at(8.0, 8.0, 48.0, 48.0))?;
    masked(w, "probe-high-shrink-v", "High shrinking a mask vertically: 64x64 at 64x40", gray_noise(64, 64, 71), Transform::at(0.0, 12.0, 64.0, 40.0))?;
    let dots = GrayImage::from_fn(64, 64, |x, y| image::Luma([if x % 8 == 3 && y % 8 == 4 { 255 } else { 0 }]));
    masked(w, "probe-high-impulse", "High shrinking single white mask pixels: 64x64 at 40x40", dots, Transform::at(4.0, 4.0, 40.0, 40.0))?;
    masked(w, "probe-high-mixed", "High stretching a mask one way and shrinking it the other", gray_noise(32, 32, 72), Transform::at(2.0, 22.0, 60.0, 20.0))?;
    masked(w, "probe-high-grow", "High enlarging a mask: 16x16 at 37x37", gray_noise(16, 16, 73), Transform::at(5.0, 9.0, 37.0, 37.0))?;
    masked(w, "probe-high-rotate", "High resampling a mask placed rotated at a fractional position", gray_noise(40, 50, 74), Transform { rotation: 15.0, ..Transform::at(10.5, 4.25, 40.0, 50.0) })?;
    // 8 source pixels over 64 put every sample phase on an odd sixteenth: Low's rounding ties.
    layer(w, "probe-low-8x", "Low at 8x: 8x8 at 64x64", images::noise(8, 8, 68, Alpha::Opaque), smooth(0.0, 0.0, 64.0, 64.0, 0.0))?;
    // A masked layer, rotated and translucent, with a mask of another size than its pixels.
    let mut d = w.doc("transform", "probe-mask-rotate", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let id = d.image("Masked", images::noise(40, 40, 69, Alpha::Varied), LayerSpec { opacity: Some(0.6), transform: Some(smooth(10.0, 14.0, 44.0, 36.0, 25.0)), ..spec() });
    d.mask(&id, gray_noise(24, 30, 75));
    w.write("transform", "probe-mask-rotate", "A rotated translucent layer through a mask of another size", d, vec![])?;
    // Folder masks drawn over the folder's own rectangle, which High resamples.
    for (case, t) in [("probe-folder-shrink", Transform::at(8.0, 4.0, 48.0, 40.0)), ("probe-folder-rotate", Transform { rotation: 20.0, ..Transform::at(12.0, 12.0, 40.0, 40.0) })] {
        let mut d = w.doc("transform", case, N, N);
        let folder = d.group("Folder", LayerSpec { transform: Some(t), ..spec() });
        d.mask(&folder, gray_noise(if case == "probe-folder-shrink" { 64 } else { 40 }, if case == "probe-folder-shrink" { 64 } else { 40 }, 76));
        d.image("Inner", white(N, N), LayerSpec { parent: Some(folder), ..spec() });
        w.write("transform", case, "A folder mask over the folder's own rectangle", d, vec![])?;
    }
    Ok(())
}

fn adj(kind: AdjustmentKind, f: impl FnOnce(&mut Adjustment)) -> Adjustment {
    let mut a = Adjustment::new(kind);
    f(&mut a);
    a
}

fn curve(points: &[(f64, f64)]) -> Vec<CurvePoint> {
    points.iter().map(|&(x, y)| CurvePoint { x, y }).collect()
}

fn adjust(w: &mut CaseWriter) -> Result<()> {
    use AdjustmentKind::*;
    let color = |r, g, b| AdjustmentColor { red: r, green: g, blue: b };
    let hsv_range = |range: ColorRange, h, s, l, invert: bool| {
        move |a: &mut Adjustment| {
            let mut adjustments = vec![(ColorRange::Master, RangeAdjustment { hue: 0.0, saturation: 0.0, lightness: 0.0 })];
            adjustments.push((range, RangeAdjustment { hue: h, saturation: s, lightness: l }));
            a.hsv_settings = Some(HueSaturationSettings {
                range,
                colorize: false,
                invert_range: invert,
                adjustments: RangeMap(adjustments),
                bands: RangeMap(ColorRange::ALL.iter().map(|&r| (r, r.default_band())).collect()),
            });
        }
    };
    let cases: Vec<(&str, &str, Adjustment)> = vec![
        ("hsl-warm", "Hue +40, saturation +30", adj(HueSaturation, |a| { a.hue = 40.0; a.saturation = 30.0; })),
        ("hsl-cool-light", "Hue -120, saturation -50, lightness +20", adj(HueSaturation, |a| { a.hue = -120.0; a.saturation = -50.0; a.lightness = 20.0; })),
        ("hsl-colorize", "Colorize at hue 200", adj(HueSaturation, |a| { a.colorize = true; a.hue = 200.0; a.saturation = 50.0; a.lightness = -10.0; })),
        ("hsl-reds", "Reds only: hue +30, saturation +20", adj(HueSaturation, hsv_range(ColorRange::Reds, 30.0, 20.0, 0.0, false))),
        ("hsl-blues-inverted", "Everything but blues: lightness -40", adj(HueSaturation, hsv_range(ColorRange::Blues, 0.0, -30.0, -40.0, true))),
        ("levels-rgb", "Levels 20 / 1.4 / 230", adj(Levels, |a| a.levels.ranges[0] = LevelRange { black: 20.0, gamma: 1.4, white: 230.0, ..Default::default() })),
        ("levels-red", "Red channel gamma 0.6", adj(Levels, |a| a.levels.ranges[1] = LevelRange { gamma: 0.6, ..Default::default() })),
        ("levels-output", "Output 30 to 220, blue input 0 to 180", adj(Levels, |a| {
            a.levels.ranges[0] = LevelRange { output_black: 30.0, output_white: 220.0, ..Default::default() };
            a.levels.ranges[3] = LevelRange { white: 180.0, ..Default::default() };
        })),
        ("curves-s", "An S curve", adj(Curves, |a| a.curves.channels[0] = curve(&[(0.0, 0.0), (64.0, 40.0), (192.0, 220.0), (255.0, 255.0)]))),
        ("curves-channels", "A warming curve per channel", adj(Curves, |a| {
            a.curves.channels[1] = curve(&[(0.0, 0.0), (120.0, 147.0), (255.0, 255.0)]);
            a.curves.channels[2] = curve(&[(0.0, 0.0), (100.0, 114.0), (255.0, 255.0)]);
            a.curves.channels[3] = curve(&[(0.0, 0.0), (115.0, 97.0), (255.0, 238.0)]);
        })),
        ("curves-steep", "A steep many-point curve", adj(Curves, |a| a.curves.channels[0] = curve(&[(0.0, 30.0), (40.0, 0.0), (90.0, 200.0), (150.0, 90.0), (200.0, 255.0), (255.0, 128.0)]))),
        ("exposure-up", "Exposure +1.5", adj(Exposure, |a| a.exposure_settings = Some(ExposureSettings { exposure: 1.5, offset: 0.0, gamma: 1.0 }))),
        ("exposure-down", "Exposure -1, offset +0.05, gamma 0.8", adj(Exposure, |a| a.exposure_settings = Some(ExposureSettings { exposure: -1.0, offset: 0.05, gamma: 0.8 }))),
        ("exposure-gamma", "Exposure +0.5, offset -0.1, gamma 1.6", adj(Exposure, |a| a.exposure_settings = Some(ExposureSettings { exposure: 0.5, offset: -0.1, gamma: 1.6 }))),
        ("gradient-map-default", "Black to white", adj(GradientMap, |a| a.gradient_map_settings = Some(GradientMapSettings::default()))),
        ("gradient-map-color", "Deep blue to orange", adj(GradientMap, |a| a.gradient_map_settings = Some(GradientMapSettings { shadows: color(0.05, 0.1, 0.4), highlights: color(1.0, 0.6, 0.1), reversed: false }))),
        ("gradient-map-reversed", "Reversed purple to green", adj(GradientMap, |a| a.gradient_map_settings = Some(GradientMapSettings { shadows: color(0.5, 0.0, 0.5), highlights: color(0.2, 0.9, 0.3), reversed: true }))),
        ("grain-default", "Default grain, seed 1", adj(Grain, |a| a.grain_settings = Some(GrainSettings { seed: 1, ..Default::default() }))),
        ("grain-coarse", "Amount 80, size 4, roughness 20", adj(Grain, |a| a.grain_settings = Some(GrainSettings { amount: 80.0, size: 4.0, roughness: 20.0, seed: 7 }))),
        ("grain-fine", "Amount 10, size 0.5, roughness 100", adj(Grain, |a| a.grain_settings = Some(GrainSettings { amount: 10.0, size: 0.5, roughness: 100.0, seed: 99 }))),
        ("noise-uniform", "Noise 10, uniform, color", adj(AddNoise, |a| { a.noise_amount = Some(10.0); a.noise_gaussian = Some(false); a.noise_monochromatic = Some(false); a.noise_seed = Some(1); })),
        ("noise-gaussian-mono", "Noise 50, Gaussian, monochromatic", adj(AddNoise, |a| { a.noise_amount = Some(50.0); a.noise_gaussian = Some(true); a.noise_monochromatic = Some(true); a.noise_seed = Some(2); })),
        ("noise-heavy", "Noise 200, Gaussian, color", adj(AddNoise, |a| { a.noise_amount = Some(200.0); a.noise_gaussian = Some(true); a.noise_monochromatic = Some(false); a.noise_seed = Some(3); })),
        ("gaussian-0.5", "Gaussian blur, radius 0.5", adj(GaussianBlur, |a| a.blur_radius = Some(0.5))),
        ("gaussian-3", "Gaussian blur, radius 3", adj(GaussianBlur, |a| a.blur_radius = Some(3.0))),
        ("gaussian-12", "Gaussian blur, radius 12", adj(GaussianBlur, |a| a.blur_radius = Some(12.0))),
        ("motion-0-10", "Motion blur, 0 degrees, 10 px", adj(MotionBlur, |a| { a.motion_angle = Some(0.0); a.motion_distance = Some(10.0); })),
        ("motion-45-20", "Motion blur, 45 degrees, 20 px", adj(MotionBlur, |a| { a.motion_angle = Some(45.0); a.motion_distance = Some(20.0); })),
        ("motion-90-5", "Motion blur, -90 degrees, 5 px", adj(MotionBlur, |a| { a.motion_angle = Some(-90.0); a.motion_distance = Some(5.0); })),
        ("invert", "Invert", Adjustment::new(Invert)),
        ("bw-default", "Black & White, default weights", adj(BlackWhite, |a| a.black_white_settings = Some(BlackWhiteSettings::default()))),
        ("bw-custom", "Black & White, strong reds and dark blues", adj(BlackWhite, |a| a.black_white_settings = Some(BlackWhiteSettings { reds: 150.0, yellows: -50.0, greens: 20.0, cyans: 200.0, blues: -150.0, magentas: 300.0, ..Default::default() }))),
        ("bw-tint", "Black & White with a sepia tint", adj(BlackWhite, |a| a.black_white_settings = Some(BlackWhiteSettings { tint: true, tint_hue: 35.0, tint_saturation: 40.0, ..Default::default() }))),
        ("balance-shadows", "Color Balance: shadows toward cyan and blue", adj(ColorBalance, |a| a.color_balance_settings = Some(ColorBalanceSettings { shadow_cyan_red: -60.0, shadow_yellow_blue: 40.0, preserve_luminosity: true, ..Default::default() }))),
        ("balance-mids", "Color Balance: midtones toward red and green", adj(ColorBalance, |a| a.color_balance_settings = Some(ColorBalanceSettings { mid_cyan_red: 50.0, mid_magenta_green: 30.0, preserve_luminosity: true, ..Default::default() }))),
        ("balance-highlights-raw", "Color Balance: highlights, luminosity not preserved", adj(ColorBalance, |a| a.color_balance_settings = Some(ColorBalanceSettings { highlight_cyan_red: 20.0, highlight_magenta_green: -70.0, highlight_yellow_blue: -30.0, preserve_luminosity: false, ..Default::default() }))),
    ];
    for (case, label, a) in cases {
        let mut d = w.doc("adjust", case, N, N);
        d.image("Photo", images::photo(N, N), spec());
        d.adjustment(label, a, spec());
        w.write("adjust", case, label, d, vec![])?;
    }
    // How an adjustment layer's own settings combine with what is below it.
    let curves = adj(Curves, |a| a.curves.channels[0] = curve(&[(0.0, 0.0), (64.0, 40.0), (192.0, 220.0), (255.0, 255.0)]));
    let variants: Vec<(&str, &str, LayerSpec, bool, Alpha)> = vec![
        ("mod-opacity", "Curves at 50% opacity", LayerSpec { opacity: Some(0.5), ..spec() }, false, Alpha::Opaque),
        ("mod-blend", "Curves in Luminosity mode", LayerSpec { blend: Some(BlendMode::Luminosity), ..spec() }, false, Alpha::Opaque),
        ("mod-mask", "Curves through a ramp mask", spec(), true, Alpha::Opaque),
        ("mod-translucent", "Curves over a translucent backdrop", spec(), false, Alpha::Varied),
    ];
    for (case, label, s, masked, alpha) in variants {
        let mut d = w.doc("adjust", case, N, N);
        let base = if matches!(alpha, Alpha::Varied) { images::noise(N, N, 51, alpha) } else { images::photo(N, N) };
        d.image("Photo", base, spec());
        let id = d.adjustment(label, curves.clone(), s);
        if masked {
            d.mask(&id, images::gray_ramp(N, N, true));
        }
        w.write("adjust", case, label, d, vec![])?;
    }
    let mut d = w.doc("adjust", "blur-translucent", N, N);
    d.image("Disc", images::disc(N, N, [240, 120, 20]), spec());
    d.adjustment("Gaussian Blur", adj(GaussianBlur, |a| a.blur_radius = Some(4.0)), spec());
    w.write("adjust", "blur-translucent", "Gaussian blur over a disc on transparency", d, vec![])?;
    let mut d = w.doc("adjust", "noise-translucent", N, N);
    d.image("Disc", images::disc(N, N, [60, 120, 220]), spec());
    d.adjustment("Add Noise", adj(AddNoise, |a| { a.noise_amount = Some(40.0); a.noise_seed = Some(9); }), spec());
    w.write("adjust", "noise-translucent", "Noise over a disc on transparency", d, vec![])?;
    Ok(())
}

fn stroke(size: f64, rgb: [f64; 3], opacity: f64, inside: bool) -> StrokeEffect {
    StrokeEffect { enabled: None, size, red: rgb[0], green: rgb[1], blue: rgb[2], opacity, inside }
}

fn shadow(angle: f64, distance: f64, blur: f64, rgb: [f64; 3], opacity: f64) -> ShadowEffect {
    ShadowEffect { enabled: None, angle, distance, blur, red: rgb[0], green: rgb[1], blue: rgb[2], opacity }
}

fn glow(size: f64, rgb: [f64; 3], opacity: f64) -> GlowEffect {
    GlowEffect { enabled: None, size, red: rgb[0], green: rgb[1], blue: rgb[2], opacity }
}

fn effects(w: &mut CaseWriter) -> Result<()> {
    const E: u32 = 96;
    let black = [0.0, 0.0, 0.0];
    let white = [1.0, 1.0, 1.0];
    let red = [0.9, 0.1, 0.1];
    let blue = [0.1, 0.2, 0.9];
    let yellow = [1.0, 0.85, 0.1];
    let fx = |f: &dyn Fn(&mut Effects)| {
        let mut e = Effects::default();
        f(&mut e);
        e
    };
    let cases: Vec<(&str, &str, Effects)> = vec![
        ("stroke-outside", "Stroke 2 px outside, black", fx(&|e| e.stroke = Some(stroke(2.0, black, 1.0, false)))),
        ("stroke-inside", "Stroke 5 px inside, red at 70%", fx(&|e| e.stroke = Some(stroke(5.0, red, 0.7, true)))),
        ("stroke-wide", "Stroke 10 px outside, blue at 50%", fx(&|e| e.stroke = Some(stroke(10.0, blue, 0.5, false)))),
        ("shadow-default", "Drop shadow, defaults", fx(&|e| e.shadow = Some(shadow(90.0, 20.0, 20.0, black, 0.5)))),
        ("shadow-hard", "Drop shadow at 45 degrees, 8 px, no blur", fx(&|e| e.shadow = Some(shadow(45.0, 8.0, 0.0, black, 0.75)))),
        ("shadow-red", "Red drop shadow at 210 degrees, 4 px, blur 12", fx(&|e| e.shadow = Some(shadow(210.0, 4.0, 12.0, red, 0.8)))),
        ("overlay-red", "Color overlay, red", fx(&|e| e.color_overlay = Some(ColorOverlayEffect { enabled: None, red: 0.9, green: 0.1, blue: 0.1, opacity: 1.0 }))),
        ("overlay-blue-half", "Color overlay, blue at 50%", fx(&|e| e.color_overlay = Some(ColorOverlayEffect { enabled: None, red: 0.1, green: 0.2, blue: 0.9, opacity: 0.5 }))),
        ("inner-shadow-default", "Inner shadow, defaults", fx(&|e| e.inner_shadow = Some(shadow(90.0, 10.0, 10.0, black, 0.5)))),
        ("inner-shadow-sharp", "Inner shadow at -30 degrees, 6 px, blur 2", fx(&|e| e.inner_shadow = Some(shadow(-30.0, 6.0, 2.0, blue, 0.9)))),
        ("outer-glow-default", "Outer glow, defaults", fx(&|e| e.outer_glow = Some(glow(20.0, white, 0.75)))),
        ("outer-glow-tight", "Outer glow 5 px, yellow", fx(&|e| e.outer_glow = Some(glow(5.0, yellow, 1.0)))),
        ("outer-glow-wide", "Outer glow 30 px, red at 50%", fx(&|e| e.outer_glow = Some(glow(30.0, red, 0.5)))),
        ("inner-glow-default", "Inner glow, defaults", fx(&|e| e.inner_glow = Some(glow(10.0, white, 0.75)))),
        ("inner-glow-dark", "Inner glow 3 px, black", fx(&|e| e.inner_glow = Some(glow(3.0, black, 1.0)))),
        ("all", "All six effects together", fx(&|e| {
            e.stroke = Some(stroke(3.0, black, 1.0, false));
            e.shadow = Some(shadow(120.0, 6.0, 6.0, black, 0.6));
            e.color_overlay = Some(ColorOverlayEffect { enabled: None, red: 0.2, green: 0.7, blue: 0.3, opacity: 0.4 });
            e.inner_shadow = Some(shadow(90.0, 3.0, 3.0, black, 0.5));
            e.outer_glow = Some(glow(8.0, yellow, 0.6));
            e.inner_glow = Some(glow(4.0, white, 0.5));
        })),
        ("disabled", "A disabled stroke beside an enabled shadow", fx(&|e| {
            e.stroke = Some(StrokeEffect { enabled: Some(false), ..stroke(6.0, red, 1.0, false) });
            e.shadow = Some(shadow(90.0, 6.0, 4.0, black, 0.5));
        })),
    ];
    for (i, (case, label, e)) in cases.into_iter().enumerate() {
        let mut d = w.doc("effects", case, E, E);
        // Alternate grounds: effects over transparency and over a picture.
        if i % 2 == 1 {
            d.image("Ground", images::photo(E, E), spec());
        }
        let id = d.image("Shape", images::disc(64, 64, [60, 140, 230]), LayerSpec { transform: Some(Transform::at(16.0, 16.0, 64.0, 64.0)), ..spec() });
        d.layer(&id).effects = Some(e);
        w.write("effects", case, label, d, vec![])?;
    }
    let base = || {
        let mut e = Effects::default();
        e.stroke = Some(stroke(4.0, black, 1.0, false));
        e.shadow = Some(shadow(45.0, 6.0, 4.0, black, 0.6));
        e
    };
    let variants: Vec<(&str, &str)> = vec![
        ("corners", "Stroke and shadow around hard corners"),
        ("masked", "Effects on a masked layer"),
        ("opacity", "Effects on a layer at 50% opacity"),
        ("multiply", "Effects on a Multiply layer"),
        ("scaled", "Effects on a layer drawn at 1.5x"),
        ("rotated", "Effects on a layer rotated 25 degrees"),
    ];
    for (case, label) in variants {
        let mut d = w.doc("effects", case, E, E);
        d.image("Ground", images::photo(E, E), spec());
        let mut s = LayerSpec { transform: Some(Transform::at(16.0, 16.0, 64.0, 64.0)), ..spec() };
        match case {
            "opacity" => s.opacity = Some(0.5),
            "multiply" => s.blend = Some(BlendMode::Multiply),
            "scaled" => s.transform = Some(Transform::at(8.0, 8.0, 80.0, 80.0)),
            "rotated" => s.transform = Some(Transform { rotation: 25.0, ..Transform::at(16.0, 16.0, 64.0, 64.0) }),
            _ => {}
        }
        let pixels = if case == "corners" { images::corners(64, 64, [230, 60, 40]) } else { images::disc(64, 64, [60, 140, 230]) };
        let id = d.image("Shape", pixels, s);
        d.layer(&id).effects = Some(base());
        if case == "masked" {
            d.mask(&id, images::gray_ramp(64, 64, true));
        }
        w.write("effects", case, label, d, vec![])?;
    }
    // The paths the cases above leave out: a base whose effects clip the layer above it; sizes
    // that round (stroke reach, blur radius), a fractional shadow offset and glows too small to
    // blur; a layer partly off the canvas whose effects reach further still.
    let mut d = w.doc("effects", "clip-base", E, E);
    d.image("Ground", images::noise(E, E, 31, Alpha::Opaque), spec());
    let base = d.image("Base", images::disc(48, 48, [240, 240, 240]), LayerSpec { transform: Some(Transform::at(24.0, 24.0, 48.0, 48.0)), ..spec() });
    d.layer(&base).effects = Some(Effects { stroke: Some(stroke(3.0, red, 1.0, false)), shadow: Some(shadow(60.0, 5.0, 3.0, black, 0.7)), ..Default::default() });
    let clipped = d.image("Clipped", images::photo(E, E), spec());
    d.layer(&clipped).mask_source_id = Some(base);
    w.write("effects", "clip-base", "A base with effects, a photo clipped to it", d, vec![])?;

    let mut d = w.doc("effects", "fractional", E, E);
    d.image("Ground", images::photo(E, E), spec());
    let id = d.image("Shape", images::noise(48, 48, 32, Alpha::Varied), LayerSpec { transform: Some(Transform::at(24.0, 24.0, 48.0, 48.0)), ..spec() });
    d.layer(&id).effects = Some(Effects {
        stroke: Some(stroke(2.5, blue, 0.8, true)),
        shadow: Some(shadow(33.0, 7.3, 5.5, red, 0.65)),
        color_overlay: Some(ColorOverlayEffect { enabled: None, red: 1.0, green: 0.85, blue: 0.1, opacity: 0.3 }),
        outer_glow: Some(glow(0.01, white, 0.9)),
        inner_glow: Some(glow(0.01, black, 0.9)),
        ..Default::default()
    });
    w.write("effects", "fractional", "Rounded sizes, a fractional shadow offset and glows too small to blur", d, vec![])?;

    let mut d = w.doc("effects", "off-canvas", E, E);
    d.image("Ground", images::photo(E, E), spec());
    let id = d.image("Shape", images::corners(48, 48, [230, 60, 40]), LayerSpec { transform: Some(Transform::at(-20.0, 60.0, 48.0, 48.0)), ..spec() });
    d.layer(&id).effects = Some(Effects {
        shadow: Some(shadow(-135.0, 12.0, 8.0, black, 0.0)),
        outer_glow: Some(glow(6.0, yellow, 0.8)),
        inner_shadow: Some(shadow(150.0, 4.0, 0.0, blue, 0.6)),
        ..Default::default()
    });
    w.write("effects", "off-canvas", "A layer partly off the canvas, a hidden shadow widening its margin", d, vec![])?;
    Ok(())
}

fn document(w: &mut CaseWriter) -> Result<()> {
    let make = |w: &CaseWriter, case: &str| {
        let mut d = w.doc("document", case, N, N);
        d.image("Photo", images::photo(N, N), spec());
        let id = d.image("Disc", images::disc(32, 32, [240, 240, 240]), LayerSpec { transform: Some(Transform { rotation: 15.0, ..Transform::at(20.0, 12.0, 32.0, 32.0) }), ..spec() });
        d.mask(&id, images::gray_ramp(32, 32, false));
        d.layer(&id).effects = Some(Effects { stroke: Some(stroke(2.0, [0.0, 0.0, 0.0], 1.0, false)), ..Default::default() });
        d
    };
    let cases: Vec<(&str, &str, serde_json::Value)> = vec![
        ("crop-inside", "Crop to 40x32 at (8, 8)", json!({ "op": "crop", "rect": [8, 8, 40, 32] })),
        ("crop-past-edge", "Crop a rectangle that runs past the canvas", json!({ "op": "crop", "rect": [32, -8, 48, 48] })),
        ("canvas-grow", "Canvas Size 96x80, content moved (16, 8)", json!({ "op": "canvasSize", "width": 96, "height": 80, "contentOffset": [16, 8] })),
        ("canvas-shrink", "Canvas Size 48x48, content moved (-8, -8)", json!({ "op": "canvasSize", "width": 48, "height": 48, "contentOffset": [-8, -8] })),
        ("canvas-fill", "Canvas Size 90x75 around the center, filled with a color", json!({ "op": "canvasSize", "width": 90, "height": 75, "anchor": 4, "fill": [0.2, 0.4, 0.6] })),
        ("canvas-anchor-corner", "Canvas Size 80x70 anchored bottom right", json!({ "op": "canvasSize", "width": 80, "height": 70, "anchor": 8 })),
        ("image-up-high", "Image Size 128x128, High quality", json!({ "op": "imageSize", "width": 128, "height": 128, "sampling": "High quality" })),
        ("image-down-smooth", "Image Size 32x32, Smooth", json!({ "op": "imageSize", "width": 32, "height": 32, "sampling": "Smooth" })),
        ("image-odd-nearest", "Image Size 50x70, Nearest", json!({ "op": "imageSize", "width": 50, "height": 70, "sampling": "Nearest" })),
    ];
    for (case, label, op) in cases {
        let d = make(w, case);
        w.write("document", case, label, d, vec![op])?;
    }
    Ok(())
}

fn filters(w: &mut CaseWriter) -> Result<()> {
    let dither = |style: &str| json!({ "dither": { "style": style } });
    let cases: Vec<(&str, &str, &str, serde_json::Value, bool)> = vec![
        ("gaussian-0.5", "Gaussian Blur 0.5", "Gaussian Blur", json!({ "radius": 0.5 }), false),
        ("gaussian-2", "Gaussian Blur 2", "Gaussian Blur", json!({ "radius": 2 }), false),
        ("gaussian-8", "Gaussian Blur 8", "Gaussian Blur", json!({ "radius": 8 }), false),
        ("gaussian-grow", "Gaussian Blur 4 on a disc, growing the layer", "Gaussian Blur", json!({ "radius": 4 }), true),
        ("motion-0", "Motion Blur 0 degrees, 10 px", "Motion Blur", json!({ "angle": 0, "distance": 10 }), false),
        ("motion-30", "Motion Blur 30 degrees, 20 px", "Motion Blur", json!({ "angle": 30, "distance": 20 }), false),
        ("motion-grow", "Motion Blur -90 degrees, 6 px on a disc", "Motion Blur", json!({ "angle": -90, "distance": 6 }), true),
        ("noise-uniform", "Add Noise 10, uniform, color", "Add Noise", json!({ "amount": 10 }), false),
        ("noise-gaussian-mono", "Add Noise 40, Gaussian, monochromatic", "Add Noise", json!({ "amount": 40, "gaussian": true, "monochromatic": true }), false),
        ("vignette-default", "Vignette, defaults", "Vignette", json!({}), false),
        ("vignette-custom", "Vignette 80, red, midpoint 30, roundness -50, feather 20", "Vignette", json!({ "vignetteAmount": 80, "vignetteColor": { "red": 0.8, "green": 0.1, "blue": 0.1 }, "vignetteMidpoint": 30, "vignetteRoundness": -50, "vignetteFeather": 20, "vignetteHighlights": 0 }), false),
        ("bloom-default", "Bloom / Glow, defaults", "Bloom / Glow", json!({}), false),
        ("bloom-strong", "Bloom / Glow 80, radius 10", "Bloom / Glow", json!({ "bloomAmount": 80, "bloomRadius": 10 }), false),
        ("tonal-default", "Tonal Contrast, defaults", "Tonal Contrast", json!({}), false),
        ("tonal-strong", "Tonal Contrast 100, radius 6, shadows -50", "Tonal Contrast", json!({ "tonalAmount": 100, "tonalRadius": 6, "tonalShadows": -50, "tonalMidtones": 100, "tonalHighlights": -30 }), false),
        ("lens-barrel", "Lens Correction +30", "Lens Correction", json!({ "distortion": 30 }), false),
        ("lens-pincushion", "Lens Correction -40", "Lens Correction", json!({ "distortion": -40 }), false),
        ("curves", "Curves filter, an S curve", "Curves", json!({ "curves": { "channel": "RGB", "channels": [
            [{ "x": 0, "y": 0 }, { "x": 64, "y": 40 }, { "x": 192, "y": 220 }, { "x": 255, "y": 255 }],
            [{ "x": 0, "y": 0 }, { "x": 255, "y": 255 }], [{ "x": 0, "y": 0 }, { "x": 255, "y": 255 }], [{ "x": 0, "y": 0 }, { "x": 255, "y": 255 }]] } }), false),
        ("exposure", "Exposure filter +1", "Exposure", json!({ "exposure": { "exposure": 1, "offset": 0, "gamma": 1 } }), false),
        ("gradient-map", "Gradient Map filter, blue to orange", "Gradient Map", json!({ "gradientMap": { "shadows": { "red": 0.05, "green": 0.1, "blue": 0.4 }, "highlights": { "red": 1, "green": 0.6, "blue": 0.1 }, "reversed": false } }), false),
        ("grain", "Grain filter, amount 50", "Grain", json!({ "grain": { "amount": 50, "size": 2, "roughness": 50, "seed": 0 } }), false),
        ("black-white", "Black & White filter, defaults", "Black & White", json!({}), false),
        ("color-balance", "Color Balance filter, midtones toward red", "Color Balance", json!({ "colorBalance": { "midCyanRed": 50 } }), false),
        ("remove-bg-basic", "Remove Background, Basic", "Remove Background", json!({ "backgroundQuality": "Basic" }), true),
        ("remove-bg-advanced", "Remove Background, Advanced", "Remove Background", json!({ "backgroundQuality": "Advanced" }), true),
        ("dither-atkinson", "Dither, Atkinson", "Dither", dither("Atkinson (Classic Mac)"), false),
        ("dither-floyd", "Dither, Floyd-Steinberg", "Dither", dither("Floyd–Steinberg"), false),
        ("dither-bayer2", "Dither, Bayer 2x2", "Dither", dither("Bayer 2 × 2"), false),
        ("dither-bayer4", "Dither, Bayer 4x4", "Dither", dither("Bayer 4 × 4"), false),
        ("dither-bayer8", "Dither, Bayer 8x8", "Dither", dither("Bayer 8 × 8"), false),
        ("dither-dots", "Dither, halftone dots", "Dither", dither("Halftone Dots"), false),
        ("dither-lines", "Dither, halftone lines", "Dither", dither("Halftone Lines"), false),
        ("dither-diamonds", "Dither, halftone diamonds", "Dither", dither("Halftone Diamonds"), false),
        ("dither-patterns", "Dither, Mac patterns", "Dither", dither("Mac Patterns"), false),
        ("dither-ascii", "Dither, ASCII", "Dither", dither("ASCII"), false),
        ("dither-scanlines", "Dither, scanlines", "Dither", dither("Scanlines (CRT)"), false),
    ];
    for (case, label, kind, settings, on_disc) in cases {
        let mut d = w.doc("filters", case, N, N);
        let target = if on_disc {
            d.image("Ground", images::photo(N, N), spec());
            d.image("Disc", images::disc(48, 48, [240, 120, 30]), LayerSpec { transform: Some(Transform::at(8.0, 8.0, 48.0, 48.0)), ..spec() })
        } else {
            d.image("Photo", images::photo(N, N), spec())
        };
        let mut op = json!({ "op": "filter", "layer": target, "kind": kind, "settings": settings });
        if kind == "Add Noise" || kind == "Grain" {
            op["seed"] = json!(1);
        }
        w.write("filters", case, label, d, vec![op])?;
    }
    // Probes. Mac Patterns in Original colors at full contrast on a bright image marks every pixel,
    // so the result is the image Dither worked on: Core Graphics's high-quality reduction by the
    // pixel size, which isn't documented.
    let reduced = |pixel_size: u32| json!({ "dither": { "style": "Mac Patterns", "colors": "Original", "contrast": 100, "pixelSize": pixel_size } });
    let mut bright_photo = images::photo(N, N);
    for p in bright_photo.pixels_mut() {
        for c in 0..3 {
            p[c] = 140 + ((p[c] as u32 * 115 + 127) / 255) as u8;
        }
    }
    let probes: Vec<(&str, &str, RgbaImage, serde_json::Value)> = vec![
        ("probe-reduce-2", "Dither's reduction by 2, shown through Mac Patterns", images::bright_noise(N, N, 71, Alpha::Opaque), reduced(2)),
        ("probe-reduce-3", "Dither's reduction by 3, shown through Mac Patterns", images::bright_noise(N, N, 72, Alpha::Opaque), reduced(3)),
        ("probe-reduce-2-alpha", "Dither's reduction by 2 of translucent pixels", images::bright_noise(N, N, 73, Alpha::Varied), reduced(2)),
        ("probe-reduce-2-smooth", "Dither's reduction by 2 of a smooth picture", bright_photo, reduced(2)),
        // The dithering itself, one pixel per pixel so no reduction is involved.
        ("probe-floyd-1", "Dither, Floyd-Steinberg, pixel size 1", images::photo(N, N), json!({ "dither": { "style": "Floyd–Steinberg", "pixelSize": 1 } })),
        ("probe-atkinson-1-original", "Dither, Atkinson, pixel size 1, 4 levels, Original, diffusion 80", images::photo(N, N),
            json!({ "dither": { "style": "Atkinson (Classic Mac)", "pixelSize": 1, "levels": 4, "colors": "Original", "diffusion": 80 } })),
        ("probe-bayer4-1-two", "Dither, Bayer 4x4, pixel size 1, two colors, contrast 30", images::photo(N, N),
            json!({ "dither": { "style": "Bayer 4 × 4", "pixelSize": 1, "colors": "Two Colors", "dark": { "red": 0.1, "green": 0.2, "blue": 0.4 }, "light": { "red": 1, "green": 0.9, "blue": 0.6 }, "contrast": 30 } })),
        ("probe-dots-1", "Dither, halftone dots, pixel size 1, cell 6, angle 30, dark on light", images::photo(N, N),
            json!({ "dither": { "style": "Halftone Dots", "pixelSize": 1, "cellSize": 6, "angle": 30, "lightOnDark": false } })),
        ("probe-lines-1-original", "Dither, halftone lines, pixel size 1, Original", images::photo(N, N),
            json!({ "dither": { "style": "Halftone Lines", "pixelSize": 1, "colors": "Original" } })),
        ("probe-diamonds-1", "Dither, halftone diamonds, pixel size 1, angle -20", images::photo(N, N),
            json!({ "dither": { "style": "Halftone Diamonds", "pixelSize": 1, "angle": -20 } })),
        ("probe-patterns-1", "Dither, Mac patterns, pixel size 1", images::photo(N, N), json!({ "dither": { "style": "Mac Patterns", "pixelSize": 1 } })),
        ("probe-scanlines-flat", "Dither, scanlines without glow", images::photo(N, N), json!({ "dither": { "style": "Scanlines (CRT)", "glow": 0 } })),
        ("probe-scanlines-dots", "Dither, scanlines without glow, dots 60, wobble 4, spacing 6, Original", images::photo(N, N),
            json!({ "dither": { "style": "Scanlines (CRT)", "glow": 0, "dots": 60, "wobble": 4, "lineSpacing": 6, "colors": "Original" } })),
        ("probe-dot-pixels", "Dither, Bayer 8x8 in round pixels of 4, two colors", images::photo(N, N),
            json!({ "dither": { "style": "Bayer 8 × 8", "pixelSize": 4, "pixelShape": "Dot", "colors": "Two Colors", "dark": { "red": 0.2, "green": 0, "blue": 0.3 } } })),
    ];
    for (case, label, image, settings) in probes {
        let mut d = w.doc("filters", case, N, N);
        let target = d.image("Photo", image, spec());
        w.write("filters", case, label, d, vec![json!({ "op": "filter", "layer": target, "kind": "Dither", "settings": settings })])?;
    }
    Ok(())
}

fn export(w: &mut CaseWriter) -> Result<()> {
    // PNG export: pixels are every case's reference; these vary what the metadata records.
    for (case, resolution, label) in [
        ("png-72", None, "PNG at the default 72 ppi"),
        ("png-300", Some(300.0), "PNG at 300 ppi"),
        ("png-96-5", Some(96.5), "PNG at a fractional 96.5 ppi"),
    ] {
        let mut d = w.doc("export", case, 37, 23);
        d.resolution = resolution;
        d.image("Photo", images::photo(37, 23), spec());
        d.image("Translucent", images::noise(37, 23, 61, Alpha::Varied), LayerSpec { opacity: Some(0.8), ..spec() });
        w.write("export", case, label, d, vec![])?;
    }
    // JPEG export: quality and matte, over opaque and translucent content.
    let jpegs: [(&str, &str, f64, [f64; 3], bool); 6] = [
        ("jpeg-q85-white", "JPEG at the default quality 0.85 on white", 0.85, [1.0, 1.0, 1.0], true),
        ("jpeg-q50-black", "JPEG at 0.5 on black", 0.5, [0.0, 0.0, 0.0], true),
        ("jpeg-q100", "JPEG at quality 1", 1.0, [1.0, 1.0, 1.0], false),
        ("jpeg-q0", "JPEG at quality 0", 0.0, [1.0, 1.0, 1.0], false),
        ("jpeg-q70-red", "JPEG at 0.7 on a red matte", 0.7, [0.9, 0.1, 0.1], true),
        ("jpeg-q92-opaque", "JPEG at 0.92, fully opaque", 0.92, [1.0, 1.0, 1.0], false),
    ];
    for (case, label, quality, matte, translucent) in jpegs {
        let mut d = w.doc("export", case, 48, 40);
        d.image("Photo", images::photo(48, 40), spec());
        if translucent {
            d.image("Translucent", images::noise(48, 40, 62, Alpha::Varied), LayerSpec { opacity: Some(0.8), ..spec() });
            d.image("Hole", images::disc(48, 40, [30, 200, 90]), LayerSpec { blend: Some(BlendMode::Normal), ..spec() });
        }
        if translucent {
            // A transparent corner so the matte shows through.
            d.layers[0].transform = Transform::at(8.0, 6.0, 48.0, 40.0);
            d.layers[0].transform.size = [48.0, 40.0];
        }
        w.write_with("export", case, label, d, vec![], json!({ "jpeg": { "quality": quality, "matte": matte } }))?;
    }
    // Probes that read ImageIO's quantization tables off every quality step.
    for step in 0..=100u32 {
        let quality = step as f64 / 100.0;
        let case = format!("jpeg-table-{step:03}");
        let mut d = w.doc("export", &case, 16, 16);
        d.image("Photo", images::photo(16, 16), spec());
        w.write_with("export", &case, &format!("JPEG quantization tables at quality {quality}"), d, vec![], json!({ "jpeg": { "quality": quality } }))?;
    }
    Ok(())
}
