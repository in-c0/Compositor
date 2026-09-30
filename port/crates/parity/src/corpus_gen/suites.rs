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
    selections(w)?;
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
    // Folder masks enlarged with High, and shrunk with Smooth.
    for (case, t, size) in [
        ("probe-folder-grow", Transform::at(4.0, 6.0, 56.0, 52.0), 20),
        ("probe-folder-smooth", Transform { sampling: Sampling::Smooth, ..Transform::at(8.0, 4.0, 48.0, 40.0) }, 64),
    ] {
        let mut d = w.doc("transform", case, N, N);
        let folder = d.group("Folder", LayerSpec { transform: Some(t), ..spec() });
        d.mask(&folder, gray_noise(size, size, 77));
        d.image("Inner", white(N, N), LayerSpec { parent: Some(folder), ..spec() });
        w.write("transform", case, "A folder mask resampled over the folder's own rectangle", d, vec![])?;
    }
    // High shrinking an image vertically while enlarging it horizontally, as `non-uniform` does.
    layer(w, "probe-high-image-v", "High: 60x64 at 64x40", images::noise(60, 64, 78, Alpha::Opaque), Transform::at(0.0, 12.0, 64.0, 40.0))?;
    let dots = RgbaImage::from_fn(60, 64, |x, y| {
        let v = if x % 6 == 2 && y % 8 == 3 { 255 } else { 0 };
        image::Rgba([v, v, v, 255])
    });
    layer(w, "probe-high-image-impulse", "High shrinking single white pixels vertically", dots, Transform::at(0.0, 12.0, 64.0, 40.0))?;
    // A masked layer enlarged with High.
    let mut d = w.doc("transform", "probe-mask-high-grow", N, N);
    let id = d.image("Masked", images::noise(16, 16, 79, Alpha::Opaque), LayerSpec { transform: Some(Transform::at(5.0, 9.0, 37.0, 37.0)), ..spec() });
    d.mask(&id, gray_noise(16, 16, 80));
    w.write("transform", "probe-mask-high-grow", "A masked layer enlarged with High", d, vec![])?;
    // An unlinked mask off the pixel grid, with a white border so what lies beyond it is white.
    let bordered = GrayImage::from_fn(40, 50, |x, y| {
        let edge = x == 0 || y == 0 || x == 39 || y == 49;
        image::Luma([if edge { 255 } else { (images::hash(81 ^ (y * 40 + x)) >> 11) as u8 }])
    });
    masked(w, "probe-unlinked-fraction", "An unlinked mask at a fractional position and size", bordered, Transform::at(10.5, 3.25, 41.5, 50.75))?;
    // A layer shrunk to a quarter with a mask of its size: the mask is halved as well.
    let mut d = w.doc("transform", "probe-halve-mask", N, N);
    let id = d.image("Masked", white(64, 64), LayerSpec { transform: Some(smooth(8.0, 8.0, 16.0, 16.0, 0.0)), ..spec() });
    d.mask(&id, gray_noise(64, 64, 82));
    w.write("transform", "probe-halve-mask", "A mask halved twice with its layer", d, vec![])?;
    // Just under half size: one halving, then Low so close to 1:1 that it copies pixels.
    layer(w, "probe-halve-1", "One Lanczos halving, then a near copy", images::noise(64, 64, 83, Alpha::Varied), smooth(8.0, 8.0, 31.99, 31.99, 0.0))?;
    layer(w, "probe-halve-1-opaque", "One Lanczos halving of opaque pixels, then a near copy", images::noise(64, 64, 84, Alpha::Opaque), smooth(8.0, 8.0, 31.99, 31.99, 0.0))?;
    let mut d = w.doc("transform", "probe-halve-1-mask", N, N);
    let id = d.image("Masked", white(64, 64), LayerSpec { transform: Some(smooth(8.0, 8.0, 31.99, 31.99, 0.0)), ..spec() });
    d.mask(&id, gray_noise(64, 64, 85));
    w.write("transform", "probe-halve-1-mask", "A mask halved once with its layer, then a near copy", d, vec![])?;
    // A rotated, masked layer inside a masked folder, and one clipped to an upright base.
    let mut d = w.doc("transform", "probe-folder-rotated-child", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let folder = d.group("Folder", spec());
    d.mask(&folder, images::gray_ramp(N, N, false));
    let id = d.image("Inner", images::noise(40, 40, 86, Alpha::Opaque), LayerSpec { parent: Some(folder), transform: Some(smooth(12.0, 10.0, 40.0, 44.0, 20.0)), ..spec() });
    d.mask(&id, gray_noise(40, 40, 87));
    w.write("transform", "probe-folder-rotated-child", "A rotated masked layer inside a masked folder", d, vec![])?;
    let mut d = w.doc("transform", "probe-clip-rotated", N, N);
    d.image("Backdrop", images::photo(N, N), spec());
    let base = d.image("Base", images::disc(N, N, [250, 250, 250]), spec());
    let id = d.image("Clipped", images::noise(40, 40, 88, Alpha::Opaque), LayerSpec { transform: Some(smooth(10.0, 12.0, 44.0, 40.0, -25.0)), ..spec() });
    d.layer(&id).mask_source_id = Some(base);
    w.write("transform", "probe-clip-rotated", "A rotated layer clipped to an upright base", d, vec![])?;
    cg_high_probes(w)
}

/// Core Graphics' High interpolation shrinking an image, drawn directly with `CGContext.draw`
/// (the harness's `probeDraw`): per axis at several factors, the other axis 1:1, and both axes
/// on noise.
fn cg_high_probes(w: &mut CaseWriter) -> Result<()> {
    // 128 pixels along the probed axis in four bands of 8 lines: white impulses on gray, black
    // impulses on gray, a rising and a falling step, and noise.
    let pattern = |t: u32, band: u32, c: u32| -> u8 {
        match band {
            0 => if t % 16 == 5 { 255 } else { 128 },
            1 => if t % 16 == 5 { 0 } else { 128 },
            2 => if (40..88).contains(&t) { 223 } else { 32 },
            _ => (images::hash(0x5eed ^ (t * 3 + c)) >> 24) as u8,
        }
    };
    let bands = |across: bool| {
        let (width, height) = if across { (128, 32) } else { (32, 128) };
        RgbaImage::from_fn(width, height, |x, y| {
            let (t, band) = if across { (x, y / 8) } else { (y, x / 8) };
            image::Rgba([pattern(t, band, 0), pattern(t, band, 1), pattern(t, band, 2), 255])
        })
    };
    let probe = |w: &mut CaseWriter, case: &str, label: &str, src: RgbaImage, size: (u32, u32), rect: [f64; 4]| -> Result<()> {
        let mut d = w.doc("transform", case, src.width(), src.height());
        let id = d.image("Probe", src, spec());
        let op = json!({ "op": "probeDraw", "layer": id, "width": size.0, "height": size.1, "rect": rect, "quality": "high" });
        w.write("transform", case, label, d, vec![op])
    };
    for f in [0.9f64, 0.75, 0.5, 0.33, 0.25] {
        let n = 128.0 * f;
        probe(w, &format!("cg-high-x-{f}"), &format!("Core Graphics High shrinking across to {f}"), bands(true), (n.ceil() as u32, 32), [0.0, 0.0, n, 32.0])?;
        probe(w, &format!("cg-high-y-{f}"), &format!("Core Graphics High shrinking down to {f}"), bands(false), (32, n.ceil() as u32), [0.0, 0.0, 32.0, n])?;
    }
    // The same off the pixel grid.
    for (f, offset) in [(0.5f64, 0.25f64), (0.75, 0.5)] {
        let n = 128.0 * f;
        probe(w, &format!("cg-high-x-{f}-at-{offset}"), &format!("Core Graphics High shrinking across to {f}, {offset} pixels right"), bands(true), ((n + offset).ceil() as u32, 32), [offset, 0.0, n, 32.0])?;
    }
    // Both axes, on noise and on translucent noise.
    for f in [0.75f64, 0.5, 0.33] {
        let n = 64.0 * f;
        probe(w, &format!("cg-high-xy-{f}"), &format!("Core Graphics High shrinking noise to {f}"), images::noise(64, 64, 90, Alpha::Opaque), (n.ceil() as u32, n.ceil() as u32), [0.0, 0.0, n, n])?;
    }
    probe(w, "cg-high-xy-0.5-alpha", "Core Graphics High halving translucent noise", images::noise(64, 64, 91, Alpha::Varied), (32, 32), [0.0, 0.0, 32.0, 32.0])?;
    // Noise on every line, the other axis 1:1: each line is its own experiment, so each output
    // pixel's weights can be read off.
    let lines = |across: bool, n: u32, seed: u32| {
        let (width, height) = if across { (n, 32) } else { (32, n) };
        RgbaImage::from_fn(width, height, |x, y| {
            let v = |c: u32| (images::hash(seed.wrapping_mul(0x9e37_79b9) ^ ((y * width + x) * 3 + c)) >> 24) as u8;
            image::Rgba([v(0), v(1), v(2), 255])
        })
    };
    for width in [127.0, 120.0, 115.2, 115.0, 100.0, 96.0, 80.0, 64.0, 51.2, 42.24, 42.0, 32.0, 25.6] {
        probe(w, &format!("cg-noise-x-{width}"), &format!("Core Graphics High shrinking noise lines across, 128 to {width}"), lines(true, 128, 92), ((width as f64).ceil() as u32, 32), [0.0, 0.0, width, 32.0])?;
    }
    // The sizes the gated cases shrink by: 64 to 40 down (`non-uniform`), and Dither's thirds.
    probe(w, "cg-noise-y-64-40", "Core Graphics High shrinking noise lines down, 64 to 40", lines(false, 64, 93), (32, 40), [0.0, 0.0, 32.0, 40.0])?;
    probe(w, "cg-noise-x-64-21.33", "Core Graphics High shrinking noise lines across, 64 to a third", lines(true, 64, 94), (22, 32), [0.0, 0.0, 64.0 / 3.0, 32.0])?;
    probe(w, "cg-noise-x-128-96-at-0.25", "Core Graphics High shrinking noise lines across to 96, a quarter pixel right", lines(true, 128, 95), (97, 32), [0.25, 0.0, 96.0, 32.0])?;
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

/// Selections, compared through `<case>.selection.png` (the coverage the Mac rasterizes at
/// document size) as well as the flattened image, which selecting leaves alone.
fn selections(w: &mut CaseWriter) -> Result<()> {
    use serde_json::Value;
    const F: &str = "selections";
    let marquee = |shape: &str, from: [f64; 2], to: [f64; 2]| json!({ "op": "marquee", "shape": shape, "from": from, "to": to });
    let with = |mut op: Value, key: &str, value: Value| {
        op[key] = value;
        op
    };
    let lasso = |kind: &str, points: &[[f64; 2]]| json!({ "op": "lasso", "kind": kind, "points": points });
    let polygon = |points: &[[f64; 2]]| lasso("Polygonal", points);
    let modify = |kind: &str, amount: u32| json!({ "op": "modifySelection", kind: amount });
    let add = |op: Value| with(op, "mode", json!("Add"));
    let subtract = |op: Value| with(op, "mode", json!("Subtract"));
    let aliased = |op: Value| with(op, "antialias", json!(false));
    let rect = || marquee("Rectangle", [10.0, 12.0], [50.0, 40.0]);
    let ellipse = || marquee("Ellipse", [8.0, 10.0], [56.0, 50.0]);
    let triangle = || polygon(&[[10.5, 5.25], [58.75, 30.5], [12.2, 59.9]]);
    // Trigonometry differs in the last bits between platforms; 1/64 px steps keep the corpus the same everywhere.
    let q = |v: f64| (v * 64.0).round() / 64.0;
    // A wobbly loop, as a freehand drag records it: fractional points about a pixel apart.
    let wobble: Vec<[f64; 2]> = (0..90)
        .map(|i| {
            let t = i as f64 / 90.0 * std::f64::consts::TAU;
            let r = 20.0 + 3.5 * (5.0 * t).sin();
            [q(32.3 + r * t.cos()), q(31.7 + r * t.sin() * 0.9)]
        })
        .collect();
    let star: Vec<[f64; 2]> = (0..5)
        .map(|i| {
            let t = (i as f64 * 144.0 - 90.0).to_radians();
            [q(32.0 + 27.5 * t.cos()), q(33.0 + 27.5 * t.sin())]
        })
        .collect();

    // Shape tools on a plain photo. The pixels don't matter to these; only the outline does.
    let shapes: Vec<(&str, &str, Vec<Value>)> = vec![
        ("rect", "Rectangular Marquee (10, 12) to (50, 40)", vec![rect()]),
        ("rect-fractional", "Rectangular Marquee between fractional points, snapped to whole pixels", vec![marquee("Rectangle", [10.3, 12.6], [49.4, 40.5])]),
        ("rect-reversed", "Rectangular Marquee dragged up and to the left", vec![marquee("Rectangle", [50.0, 40.0], [10.0, 12.0])]),
        ("rect-square", "Rectangular Marquee with Shift, a square", vec![with(marquee("Rectangle", [8.0, 8.0], [40.0, 30.0]), "square", json!(true))]),
        ("rect-past-canvas", "Rectangular Marquee running past the canvas, clipped to it", vec![marquee("Rectangle", [-10.0, -5.0], [30.0, 80.0])]),
        ("rect-thin", "A one-pixel-wide Rectangular Marquee", vec![marquee("Rectangle", [20.0, 4.0], [21.0, 60.0])]),
        ("ellipse", "Elliptical Marquee, anti-aliased", vec![ellipse()]),
        ("ellipse-aliased", "Elliptical Marquee without anti-aliasing", vec![aliased(ellipse())]),
        ("ellipse-circle", "Elliptical Marquee with Shift, a circle", vec![with(marquee("Ellipse", [16.0, 16.0], [47.0, 45.0]), "square", json!(true))]),
        ("ellipse-small", "A 5x3 Elliptical Marquee", vec![marquee("Ellipse", [30.0, 30.0], [35.0, 33.0])]),
        ("ellipse-small-aliased", "A 5x3 Elliptical Marquee without anti-aliasing", vec![aliased(marquee("Ellipse", [30.0, 30.0], [35.0, 33.0]))]),
        ("ellipse-wide", "A long, flat Elliptical Marquee", vec![marquee("Ellipse", [3.0, 5.0], [60.0, 20.0])]),
        ("ellipse-past-canvas", "Elliptical Marquee running past the canvas, clipped to it", vec![marquee("Ellipse", [-20.0, 20.0], [40.0, 90.0])]),
        ("ellipse-past-canvas-aliased", "Elliptical Marquee past the canvas without anti-aliasing", vec![aliased(marquee("Ellipse", [-20.0, 20.0], [40.0, 90.0]))]),
        ("polygon-triangle", "Polygonal Lasso triangle with fractional corners", vec![triangle()]),
        ("polygon-triangle-aliased", "Polygonal Lasso triangle without anti-aliasing", vec![aliased(triangle())]),
        ("polygon-star", "A self-crossing five-point star (the winding rule fills the middle)", vec![polygon(&star)]),
        ("polygon-star-aliased", "The star without anti-aliasing", vec![aliased(polygon(&star))]),
        ("polygon-shallow", "A quadrilateral with nearly horizontal and nearly vertical edges", vec![polygon(&[[4.0, 10.2], [60.0, 13.1], [58.6, 55.0], [6.5, 52.7]])]),
        ("polygon-sliver", "A long triangle thinner than a pixel", vec![polygon(&[[2.0, 30.0], [62.0, 33.0], [2.0, 30.6]])]),
        ("polygon-tiny", "A triangle smaller than one pixel", vec![polygon(&[[20.2, 20.2], [20.9, 20.4], [20.5, 20.8]])]),
        ("polygon-tiny-aliased", "A triangle smaller than one pixel, without anti-aliasing", vec![aliased(polygon(&[[20.2, 20.2], [20.9, 20.4], [20.5, 20.8]]))]),
        ("polygon-past-canvas", "A Polygonal Lasso that leaves the canvas and comes back", vec![polygon(&[[-12.5, 8.0], [40.0, -9.5], [75.25, 44.0], [20.0, 70.0], [30.0, 30.0]])]),
        ("polygon-integer", "A Polygonal Lasso on whole pixels, with diagonal edges", vec![polygon(&[[8.0, 8.0], [56.0, 8.0], [40.0, 56.0], [8.0, 40.0]])]),
        ("polygon-two-points", "A Polygonal Lasso with two points deselects", vec![json!({ "op": "selectAll" }), polygon(&[[5.0, 5.0], [50.0, 50.0]])]),
        ("polygon-flat", "A Polygonal Lasso with no area deselects", vec![rect(), polygon(&[[5.0, 20.0], [30.0, 20.0], [50.0, 20.0]])]),
        ("freehand", "A freehand Lasso loop", vec![lasso("Freehand", &wobble)]),
        ("freehand-aliased", "A freehand Lasso loop without anti-aliasing", vec![aliased(lasso("Freehand", &wobble))]),
        ("freehand-close-points", "A freehand Lasso whose points under a quarter pixel apart are skipped", vec![lasso("Freehand", &[[10.0, 10.0], [10.1, 10.1], [40.0, 12.0], [40.2, 12.1], [52.5, 50.25], [52.6, 50.3], [12.0, 44.0]])]),
        // Add and Subtract.
        ("add-rects", "Two overlapping rectangles added", vec![rect(), add(marquee("Rectangle", [30.0, 25.0], [60.0, 58.0]))]),
        ("add-rect-ellipse", "An ellipse added to a rectangle", vec![rect(), add(marquee("Ellipse", [28.0, 22.0], [62.0, 60.0]))]),
        ("add-disjoint", "Two separate triangles added", vec![polygon(&[[4.5, 4.5], [28.0, 6.0], [10.0, 28.0]]), add(polygon(&[[60.0, 36.5], [58.0, 60.0], [34.5, 58.25]]))]),
        ("add-ellipses", "Two overlapping ellipses added", vec![marquee("Ellipse", [4.0, 8.0], [40.0, 44.0]), add(marquee("Ellipse", [24.0, 20.0], [60.0, 56.0]))]),
        ("subtract-ellipse-from-rect", "An ellipse subtracted from a rectangle", vec![rect(), subtract(marquee("Ellipse", [30.0, 25.0], [60.0, 58.0]))]),
        ("subtract-rect-from-ellipse", "A rectangle subtracted from an ellipse", vec![ellipse(), subtract(marquee("Rectangle", [20.0, 0.0], [40.0, 64.0]))]),
        ("subtract-hole", "A triangle cut out of a rectangle, leaving a hole", vec![marquee("Rectangle", [6.0, 6.0], [58.0, 58.0]), subtract(polygon(&[[20.5, 18.0], [46.0, 30.5], [22.0, 47.25]]))]),
        ("subtract-nothing", "Subtracting with no selection changes nothing", vec![subtract(rect())]),
        ("subtract-all", "Subtracting all of the selection leaves an empty selection", vec![rect(), subtract(marquee("Rectangle", [0.0, 0.0], [64.0, 64.0]))]),
        ("add-subtract-chain", "Add, add, subtract", vec![rect(), add(ellipse()), subtract(triangle())]),
        ("add-aliased-ellipse", "A hard-edged ellipse added to an anti-aliased one", vec![ellipse(), aliased(add(marquee("Ellipse", [30.0, 2.0], [62.0, 30.0])))]),
        // Select menu.
        ("select-all", "Select > All", vec![json!({ "op": "selectAll" })]),
        ("deselect", "Select > Deselect after a marquee", vec![rect(), json!({ "op": "deselect" })]),
        ("invert-rect", "Select > Inverse of a rectangle", vec![rect(), json!({ "op": "invertSelection" })]),
        ("invert-ellipse", "Select > Inverse of an anti-aliased ellipse", vec![ellipse(), json!({ "op": "invertSelection" })]),
        ("invert-triangle-aliased", "Select > Inverse of a hard-edged triangle", vec![aliased(triangle()), json!({ "op": "invertSelection" })]),
        ("invert-all", "Select > Inverse of everything leaves no selection", vec![json!({ "op": "selectAll" }), json!({ "op": "invertSelection" })]),
        ("invert-none", "Select > Inverse without a selection does nothing", vec![json!({ "op": "invertSelection" })]),
        ("invert-twice", "Select > Inverse twice", vec![ellipse(), json!({ "op": "invertSelection" }), json!({ "op": "invertSelection" })]),
        // Modify.
        ("expand-rect", "Expand a rectangle by 3 (rounded corners)", vec![rect(), modify("expand", 3)]),
        ("contract-rect", "Contract a rectangle by 3", vec![rect(), modify("contract", 3)]),
        ("expand-ellipse", "Expand an ellipse by 2", vec![ellipse(), modify("expand", 2)]),
        ("contract-triangle", "Contract a triangle by 2", vec![triangle(), modify("contract", 2)]),
        ("expand-past-canvas", "Expand a rectangle near the edge by 10, clipped to the canvas", vec![marquee("Rectangle", [2.0, 40.0], [30.0, 60.0]), modify("expand", 10)]),
        ("contract-canvas-edge", "Contract Select All by 4, away from the canvas edges too", vec![json!({ "op": "selectAll" }), modify("contract", 4)]),
        ("contract-to-nothing", "Contract past the middle leaves an empty selection", vec![marquee("Rectangle", [20.0, 20.0], [36.0, 30.0]), modify("contract", 6)]),
        ("expand-large", "Expand a small triangle by 20", vec![polygon(&[[28.0, 28.0], [36.5, 30.0], [30.0, 37.0]]), modify("expand", 20)]),
        ("feather-rect", "Feather a rectangle by 4", vec![rect(), modify("feather", 4)]),
        ("feather-ellipse", "Feather an ellipse by 2", vec![ellipse(), modify("feather", 2)]),
        ("feather-1", "Feather a rectangle by 1", vec![rect(), modify("feather", 1)]),
        ("feather-large", "Feather a triangle by 12", vec![triangle(), modify("feather", 12)]),
        ("feather-twice", "Feather 3, then 4: the edge softens to 5", vec![rect(), modify("feather", 3), modify("feather", 4)]),
        ("feather-aliased", "Feather a hard-edged triangle by 2", vec![aliased(triangle()), modify("feather", 2)]),
        ("feather-canvas-edge", "Feather a rectangle touching the canvas edge by 6", vec![marquee("Rectangle", [0.0, 0.0], [40.0, 30.0]), modify("feather", 6)]),
        ("feather-invert", "Feather 3, then Inverse", vec![ellipse(), modify("feather", 3), json!({ "op": "invertSelection" })]),
        ("feather-then-add", "A feathered rectangle, then an ellipse added (the new selection is sharp again)", vec![rect(), modify("feather", 3), add(marquee("Ellipse", [30.0, 30.0], [62.0, 62.0]))]),
        ("expand-then-feather", "Expand 2, then Feather 2", vec![triangle(), modify("expand", 2), modify("feather", 2)]),
    ];
    for (case, label, ops) in shapes {
        let mut d = w.doc(F, case, N, N);
        d.image("Photo", images::photo(N, N), spec());
        w.write(F, case, label, d, ops)?;
    }

    // The Magic Wand, on a checkerboard (hard edges, cells that touch only at corners) or the photo
    // (smooth ramps, where Tolerance decides the extent).
    let wand = |x: f64, y: f64| json!({ "op": "wand", "point": [x, y] });
    let tol = |op: Value, t: u32| with(op, "tolerance", json!(t));
    let scattered = |op: Value| with(op, "contiguous", json!(false));
    let wands: Vec<(&str, &str, &str, Vec<Value>)> = vec![
        ("wand-cell", "Magic Wand on one checker cell, contiguous", "checker", vec![tol(wand(12.5, 3.5), 0)]),
        ("wand-cells-global", "Magic Wand, not contiguous: every cell of that color", "checker", vec![scattered(tol(wand(12.5, 3.5), 0))]),
        ("wand-diagonal", "Magic Wand on the white diagonal", "checker", vec![tol(wand(20.2, 20.9), 10)]),
        ("wand-photo-32", "Magic Wand on the photo, Tolerance 32", "photo", vec![wand(20.0, 20.0)]),
        ("wand-photo-8", "Magic Wand on the photo, Tolerance 8", "photo", vec![tol(wand(40.0, 30.0), 8)]),
        ("wand-photo-global", "Magic Wand on the photo, Tolerance 20, not contiguous", "photo", vec![scattered(tol(wand(40.0, 30.0), 20))]),
        ("wand-photo-255", "Magic Wand, Tolerance 255 selects everything", "photo", vec![tol(wand(1.0, 1.0), 255)]),
        ("wand-3x3", "Magic Wand, 3 by 3 Average", "noise", vec![tol(with(wand(30.5, 30.5), "sampleSize", json!("3 by 3 Average")), 60)]),
        ("wand-5x5-corner", "Magic Wand, 5 by 5 Average at the corner (the square is clipped to the image)", "noise", vec![scattered(tol(with(wand(0.5, 63.5), "sampleSize", json!("5 by 5 Average")), 70))]),
        ("wand-5x5-miss", "Magic Wand whose average matches nothing around the click deselects", "noise", vec![json!({ "op": "selectAll" }), tol(with(wand(30.5, 30.5), "sampleSize", json!("5 by 5 Average")), 0)]),
        ("wand-transparent", "Magic Wand on the transparent ground around a disc (alpha is matched too)", "disc", vec![tol(wand(2.0, 2.0), 40)]),
        ("wand-disc", "Magic Wand inside a disc, anti-aliasing off", "disc", vec![aliased(tol(wand(32.0, 32.0), 100))]),
        ("wand-ring", "Magic Wand on a ring: an outline with a hole", "ring", vec![tol(wand(32.0, 10.0), 16)]),
        ("wand-layer-offset", "Magic Wand on a small layer placed at (16, 16): outside it the layer reads as transparent", "offset", vec![tol(wand(4.0, 4.0), 0)]),
        ("wand-all-layers", "Magic Wand reading all layers", "offset", vec![with(tol(wand(20.0, 20.0), 24), "sampleAllLayers", json!(true))]),
        ("wand-add", "Magic Wand adding a second checker color", "checker", vec![tol(wand(3.5, 12.5), 0), add(scattered(tol(wand(12.5, 3.5), 0)))]),
        ("wand-subtract", "Magic Wand subtracting from an ellipse", "checker", vec![ellipse(), subtract(scattered(tol(wand(12.5, 3.5), 0)))]),
        ("wand-expand", "Magic Wand, then Expand 1 around the staircase outline", "checker", vec![tol(wand(12.5, 3.5), 0), modify("expand", 1)]),
        ("wand-contract", "Magic Wand on the photo, then Contract 2", "photo", vec![wand(20.0, 20.0), modify("contract", 2)]),
        ("wand-feather", "Magic Wand on the ring, then Feather 2", "ring", vec![tol(wand(32.0, 10.0), 16), modify("feather", 2)]),
        ("wand-invert", "Magic Wand on the ring, then Inverse", "ring", vec![tol(wand(32.0, 10.0), 16), json!({ "op": "invertSelection" })]),
        ("object", "Object Selection on a disc", "disc", vec![json!({ "op": "objectSelection", "point": [32, 32] })]),
    ];
    for (case, label, input, ops) in wands {
        let mut d = w.doc(F, case, N, N);
        match input {
            "checker" => {
                d.image("Checker", images::checker(N, N, 8), spec());
            }
            "photo" => {
                d.image("Photo", images::photo(N, N), spec());
            }
            "noise" => {
                d.image("Noise", images::noise(N, N, 31, Alpha::Varied), spec());
            }
            "disc" => {
                d.image("Photo", images::photo(N, N), spec());
                d.image("Disc", images::disc(N, N, [250, 200, 20]), spec());
            }
            "ring" => {
                d.image("Ring", ring(N), spec());
            }
            _ => {
                d.image("Photo", images::photo(N, N), spec());
                d.image("Small", images::checker(32, 32, 4), LayerSpec { transform: Some(Transform::at(16.0, 16.0, 32.0, 32.0)), ..spec() });
            }
        }
        w.write(F, case, label, d, ops)?;
    }

    // Probes for Core Graphics' fill: combs whose teeth have edges at many fractions of a pixel,
    // and long edges at several slopes. The combs sit on a 1/1024 grid, as the rasterizer's
    // arithmetic is binary.
    let comb = |horizontal: bool, fractions: &dyn Fn(usize) -> (f64, f64)| {
        let mut points: Vec<[f64; 2]> = Vec::new();
        let (f0, _) = fractions(0);
        points.push([f0, 60.0]);
        for i in 0..31 {
            let (f, g) = fractions(i);
            let (left, right) = (2.0 * i as f64 + f, 2.0 * i as f64 + 1.0 + g);
            if i > 0 {
                points.push([left, 44.0]);
            }
            points.push([left, 4.0]);
            points.push([right, 4.0]);
            points.push([right, if i == 30 { 60.0 } else { 44.0 }]);
        }
        let points: Vec<[f64; 2]> = if horizontal { points.iter().map(|p| [p[1], p[0]]).collect() } else { points };
        polygon(&points)
    };
    let grid = |v: f64| (v * 1024.0).round() / 1024.0;
    let spread = |i: usize| (grid((i as f64 + 0.37) / 31.0), grid((i as f64 + 0.71) / 31.0));
    // Each tooth's left edge leaves its pixel a sliver of 1…31/1024; the right edge covers half.
    let slivers = |i: usize| (1.0 - (i + 1) as f64 / 1024.0, 0.5);
    let slope = |dx_dy: f64, top: f64, height: f64, x: f64| polygon(&[[x, top], [62.0, top], [62.0, top + height], [x + dx_dy * height, top + height]]);
    let ellipse_then = |second: Value| vec![ellipse(), second];
    let probes: Vec<(&str, &str, Vec<Value>)> = vec![
        ("probe-comb-x", "Probe: vertical edges at 62 fractions of a pixel", vec![comb(false, &spread)]),
        ("probe-comb-y", "Probe: horizontal edges at 62 fractions of a pixel", vec![comb(true, &spread)]),
        ("probe-comb-x-tiny", "Probe: vertical edges leaving slivers of 1/1024 to 31/1024 px", vec![comb(false, &slivers)]),
        ("probe-comb-x-tiny-aliased", "Probe: vertical edges leaving slivers of 1/1024 to 31/1024 px, aliased", vec![aliased(comb(false, &slivers))]),
        ("probe-slope-0.0311", "Probe: an edge 0.0311 px across per row", vec![slope(0.0311, 2.3, 59.0, 3.4)]),
        ("probe-slope-0.37", "Probe: an edge 0.37 px across per row", vec![slope(0.37, 2.3, 59.0, 3.4)]),
        ("probe-slope-1.6", "Probe: an edge 1.6 px across per row", vec![slope(1.6, 10.0, 35.0, 1.2)]),
        ("probe-slope-7.3", "Probe: an edge 7.3 px across per row", vec![slope(7.3, 20.2, 7.5, 2.1)]),
        ("probe-slope-back", "Probe: an edge -0.23 px across per row", vec![slope(-0.23, 1.1, 60.0, 17.9)]),
        ("probe-ellipse-subtract-far", "Probe: an ellipse, less a rectangle that doesn't touch it", ellipse_then(subtract(marquee("Rectangle", [58.0, 56.0], [62.0, 62.0])))),
        ("probe-ellipse-add-far", "Probe: an ellipse, plus a rectangle that doesn't touch it", ellipse_then(add(marquee("Rectangle", [58.0, 56.0], [62.0, 62.0])))),
        ("probe-ellipse-add-self", "Probe: an ellipse added to itself", ellipse_then(add(ellipse()))),
        ("probe-ellipse-cut-center", "Probe: an ellipse, less the rectangle below its middle", ellipse_then(subtract(marquee("Rectangle", [0.0, 30.0], [64.0, 64.0])))),
        ("probe-ellipse-cut-right", "Probe: an ellipse, less the rectangle right of x = 45", ellipse_then(subtract(marquee("Rectangle", [45.0, 0.0], [64.0, 64.0])))),
        ("probe-circle-cut-thin", "Probe: a circle, less a one-pixel column through it", vec![marquee("Ellipse", [8.0, 8.0], [56.0, 56.0]), subtract(marquee("Rectangle", [31.0, 0.0], [32.0, 64.0]))]),
        ("probe-ellipse-large", "Probe: an ellipse larger than the canvas, inside it at the corners", vec![marquee("Ellipse", [-6.0, -4.0], [70.0, 68.0])]),
        // Long shallow edges whose 1/16 px steps round well away from their slope (Core Graphics
        // truncates each step), meeting at vertices inside pixels.
        ("probe-vertex-right", "Probe: two long shallow edges meeting at a vertex on the right", vec![polygon(&[[2.3, 2.4286], [61.4, 31.3], [2.3, 52.9571]])]),
        ("probe-vertex-left", "Probe: two long shallow edges meeting at a vertex on the left", vec![polygon(&[[61.7, 3.8286], [61.7, 54.3571], [2.6, 32.7]])]),
        ("probe-vertex-bottom", "Probe: two long steep edges meeting at a vertex at the bottom", vec![polygon(&[[2.43, 2.3], [52.96, 2.3], [31.3, 61.4]])]),
        ("probe-edge-long-up", "Probe: one long shallow edge between two vertical ones", vec![polygon(&[[1.5, 20.3], [62.5, 29.25], [62.5, 60.0], [1.5, 60.0]])]),
        ("probe-edge-long-down", "Probe: one long shallow edge the other way", vec![polygon(&[[1.5, 29.25], [62.5, 20.3], [62.5, 60.0], [1.5, 60.0]])]),
        // Inside corners after Contract, and bands that cut each other's round joins.
        ("probe-contract-l", "Probe: an L contracted by 3 (a whole round inside corner)", vec![polygon(&[[6.0, 6.0], [30.0, 6.0], [30.0, 30.0], [58.0, 30.0], [58.0, 58.0], [6.0, 58.0]]), modify("contract", 3)]),
        ("probe-contract-step1", "Probe: a one-pixel step contracted by 2 (the corner's arc cut by the next edge's band)", vec![polygon(&[[20.0, 8.0], [26.0, 8.0], [26.0, 50.0], [18.0, 50.0], [18.0, 30.0], [20.0, 30.0]]), modify("contract", 2)]),
        ("probe-contract-step3", "Probe: a three-pixel step contracted by 2", vec![polygon(&[[20.0, 8.0], [26.0, 8.0], [26.0, 50.0], [15.0, 50.0], [15.0, 30.0], [20.0, 30.0]]), modify("contract", 2)]),
        ("probe-expand-gap", "Probe: a C with a 3 px gap expanded by 2 (round joins cut by each other)", vec![polygon(&[[10.0, 10.0], [54.0, 10.0], [54.0, 20.0], [20.0, 20.0], [20.0, 23.0], [54.0, 23.0], [54.0, 54.0], [10.0, 54.0]]), modify("expand", 2)]),
        // A curve cut, then its piece cut again.
        ("probe-recut-strips", "Probe: an ellipse less a strip along the top, then less a strip on the left", vec![ellipse(), subtract(marquee("Rectangle", [0.0, 0.0], [64.0, 14.0])), subtract(marquee("Rectangle", [0.0, 0.0], [26.0, 64.0]))]),
        ("probe-recut-left", "Probe: an ellipse less the strip on the left only", vec![ellipse(), subtract(marquee("Rectangle", [0.0, 0.0], [26.0, 64.0]))]),
        ("probe-recut-top", "Probe: an ellipse less the strip along the top only", vec![ellipse(), subtract(marquee("Rectangle", [0.0, 0.0], [64.0, 14.0]))]),
        ("probe-recut-union", "Probe: a rectangle plus an ellipse, less a strip on the left", vec![marquee("Rectangle", [20.0, 20.0], [44.0, 44.0]), add(ellipse()), subtract(marquee("Rectangle", [0.0, 0.0], [30.0, 64.0]))]),
        ("probe-recut-diagonal", "Probe: an ellipse less a strip along the top, then less a triangle", vec![ellipse(), subtract(marquee("Rectangle", [0.0, 0.0], [64.0, 14.0])), subtract(polygon(&[[0.0, 0.0], [40.0, 0.0], [0.0, 40.0]]))]),
        // Expand and Contract on curves.
        ("probe-expand-circle", "Probe: a circle expanded by 3", vec![marquee("Ellipse", [16.0, 16.0], [48.0, 48.0]), modify("expand", 3)]),
        ("probe-contract-ellipse", "Probe: an ellipse contracted by 2", vec![ellipse(), modify("contract", 2)]),
        ("probe-expand-small-ellipse", "Probe: a small ellipse expanded by 4", vec![marquee("Ellipse", [28.0, 28.0], [36.0, 34.0]), modify("expand", 4)]),
    ];
    for (case, label, ops) in probes {
        let mut d = w.doc(F, case, N, N);
        d.image("Photo", images::photo(N, N), spec());
        w.write(F, case, label, d, ops)?;
    }

    // Select > Color Range, which reads every visible layer.
    let range = |samples: Value| json!({ "op": "colorRange", "samples": samples });
    let ranges: Vec<(&str, &str, Value)> = vec![
        ("range", "Color Range, one color, Fuzziness 40", range(json!([{ "point": [20, 20] }]))),
        ("range-add", "Color Range, two colors", range(json!([{ "point": [20, 20] }, { "point": [50, 40], "mode": "Add" }]))),
        ("range-remove", "Color Range, a color taken away", with(range(json!([{ "point": [20, 20] }, { "point": [24, 22], "mode": "Remove" }])), "fuzziness", json!(80))),
        ("range-replace", "Color Range, a second click starts over", range(json!([{ "point": [20, 20] }, { "point": [50, 60] }]))),
        ("range-invert", "Color Range, inverted", with(range(json!([{ "point": [20, 20] }])), "invert", json!(true))),
        ("range-fuzziness-0", "Color Range, Fuzziness 0", with(range(json!([{ "point": [33, 61] }])), "fuzziness", json!(0))),
        ("range-fuzziness-200", "Color Range, Fuzziness 200", with(range(json!([{ "point": [5, 5] }])), "fuzziness", json!(200))),
        ("range-edge", "Color Range sampled at the corner (the 3 by 3 average reaches past the image)", with(range(json!([{ "point": [63.5, 0.5] }])), "fuzziness", json!(30))),
        ("range-translucent", "Color Range on a disc over transparency (transparent pixels never match)", range(json!([{ "point": [32, 32] }]))),
        ("range-nothing", "Color Range picking only transparency keeps no selection", range(json!([{ "point": [1, 1] }]))),
    ];
    for (case, label, op) in ranges {
        let mut d = w.doc(F, case, N, N);
        if matches!(case, "range-translucent" | "range-nothing") {
            d.image("Disc", images::disc(N, N, [40, 160, 230]), spec());
        } else {
            d.image("Photo", images::photo(N, N), spec());
            d.image("Screen", images::noise(N, N, 32, Alpha::Varied), LayerSpec { opacity: Some(0.3), ..spec() });
        }
        w.write(F, case, label, d, vec![op])?;
    }

    // Loading a layer's pixels or its mask's black areas (Cmd-click on the thumbnails).
    let loads: Vec<(&str, &str)> = vec![
        ("load-pixels", "A layer's pixels (at least half opaque) as the selection"),
        ("load-pixels-offset", "The pixels of a layer that runs past the canvas"),
        ("load-mask", "A mask's black areas as the selection"),
        ("load-add", "A layer's pixels added to a rectangle"),
        ("load-subtract", "A mask's black areas subtracted from Select All"),
        ("load-then-feather", "A layer's pixels, then Feather 3"),
    ];
    for (case, label) in loads {
        let mut d = w.doc(F, case, N, N);
        d.image("Photo", images::photo(N, N), spec());
        let placed = if case == "load-pixels-offset" { Some(Transform::at(-20.0, 30.0, 64.0, 64.0)) } else { None };
        let noise = d.image("Noise", images::noise(N, N, 33, Alpha::Varied), LayerSpec { transform: placed, ..spec() });
        let masked = d.image("Masked", images::disc(N, N, [200, 60, 90]), spec());
        d.mask(&masked, images::mask_mixed(N, N));
        let load = |layer: &str, mask: bool| json!({ "op": "loadSelection", "layer": layer, "mask": mask });
        let ops = match case {
            "load-mask" => vec![load(&masked, true)],
            "load-add" => vec![rect(), add(load(&masked, false))],
            "load-subtract" => vec![json!({ "op": "selectAll" }), subtract(load(&masked, true))],
            "load-then-feather" => vec![load(&masked, false), modify("feather", 3)],
            _ => vec![load(&noise, false)],
        };
        w.write(F, case, label, d, ops)?;
    }
    Ok(())
}

/// A ring of one color with a soft gradient inside it, on a flat ground: the Wand's outline has
/// an outer loop and a hole.
fn ring(n: u32) -> RgbaImage {
    RgbaImage::from_fn(n, n, |x, y| {
        let d = ((x as f64 + 0.5 - 32.0).powi(2) + (y as f64 + 0.5 - 32.0).powi(2)).sqrt();
        if (14.0..24.0).contains(&d) {
            image::Rgba([200, 40 + (x % 4) as u8 * 3, 60, 255])
        } else if d < 14.0 {
            image::Rgba([20 + (d * 8.0) as u8, 120, 200, 255])
        } else {
            image::Rgba([240, 235, 220, 255])
        }
    })
}
