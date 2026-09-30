use super::images::hash;
use anyhow::Result;
use comp_format::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A project under construction. Layers are added bottom to top.
pub struct Doc {
    pub width: u32,
    pub height: u32,
    pub version: i64,
    pub layers: Vec<LayerRecord>,
    images: HashMap<String, RgbaImage>,
    masks: HashMap<String, GrayImage>,
    seed: u32,
    next: u32,
}

/// Optional layer settings, applied by `Doc::image` and friends.
#[derive(Default, Clone)]
pub struct LayerSpec {
    pub opacity: Option<f64>,
    pub blend: Option<BlendMode>,
    pub transform: Option<Transform>,
    pub parent: Option<String>,
    pub visible: Option<bool>,
}

impl Doc {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, version: CURRENT_VERSION, layers: Vec::new(), images: HashMap::new(), masks: HashMap::new(), seed: 0, next: 0 }
    }

    fn id(&mut self) -> String {
        let mut bytes = [0u8; 16];
        for (i, chunk) in bytes.chunks_mut(4).enumerate() {
            chunk.copy_from_slice(&hash(self.seed ^ hash(self.next * 4 + i as u32 + 1)).to_be_bytes());
        }
        self.next += 1;
        uuid::Builder::from_random_bytes(bytes).into_uuid().to_string().to_ascii_uppercase()
    }

    fn record(&mut self, name: &str, spec: &LayerSpec, image_file: bool, default_transform: Transform) -> LayerRecord {
        let id = self.id();
        LayerRecord {
            image_file: image_file.then(|| format!("{id}.png")),
            id,
            name: name.into(),
            is_visible: spec.visible.unwrap_or(true),
            transform: spec.transform.unwrap_or(default_transform),
            parent_id: spec.parent.clone(),
            is_group: None,
            opacity: spec.opacity,
            blend_mode: spec.blend,
            mask_file: None,
            mask_enabled: None,
            mask_source_id: None,
            adjustment: None,
            mask_placement: None,
            mask_linked: None,
            shape: None,
            effects: None,
            text: None,
        }
    }

    fn canvas(&self) -> Transform {
        Transform::at(0.0, 0.0, self.width as f64, self.height as f64)
    }

    /// Adds an image layer and returns its id. Without a transform it is placed at the top-left
    /// corner at its own pixel size, as a paste would place it.
    pub fn image(&mut self, name: &str, pixels: RgbaImage, spec: LayerSpec) -> String {
        let t = Transform::at(0.0, 0.0, pixels.width() as f64, pixels.height() as f64);
        let record = self.record(name, &spec, true, t);
        let id = record.id.clone();
        self.images.insert(id.clone(), pixels);
        self.layers.push(record);
        id
    }

    pub fn adjustment(&mut self, name: &str, adjustment: Adjustment, spec: LayerSpec) -> String {
        let t = self.canvas();
        let mut record = self.record(name, &spec, false, t);
        record.adjustment = Some(adjustment);
        let id = record.id.clone();
        self.layers.push(record);
        id
    }

    pub fn group(&mut self, name: &str, spec: LayerSpec) -> String {
        let t = self.canvas();
        let mut record = self.record(name, &spec, false, t);
        record.is_group = Some(true);
        let id = record.id.clone();
        self.layers.push(record);
        id
    }

    pub fn layer(&mut self, id: &str) -> &mut LayerRecord {
        self.layers.iter_mut().find(|l| l.id == id).expect("layer exists")
    }

    pub fn mask(&mut self, id: &str, mask: GrayImage) {
        let layer = self.layer(id);
        layer.mask_file = Some(format!("{id}.mask.png"));
        self.masks.insert(id.to_string(), mask);
    }

    fn into_project(self) -> Project {
        let active = self.layers.last().map(|l| l.id.clone());
        let doc_seed = hash(self.seed ^ 0xd0c);
        let mut bytes = [0u8; 16];
        for (i, chunk) in bytes.chunks_mut(4).enumerate() {
            chunk.copy_from_slice(&hash(doc_seed.wrapping_add(i as u32)).to_be_bytes());
        }
        let manifest = Manifest {
            format: FORMAT.into(),
            version: self.version,
            color_space: "sRGB".into(),
            resolution: None,
            document_id: uuid::Builder::from_random_bytes(bytes).into_uuid().to_string().to_ascii_uppercase(),
            width: self.width as i64,
            height: self.height as i64,
            active_layer_id: active,
            layers: self.layers,
            guides: None,
        };
        Project {
            manifest,
            images: self.images.into_iter().map(|(k, v)| (k, Asset::new(v))).collect(),
            masks: self.masks.into_iter().map(|(k, v)| (k, Asset::new(v))).collect(),
        }
    }
}

pub struct CaseWriter {
    root: PathBuf,
    pub count: usize,
}

impl CaseWriter {
    pub fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf(), count: 0 }
    }

    /// A fresh document whose layer ids are derived from the case name, so they are stable.
    pub fn doc(&self, feature: &str, case: &str, width: u32, height: u32) -> Doc {
        let mut d = Doc::new(width, height);
        d.seed = feature.bytes().chain(b"/".iter().copied()).chain(case.bytes()).fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
        d
    }

    pub fn write(&mut self, feature: &str, case: &str, label: &str, doc: Doc, ops: Vec<Value>) -> Result<()> {
        let dir = self.root.join(feature).join(case);
        std::fs::create_dir_all(&dir)?;
        comp_format::save(&doc.into_project(), &dir.join("input.comp"))?;
        let mut spec = serde_json::json!({ "feature": feature, "label": label, "input": "input.comp" });
        if !ops.is_empty() {
            spec["ops"] = Value::Array(ops);
        }
        std::fs::write(dir.join("case.json"), serde_json::to_string_pretty(&spec)? + "\n")?;
        self.count += 1;
        Ok(())
    }
}
