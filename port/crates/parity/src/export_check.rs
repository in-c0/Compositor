//! Export checks: the port's PNG carries the same metadata chunks as the Mac's, and its JPEG
//! decodes to the same pixels (within the case's limit).

use crate::compare::{self, CaseResult, Status, Tolerances};
use anyhow::{Context, Result};
use std::path::Path;

fn result(id: String, feature: &str, label: String) -> CaseResult {
    CaseResult {
        id,
        feature: feature.into(),
        label,
        status: Status::Fail,
        tolerance: 0,
        max_channel_diff: None,
        differing_pixels: None,
        total_pixels: None,
        message: None,
        heatmap: None,
    }
}

/// PNG chunks other than the compressed pixels, in file order.
fn metadata_chunks(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    anyhow::ensure!(bytes.len() > 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n", "not a PNG");
    let mut out = Vec::new();
    let mut i = 8;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into()?) as usize;
        let kind = String::from_utf8_lossy(&bytes[i + 4..i + 8]).into_owned();
        let data = bytes.get(i + 8..i + 8 + len).context("truncated chunk")?.to_vec();
        if kind != "IDAT" {
            out.push((kind, data));
        }
        i += 12 + len;
    }
    Ok(out)
}

pub fn png_metadata(id: &str, feature: &str, port_png: &[u8], refs: &Path) -> CaseResult {
    let mut r = result(format!("{id}#png-metadata"), feature, format!("The PNG export for {id} carries the Mac's metadata"));
    let check = || -> Result<()> {
        let mac = metadata_chunks(&std::fs::read(refs.join(format!("{id}.png")))?)?;
        let port = metadata_chunks(port_png)?;
        let names = |c: &[(String, Vec<u8>)]| c.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>().join(",");
        anyhow::ensure!(names(&mac) == names(&port), "chunks differ: Mac {}, port {}", names(&mac), names(&port));
        for ((kind, a), (_, b)) in mac.iter().zip(&port) {
            anyhow::ensure!(a == b, "{kind} differs");
        }
        Ok(())
    };
    match check() {
        Ok(()) => r.status = Status::Pass,
        Err(e) => r.message = Some(format!("{e:#}")),
    }
    r
}

pub fn jpeg(id: &str, feature: &str, port_jpeg: &[u8], refs: &Path, out: &Path, tolerances: &Tolerances) -> CaseResult {
    let key = format!("{id}#jpeg");
    let limit = tolerances.for_case(&key);
    let mut r = result(key, feature, format!("The JPEG export for {id} decodes to the Mac's pixels"));
    r.tolerance = limit.max_channel_diff;
    let run = || -> Result<compare::Diff> {
        let mac = image::load_from_memory(&std::fs::read(refs.join(format!("{id}.jpg")))?)?.to_rgba8();
        let port = image::load_from_memory(port_jpeg)?.to_rgba8();
        let d = compare::diff(&mac, &port, limit.max_channel_diff)?;
        if d.differing_pixels > limit.max_pixels_over {
            let name = format!("{}__jpeg.png", id.replace('/', "__"));
            let _ = compare::heatmap(&mac, &port, &d, limit.max_channel_diff).save(out.join("heatmaps").join(name));
        }
        Ok(d)
    };
    match run() {
        Ok(d) => {
            r.max_channel_diff = Some(d.max_channel_diff);
            r.differing_pixels = Some(d.differing_pixels);
            r.total_pixels = Some(d.total_pixels);
            r.status = if d.differing_pixels <= limit.max_pixels_over { Status::Pass } else { Status::Fail };
        }
        Err(e) => r.message = Some(format!("{e:#}")),
    }
    r
}
