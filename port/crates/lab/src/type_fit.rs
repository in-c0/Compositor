//! Text layers: the port's layout and rasterization against what the Mac's harness recorded
//! (`textLayout` notes in harness-info.json) and saved (the text layer in `<case>.comp`).

use anyhow::{Context, Result};
use image::RgbaImage;
use serde_json::Value;
use std::path::Path;

pub struct TypeCase {
    pub id: String,
    pub port: comp_format::Project,
    pub mac: comp_format::Project,
    pub notes: Vec<Value>,
}

impl TypeCase {
    /// The text layer the case's ops made or edited: the active layer.
    pub fn layers(&self) -> Option<(&comp_format::LayerRecord, &RgbaImage, &comp_format::LayerRecord, &RgbaImage)> {
        let pid = self.port.manifest.active_layer_id.as_ref()?;
        let mid = self.mac.manifest.active_layer_id.as_ref()?;
        let pl = self.port.manifest.layers.iter().find(|l| &l.id == pid)?;
        let ml = self.mac.manifest.layers.iter().find(|l| &l.id == mid)?;
        Some((pl, &self.port.images.get(pid)?.pixels, ml, &self.mac.images.get(mid)?.pixels))
    }
}

pub fn load(corpus: &Path, refs: &Path, pattern: &str) -> Result<Vec<TypeCase>> {
    let info: Value = serde_json::from_slice(&std::fs::read(refs.join("harness-info.json"))?)?;
    let mut out = Vec::new();
    let glob = format!("{}/{pattern}/case.json", corpus.display()).replace('\\', "/");
    for entry in glob::glob(&glob)? {
        let dir = entry?.parent().unwrap().to_path_buf();
        let id = dir.strip_prefix(corpus)?.to_string_lossy().replace('\\', "/");
        let Ok(mac) = comp_format::load(&refs.join(format!("{id}.comp"))) else { continue };
        let spec: Value = serde_json::from_slice(&std::fs::read(dir.join("case.json"))?)?;
        let mut port = comp_format::load(&dir.join("input.comp")).with_context(|| id.clone())?;
        for op in spec["ops"].as_array().into_iter().flatten() {
            port = match op["op"].as_str() {
                Some("text") => engine::text::apply_text(&port, op).map_err(|e| anyhow::anyhow!("{e}"))?,
                Some("editText") => engine::text::apply_edit_text(&port, op).map_err(|e| anyhow::anyhow!("{e}"))?,
                _ => port,
            };
        }
        let notes = info["cases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["id"] == id.as_str())
            .flat_map(|c| c["notes"].as_array().cloned().unwrap_or_default())
            .filter_map(|n| n.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()))
            .collect();
        out.push(TypeCase { id, port, mac, notes });
    }
    Ok(out)
}

/// Per case: the layer's placement and size on both sides, the glyph positions against the Mac's,
/// and how far the pixels are apart.
pub fn report(corpus: &Path, refs: &Path, pattern: &str, verbose: bool) -> Result<()> {
    for case in load(corpus, refs, pattern)? {
        let Some((pl, pimg, ml, mimg)) = case.layers() else {
            println!("{}: no text layer", case.id);
            continue;
        };
        println!(
            "{}: port {:?} {}x{} | mac {:?} {}x{}",
            case.id,
            pl.transform.origin,
            pimg.width(),
            pimg.height(),
            ml.transform.origin,
            mimg.width(),
            mimg.height()
        );
        if let Some(note) = case.notes.last() {
            let layout = &note["textLayout"];
            let style = ml.text.as_ref().unwrap();
            let (w, h) = engine::text::box_size(style);
            let laid = engine::text::layout::layout(style, (w.ceil() - 24.0).max(1.0), (h.ceil() - 24.0).max(1.0));
            let mut mac_glyphs = Vec::new();
            for line in layout["lines"].as_array().into_iter().flatten() {
                let frag = &line["fragment"];
                for g in line["glyphs"].as_array().into_iter().flatten() {
                    if g[4].as_f64() == Some(1.0) && g[0].as_f64() != Some(65535.0) {
                        mac_glyphs.push((g[0].as_f64().unwrap() as u32, frag[0].as_f64().unwrap() + g[2].as_f64().unwrap(), frag[1].as_f64().unwrap() + g[3].as_f64().unwrap()));
                    }
                }
            }
            let mut worst: f64 = 0.0;
            let mut first_bad = None;
            let port_glyphs: Vec<_> = laid.glyphs.iter().map(|g| (g.id, g.x, g.y)).collect();
            let n = mac_glyphs.len().min(port_glyphs.len());
            for i in 0..n {
                let (m, p) = (mac_glyphs[i], port_glyphs[i]);
                let d = (m.1 - p.1).abs().max((m.2 - p.2).abs());
                if (d > 1e-6 || m.0 != p.0) && first_bad.is_none() {
                    first_bad = Some((i, m, p));
                }
                worst = worst.max(d);
            }
            println!(
                "  glyphs mac {} port {}; worst position gap {worst:.6}; measured mac {} port w {:.4} h {:.4}",
                mac_glyphs.len(),
                port_glyphs.len(),
                layout["measured"],
                laid.width,
                laid.height
            );
            if let Some((i, m, p)) = first_bad {
                println!("  first difference at glyph {i}: mac {m:?} port {p:?}");
            }
            if verbose {
                println!("  fonts {}", layout["fonts"]);
            }
        }
        if let Ok(dir) = std::env::var("TYPE_SAVE") {
            let name = case.id.replace('/', "__");
            pimg.save(format!("{dir}/{name}.port.png"))?;
            mimg.save(format!("{dir}/{name}.mac.png"))?;
        }
        if pimg.dimensions() == mimg.dimensions() {
            let mut hist = [0u64; 9];
            let mut max = 0u8;
            let mut at = (0, 0);
            let mut sum = 0i64;
            let mut abs = 0u64;
            for (i, (p, m)) in pimg.pixels().zip(mimg.pixels()).enumerate() {
                let d = p[3].abs_diff(m[3]);
                if d > max {
                    at = (i as u32 % pimg.width(), i as u32 / pimg.width());
                }
                max = max.max(d);
                sum += p[3] as i64 - m[3] as i64;
                abs += d as u64;
                let bucket = match d {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    3..=4 => 3,
                    5..=8 => 4,
                    9..=16 => 5,
                    17..=32 => 6,
                    33..=64 => 7,
                    _ => 8,
                };
                hist[bucket] += 1;
            }
            println!("  alpha diff max {max} at {at:?}, abs {abs} sum port-mac {sum}, histogram 0,1,2,3-4,5-8,9-16,17-32,33-64,65+: {hist:?}");
        }
    }
    Ok(())
}

/// Prints the alpha of both layers side by side over a region.
pub fn dump(corpus: &Path, refs: &Path, id: &str, region: [u32; 4]) -> Result<()> {
    for case in load(corpus, refs, id)? {
        let Some((_, pimg, _, mimg)) = case.layers() else { continue };
        let [x0, y0, w, h] = region;
        for y in y0..(y0 + h).min(mimg.height()) {
            let row = |img: &RgbaImage| {
                (x0..(x0 + w).min(img.width())).map(|x| format!("{:3}", img.get_pixel(x, y)[3])).collect::<Vec<_>>().join(" ")
            };
            println!("{y:3} mac {}", row(mimg));
            if y < pimg.height() {
                println!("    prt {}", row(pimg));
            }
        }
    }
    Ok(())
}

