//! .comp round trips: a project the Mac app saved, opened and saved again by the port, must come
//! back identical, and the port's manifest bytes should match the Mac's.

use crate::compare::{CaseResult, Status};
use anyhow::Result;
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
