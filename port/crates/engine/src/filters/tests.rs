use super::*;
use std::path::{Path, PathBuf};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../parity/corpus")
}

#[test]
fn settings_patch_nested_fields_onto_defaults() {
    let json = serde_json::json!({ "colorBalance": { "midCyanRed": 50 }, "vignetteColor": { "red": 0.8 }, "radius": 3 });
    let s = FilterSettings::from_json(Some(&json)).unwrap();
    assert_eq!(s.color_balance.mid_cyan_red, 50.0);
    assert!(s.color_balance.preserve_luminosity);
    assert_eq!((s.vignette_color.red, s.vignette_color.green), (0.8, 0.0));
    assert_eq!(s.radius, 3.0);
    assert!(FilterSettings::from_json(Some(&serde_json::json!({ "radios": 3 }))).is_err());
    assert!(FilterSettings::from_json(Some(&serde_json::json!({ "grain": { "amont": 3 } }))).is_err());
}

#[test]
fn grow_and_trim_place_the_new_grid() {
    let mut pixels = image::RgbaImage::new(4, 2);
    pixels.put_pixel(1, 1, image::Rgba([9, 9, 9, 255]));
    let t = Transform::at(10.0, 20.0, 4.0, 2.0);
    let (grid, grown) = grow(&pixels, &t, 3).unwrap();
    assert_eq!(grid.dimensions(), (10, 8));
    assert_eq!((grown.origin, grown.size), ([7.0, 17.0], [10.0, 8.0]));
    let (trimmed, placed) = trimmed(grid, &grown);
    assert_eq!(trimmed.dimensions(), (1, 1));
    assert_eq!((placed.origin, placed.size), ([11.0, 21.0], [1.0, 1.0]));
}

/// Every filter case, compared layer by layer with the project the Mac saved after the op
/// (`<case>.comp` in the references): the filtered layer's placement, size and pixels. Set
/// `PARITY_REFS` to the references folder (the one holding `filters/<case>.comp`); `FILTER_CASES`
/// narrows the cases by substring. Prints each case and fails when a supported case's pixels are
/// off by more than 1/255 or its layer is placed differently.
#[test]
fn filter_cases_match_the_mac_projects() {
    let Some(refs) = std::env::var_os("PARITY_REFS").map(PathBuf::from) else {
        eprintln!("PARITY_REFS not set; skipping");
        return;
    };
    let filter = std::env::var("FILTER_CASES").unwrap_or_default();
    // FILTER_INEXACT=1 also runs the filters that aren't exact yet, to measure them.
    let measure = std::env::var_os("FILTER_INEXACT").is_some();
    let gpu = Gpu::new().unwrap();
    let mut failures = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(corpus().join("filters")).unwrap().map(|e| e.unwrap().path()).collect();
    dirs.sort();
    for dir in dirs {
        let id = dir.file_name().unwrap().to_string_lossy().to_string();
        if !id.contains(&filter) {
            continue;
        }
        let spec: Value = serde_json::from_slice(&std::fs::read(dir.join("case.json")).unwrap()).unwrap();
        let Ok(want) = comp_format::load(&refs.join(format!("filters/{id}.comp"))) else { continue };
        let mut project = comp_format::load(&dir.join("input.comp")).unwrap();
        let mut outcome = Ok(());
        for op in spec["ops"].as_array().unwrap() {
            outcome = outcome.and_then(|_| apply_measuring(&gpu, &mut project, op, measure));
        }
        if let Err(e) = outcome {
            println!("{id:<22} {e}");
            continue;
        }
        let layer = spec["ops"][0]["layer"].as_str().unwrap();
        let got_record = project.manifest.layers.iter().find(|l| l.id == layer).unwrap();
        let want_record = want.manifest.layers.iter().find(|l| l.id == layer).unwrap();
        let placed = got_record.transform == want_record.transform;
        let got = &project.images[layer].pixels;
        let want_pixels = &want.images[layer].pixels;
        if got.dimensions() != want_pixels.dimensions() {
            println!("{id:<22} size {:?}, the Mac's {:?}", got.dimensions(), want_pixels.dimensions());
            failures.push(id);
            continue;
        }
        let (mut max, mut count) = (0u8, 0usize);
        for (g, w) in got.pixels().zip(want_pixels.pixels()) {
            if g[3] == 0 && w[3] == 0 {
                continue;
            }
            let d = (0..4).map(|c| g[c].abs_diff(w[c])).max().unwrap();
            max = max.max(d);
            count += (d != 0) as usize;
        }
        if let Some(want_mask) = want.masks.get(layer) {
            // Remove Background: Vision's mask can't be reproduced; report how far the stand-in is.
            let Some(got_mask) = project.masks.get(layer) else {
                println!("{id:<22} no mask");
                failures.push(id);
                continue;
            };
            let (iou, mad) = mask_difference(&got_mask.pixels, &want_mask.pixels);
            println!("{id:<22} mask IoU {iou:.4}  mean difference {mad:.2}/255  pixels max {max}  placement {placed}");
            if max > 0 || !placed || got_record.mask_enabled() != want_record.mask_enabled() {
                failures.push(id);
            }
            continue;
        }
        let (w, h) = got.dimensions();
        println!(
            "{id:<22} max {max:>3}  differing pixels {count:>5} of {}  placement {}",
            w * h,
            if placed { "matches" } else { "differs" }
        );
        if (max > 1 || !placed) && !measure {
            failures.push(id);
        }
    }
    assert!(failures.is_empty(), "differ from the Mac: {failures:?}");
}

/// Intersection over union of the masks' halves above 50%, and their mean absolute difference.
fn mask_difference(a: &image::GrayImage, b: &image::GrayImage) -> (f64, f64) {
    let (mut inter, mut union, mut total) = (0usize, 0usize, 0f64);
    for (p, q) in a.pixels().zip(b.pixels()) {
        let (p, q) = (p[0], q[0]);
        inter += (p > 127 && q > 127) as usize;
        union += (p > 127 || q > 127) as usize;
        total += p.abs_diff(q) as f64;
    }
    (inter as f64 / union.max(1) as f64, total / (a.width() * a.height()) as f64)
}
