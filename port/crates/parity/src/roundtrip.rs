//! .comp round trips: a project the Mac app saved, opened and saved again by the port, must come
//! back identical, and the port's manifest bytes should match the Mac's.

use crate::compare::{CaseResult, Limit, Status};
use anyhow::{Context, Result};
use std::path::Path;

/// Checks the Mac-saved project `refs/<id>.comp`, if the harness wrote one for this case.
pub fn check(id: &str, refs: &Path, scratch: &Path) -> Option<CaseResult> {
    let mac = refs.join(format!("{id}.comp"));
    if !mac.is_dir() {
        return None;
    }
    let mut result = CaseResult {
        id: format!("format/{id}"),
        feature: "format".into(),
        label: format!("Round trip of the project the Mac saved for {id}"),
        status: Status::Fail,
        tolerance: 0,
        max_channel_diff: None,
        differing_pixels: None,
        total_pixels: None,
        message: None,
        heatmap: None,
    };
    match round_trip(&mac, &scratch.join(format!("{}.comp", id.replace('/', "__")))) {
        Ok(note) => {
            result.status = Status::Pass;
            result.message = note;
        }
        Err(e) => result.message = Some(format!("{e:#}")),
    }
    Some(result)
}

fn round_trip(mac: &Path, out: &Path) -> Result<Option<String>> {
    let original = comp_format::load(mac)?;
    // Save from decoded pixels, not the Mac's PNG bytes, so the port's own encoder is exercised.
    let mut copy = original.clone();
    for asset in copy.images.values_mut() {
        asset.png = None;
    }
    for asset in copy.masks.values_mut() {
        asset.png = None;
    }
    comp_format::save(&copy, out)?;
    let reloaded = comp_format::load(out)?;
    anyhow::ensure!(reloaded.manifest == original.manifest, "the manifest changed on the way through");
    for (id, asset) in &original.images {
        anyhow::ensure!(reloaded.images.get(id).is_some_and(|a| a.pixels == asset.pixels), "layer {id} pixels changed");
    }
    for (id, asset) in &original.masks {
        anyhow::ensure!(reloaded.masks.get(id).is_some_and(|a| a.pixels == asset.pixels), "mask {id} pixels changed");
    }
    let mac_bytes = std::fs::read(mac.join("manifest.json"))?;
    let port_bytes = std::fs::read(out.join("manifest.json"))?;
    Ok((mac_bytes != port_bytes).then(|| "identical project; manifest bytes differ from the Mac's in formatting only".to_string()))
}

/// Compares the project the port made for a case (after its ops or PSD import) with the one the
/// Mac saved, `refs/<id>.comp`: the manifests as the Mac writes them, with layer and document IDs
/// replaced by their positions (new layers get random IDs on both sides), and every layer and mask
/// pixel. Layer and mask pixels are held to the case's own tolerance (`limit`), the same as its
/// render, or to its override's `structure_max_channel_diff` (a mask a stand-in model made, for
/// example); text layers' gap is also measured and reported, since their rasterizer isn't Core Text's.
pub fn check_structure(id: &str, feature: &str, port: &comp_format::Project, refs: &Path, limit: Limit) -> Option<CaseResult> {
    let mac = refs.join(format!("{id}.comp"));
    if !mac.is_dir() {
        return None;
    }
    let mut result = CaseResult {
        id: format!("{id}#structure"),
        feature: feature.into(),
        label: format!("The project after {id} matches the one the Mac saved"),
        status: Status::Fail,
        tolerance: limit.max_channel_diff,
        max_channel_diff: None,
        differing_pixels: None,
        total_pixels: None,
        message: None,
        heatmap: None,
    };
    let port = engine::session::normalize(port);
    let mut text = None;
    match compare_projects(&port, &mac, limit, &mut text) {
        Ok(()) => result.status = Status::Pass,
        Err(e) => {
            result.message = Some(format!("{e:#}"));
            // PARITY_DUMP_STRUCTURE=<dir> saves the port's side of a failing check for diffing.
            if let Some(dir) = std::env::var_os("PARITY_DUMP_STRUCTURE") {
                let path = Path::new(&dir).join(format!("{}.comp", id.replace('/', "__")));
                let _ = comp_format::save(&port, &path);
            }
        }
    }
    if let Some(text) = text {
        result.tolerance = limit.max_channel_diff;
        result.max_channel_diff = Some(text.max_channel_diff);
        result.differing_pixels = Some(text.differing_pixels);
        result.total_pixels = Some(text.total_pixels);
    }
    Some(result)
}

fn canonical(project: &comp_format::Project) -> Result<serde_json::Value> {
    let index: std::collections::HashMap<&str, usize> =
        project.manifest.layers.iter().enumerate().map(|(i, l)| (l.id.as_str(), i)).collect();
    let mut value = serde_json::to_value(&project.manifest)?;
    value["documentID"] = serde_json::Value::Null;
    let rename = |v: &mut serde_json::Value, suffix: &str| {
        if let Some(s) = v.as_str() {
            let key = s.trim_end_matches(suffix);
            if let Some(i) = index.get(key) {
                *v = serde_json::Value::String(format!("layer {i}{suffix}"));
            }
        }
    };
    rename(&mut value["activeLayerID"], "");
    for layer in value["layers"].as_array_mut().into_iter().flatten() {
        for key in ["id", "parentID", "maskSourceID"] {
            rename(&mut layer[key], "");
        }
        rename(&mut layer["imageFile"], ".png");
        rename(&mut layer["maskFile"], ".mask.png");
    }
    // Swift writes a [ColorRange: …] dictionary as a flat key, value, key, value… array in its
    // dictionary's order, which changes from run to run, so the pairs are compared sorted.
    for layer in value["layers"].as_array_mut().into_iter().flatten() {
        let hsv = &mut layer["adjustment"]["hsvSettings"];
        for key in ["adjustments", "bands"] {
            if let Some(flat) = hsv[key].as_array() {
                let mut pairs: Vec<(String, serde_json::Value)> =
                    flat.chunks(2).filter(|p| p.len() == 2).map(|p| (p[0].to_string(), p[1].clone())).collect();
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                hsv[key] = serde_json::Value::Array(pairs.into_iter().flat_map(|(k, v)| [serde_json::from_str(&k).unwrap(), v]).collect());
            }
        }
    }
    if let Some(guides) = value["guides"].as_array_mut() {
        for g in guides {
            g["id"] = serde_json::Value::Null;
        }
    }
    Ok(value)
}

/// How far the text layers' pixels are from the Mac's, over all of them.
pub struct TextGap {
    pub max_channel_diff: u8,
    pub differing_pixels: u64,
    pub total_pixels: u64,
}

fn compare_projects(port: &comp_format::Project, mac_path: &Path, limit: Limit, text: &mut Option<TextGap>) -> Result<()> {
    let mac = comp_format::load(mac_path)?;
    let (a, b) = (canonical(port)?, canonical(&mac)?);
    if a != b {
        let (pa, pb) = (serde_json::to_string_pretty(&a)?, serde_json::to_string_pretty(&b)?);
        let first = pa.lines().zip(pb.lines()).enumerate().find(|(_, (x, y))| x != y);
        anyhow::bail!(
            "manifest differs{}",
            first.map(|(n, (x, y))| format!(" at line {}: port `{}`, Mac `{}`", n + 1, x.trim(), y.trim())).unwrap_or_default()
        );
    }
    for (i, (pl, ml)) in port.manifest.layers.iter().zip(&mac.manifest.layers).enumerate() {
        let same_image = match (port.images.get(&pl.id), mac.images.get(&ml.id)) {
            (Some(p), Some(m)) if ml.text.is_some() => {
                let d = crate::compare::diff(&m.pixels, &p.pixels, limit.max_channel_diff)
                    .with_context(|| format!("text layer {i} ({})", pl.name))?;
                let gap = text.get_or_insert(TextGap { max_channel_diff: 0, differing_pixels: 0, total_pixels: 0 });
                gap.max_channel_diff = gap.max_channel_diff.max(d.max_channel_diff);
                gap.differing_pixels += d.differing_pixels;
                gap.total_pixels += d.total_pixels;
                d.differing_pixels <= limit.max_pixels_over
            }
            (Some(p), Some(m)) => crate::compare::diff(&m.pixels, &p.pixels, limit.max_channel_diff)
                .is_ok_and(|d| d.differing_pixels <= limit.max_pixels_over),
            (None, None) => true,
            _ => false,
        };
        anyhow::ensure!(same_image, "layer {i} ({}) pixels differ", pl.name);
        let same_mask = match (port.masks.get(&pl.id), mac.masks.get(&ml.id)) {
            (Some(p), Some(m)) => match Some(&limit) {
                None => p.pixels == m.pixels,
                Some(limit) => {
                    p.pixels.dimensions() == m.pixels.dimensions()
                        && p.pixels.as_raw().iter().zip(m.pixels.as_raw()).filter(|(a, b)| a.abs_diff(**b) > limit.max_channel_diff).count() as u64
                            <= limit.max_pixels_over
                }
            },
            (None, None) => true,
            _ => false,
        };
        anyhow::ensure!(same_mask, "layer {i} ({}) mask differs", pl.name);
    }
    Ok(())
}
