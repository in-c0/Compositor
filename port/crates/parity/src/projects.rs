//! `parity compare-projects`: the project the port imports from each Photoshop case against the
//! `.comp` the Mac app saved for it. Manifests are compared field by field, with layers matched by
//! position and their IDs (random on the Mac) replaced by that position; layer and mask pixels are
//! compared byte for byte; the conversions the Mac listed are compared with the port's; and files
//! the Mac refused must be refused.

use crate::cases::{self, Case};
use anyhow::{Result, bail};
use comp_format::Project;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

pub fn compare_projects(corpus: &Path, refs: &Path, patterns: &[String]) -> Result<()> {
    let cases: Vec<Case> = cases::select(cases::discover(corpus)?, patterns)?
        .into_iter()
        .filter(|c| {
            let input = c.spec.input.to_ascii_lowercase();
            (input.ends_with(".psd") || input.ends_with(".psb")) && c.spec.ops.is_empty()
        })
        .collect();
    if cases.is_empty() {
        bail!("no Photoshop cases match");
    }
    let info: Value = std::fs::read(refs.join("harness-info.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let by_id: HashMap<&str, &Value> =
        info["cases"].as_array().into_iter().flatten().filter_map(|c| Some((c["id"].as_str()?, c))).collect();
    let (mut same, mut pending) = (0, 0);
    let mut different = Vec::new();
    for case in &cases {
        let Some(mac) = by_id.get(case.id.as_str()) else {
            different.push((case.id.clone(), vec!["no entry in harness-info.json".to_string()]));
            continue;
        };
        let imported = psd::import_file(&case.input_path());
        let problems = match (mac["status"].as_str(), imported) {
            (Some("error"), Err(psd::ImportError::NotPorted(what))) => {
                println!("pending  {}: {what}", case.id);
                pending += 1;
                continue;
            }
            (Some("error"), Err(e)) => {
                let (mac_error, port_error) = (mac["error"].as_str().unwrap_or(""), format!("{}: {e}", case.spec.input));
                if mac_error == port_error { vec![] } else { vec![format!("both refuse it, with different messages. Mac: {mac_error} Port: {port_error}")] }
            }
            (Some("error"), Ok(_)) => vec![format!("the Mac refuses this file ({}) but the port imported it", mac["error"].as_str().unwrap_or(""))],
            (_, Err(psd::ImportError::NotPorted(what))) => {
                println!("pending  {}: {what}", case.id);
                pending += 1;
                continue;
            }
            (_, Err(e)) => vec![format!("the port refuses it: {e}")],
            (_, Ok(imported)) => match comp_format::load(&refs.join(format!("{}.comp", case.id))) {
                Ok(reference) => {
                    let mut problems = compare(&reference, &imported.project);
                    let notes: Vec<String> = mac["notes"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(String::from)).collect();
                    let ours: Vec<String> = imported
                        .conversions
                        .iter()
                        .map(|c| format!("PSD conversion accepted, layer \u{201c}{}\u{201d}: {}", c.layer_name, c.message))
                        .collect();
                    if notes != ours {
                        problems.push(format!("conversions differ.\n      Mac:  {notes:?}\n      Port: {ours:?}"));
                    }
                    problems
                }
                Err(e) => vec![format!("reading the Mac's project: {e:#}")],
            },
        };
        if problems.is_empty() {
            println!("same     {}", case.id);
            same += 1;
        } else {
            println!("DIFFERS  {}", case.id);
            for p in &problems {
                println!("    {p}");
            }
            different.push((case.id.clone(), problems));
        }
    }
    println!("\n{same} of {} Photoshop cases match the Mac's projects, {pending} pending, {} differ.", cases.len(), different.len());
    if !different.is_empty() {
        bail!("{} cases differ", different.len());
    }
    Ok(())
}

/// Every difference between the Mac's project and the port's.
pub fn compare(mac: &Project, port: &Project) -> Vec<String> {
    let mut problems = Vec::new();
    let (a, b) = (&mac.manifest.layers, &port.manifest.layers);
    if a.len() != b.len() {
        problems.push(format!("{} layers on the Mac, {} in the port", a.len(), b.len()));
    }
    let mac_json = normalized(mac);
    let port_json = normalized(port);
    diff("", &mac_json, &port_json, &mut problems);
    for (i, (ma, pa)) in a.iter().zip(b).enumerate() {
        match (mac.images.get(&ma.id), port.images.get(&pa.id)) {
            (Some(m), Some(p)) => pixels(&format!("layer {i} pixels"), m.pixels.dimensions(), m.pixels.as_raw(), p.pixels.dimensions(), p.pixels.as_raw(), &mut problems),
            (None, None) => {}
            (m, _) => problems.push(format!("layer {i}: pixels only in the {}", if m.is_some() { "Mac's" } else { "port's" })),
        }
        match (mac.masks.get(&ma.id), port.masks.get(&pa.id)) {
            (Some(m), Some(p)) => pixels(&format!("layer {i} mask"), m.pixels.dimensions(), m.pixels.as_raw(), p.pixels.dimensions(), p.pixels.as_raw(), &mut problems),
            (None, None) => {}
            (m, _) => problems.push(format!("layer {i}: a mask only in the {}", if m.is_some() { "Mac's" } else { "port's" })),
        }
    }
    problems
}

fn pixels(what: &str, mac_size: (u32, u32), mac: &[u8], port_size: (u32, u32), port: &[u8], problems: &mut Vec<String>) {
    if mac_size != port_size {
        problems.push(format!("{what}: {mac_size:?} on the Mac, {port_size:?} in the port"));
        return;
    }
    let differing = mac.iter().zip(port).filter(|(a, b)| a != b).count();
    if differing > 0 {
        let max = mac.iter().zip(port).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        problems.push(format!("{what}: {differing} of {} bytes differ, by up to {max}", mac.len()));
    }
}

/// The manifest as JSON, with document and layer IDs replaced by layer positions and Swift's
/// unordered `[ColorRange: V]` dictionaries sorted.
fn normalized(project: &Project) -> Value {
    let ids: HashMap<String, String> =
        project.manifest.layers.iter().enumerate().map(|(i, l)| (l.id.clone(), format!("layer {i}"))).collect();
    let mut value = serde_json::to_value(&project.manifest).expect("manifest to JSON");
    let root = value.as_object_mut().unwrap();
    root.remove("documentID");
    if let Some(Value::String(active)) = root.get_mut("activeLayerID") {
        *active = ids.get(active.as_str()).cloned().unwrap_or_else(|| format!("unknown {active}"));
    }
    for layer in root.get_mut("layers").and_then(|l| l.as_array_mut()).into_iter().flatten() {
        let layer = layer.as_object_mut().unwrap();
        for key in ["id", "parentID", "maskSourceID"] {
            if let Some(Value::String(id)) = layer.get_mut(key) {
                *id = ids.get(id.as_str()).cloned().unwrap_or_else(|| format!("unknown {id}"));
            }
        }
        for (key, suffix) in [("imageFile", ".png"), ("maskFile", ".mask.png")] {
            if let Some(Value::String(file)) = layer.get_mut(key) {
                let id = file.strip_suffix(suffix).unwrap_or(file);
                *file = format!("{}{suffix}", ids.get(id).cloned().unwrap_or_else(|| format!("unknown {id}")));
            }
        }
        if let Some(settings) = layer.get_mut("adjustment").and_then(|a| a.get_mut("hsvSettings")).and_then(|h| h.as_object_mut()) {
            for key in ["adjustments", "bands"] {
                if let Some(Value::Array(flat)) = settings.get(key) {
                    let map: Map<String, Value> = flat.chunks(2).map(|p| (p[0].as_str().unwrap_or("?").to_string(), p[1].clone())).collect();
                    settings.insert(key.into(), Value::Object(map));
                }
            }
        }
    }
    value
}

fn diff(path: &str, mac: &Value, port: &Value, problems: &mut Vec<String>) {
    match (mac, port) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => diff(&child, x, y, problems),
                    (Some(x), None) => problems.push(format!("{child}: {x} on the Mac, absent in the port")),
                    (None, Some(y)) => problems.push(format!("{child}: absent on the Mac, {y} in the port")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                problems.push(format!("{path}: {} items on the Mac, {} in the port", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                diff(&format!("{path}[{i}]"), x, y, problems);
            }
        }
        // Numbers compare by value: the Mac writes `1` where serde writes `1.0`.
        (Value::Number(a), Value::Number(b)) if a.as_f64() == b.as_f64() => {}
        (a, b) if a == b => {}
        (a, b) => problems.push(format!("{path}: {a} on the Mac, {b} in the port")),
    }
}
