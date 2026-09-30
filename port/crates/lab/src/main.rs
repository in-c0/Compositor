//! Fits candidate pixel formulas to the Mac references.
//!
//! `lab <experiment> <corpus dir> <refs dir>` prints, for each candidate, how many channels differ
//! from the references and by how much, so the exact Core Graphics arithmetic can be read off
//! before it goes into WGSL.

use anyhow::{Context, Result, bail};
use image::RgbaImage;
use std::path::{Path, PathBuf};

mod blend;
mod experiments;
mod type_fit;

pub struct Case {
    pub id: String,
    pub project: comp_format::Project,
    pub reference: RgbaImage,
}

impl Case {
    /// A layer's straight RGBA pixels, by position bottom to top.
    pub fn layer(&self, index: usize) -> &RgbaImage {
        let id = &self.project.manifest.layers[index].id;
        &self.project.images[id].pixels
    }
    pub fn record(&self, index: usize) -> &comp_format::LayerRecord {
        &self.project.manifest.layers[index]
    }
}

pub fn load_cases(corpus: &Path, refs: &Path, pattern: &str) -> Result<Vec<Case>> {
    let mut out = Vec::new();
    let glob = format!("{}/{pattern}/case.json", corpus.display()).replace('\\', "/");
    for entry in glob::glob(&glob)? {
        let dir = entry?.parent().unwrap().to_path_buf();
        let id = dir.strip_prefix(corpus)?.to_string_lossy().replace('\\', "/");
        let reference = match image::open(refs.join(format!("{id}.png"))) {
            Ok(img) => img.to_rgba8(),
            Err(_) => continue,
        };
        let project = comp_format::load(&dir.join("input.comp")).with_context(|| id.clone())?;
        out.push(Case { id, project, reference });
    }
    if out.is_empty() {
        bail!("no cases with references match {pattern}");
    }
    Ok(out)
}

/// Mismatch statistics for one candidate over many pixels.
#[derive(Default, Debug, Clone, Copy)]
pub struct Score {
    pub channels: u64,
    pub wrong: u64,
    pub max: u8,
}

impl Score {
    pub fn add(&mut self, got: u8, want: u8) {
        self.channels += 1;
        let d = got.abs_diff(want);
        if d != 0 {
            self.wrong += 1;
        }
        self.max = self.max.max(d);
    }
    /// Straight-alpha pixels, skipping color where both are fully transparent.
    pub fn add_pixel(&mut self, got: [u8; 4], want: [u8; 4]) {
        if got[3] == 0 && want[3] == 0 {
            self.add(0, 0);
            return;
        }
        for c in 0..4 {
            self.add(got[c], want[c]);
        }
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:>8} wrong of {:>8} (max {:>3})", self.wrong, self.channels, self.max)
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        bail!("usage: lab <experiment> <corpus> <refs>");
    }
    let (corpus, refs) = (PathBuf::from(&args[2]), PathBuf::from(&args[3]));
    experiments::run(&args[1], &corpus, &refs, &args[4..])
}
