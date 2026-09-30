use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One corpus case: `corpus/<id>/case.json` plus its input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseSpec {
    pub feature: String,
    pub label: String,
    pub input: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<Value>,
    /// Export JPEG options (`quality`, `matte`); the harness writes `<case>.jpg` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jpeg: Option<Value>,
    /// For a camera RAW input, the develop sheet's settings to change before importing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct Case {
    /// Path relative to the corpus root with `/` separators, e.g. `blend/multiply-o100-opaque`.
    pub id: String,
    pub dir: PathBuf,
    pub spec: CaseSpec,
}

impl Case {
    pub fn input_path(&self) -> PathBuf {
        self.dir.join(&self.spec.input)
    }
}

/// Every case under `root`, sorted by id.
pub fn discover(root: &Path) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    let pattern = format!("{}/**/case.json", root.display()).replace('\\', "/");
    for entry in glob::glob(&pattern)? {
        let path = entry?;
        let dir = path.parent().unwrap().to_path_buf();
        let id = dir
            .strip_prefix(root)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let spec: CaseSpec = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("reading {}", path.display()))?;
        cases.push(Case { id, dir, spec });
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cases)
}

/// Keeps the cases whose id matches any of `patterns` (glob syntax); all of them when empty.
pub fn select(cases: Vec<Case>, patterns: &[String]) -> Result<Vec<Case>> {
    if patterns.is_empty() {
        return Ok(cases);
    }
    let compiled = patterns.iter().map(|p| glob::Pattern::new(p)).collect::<Result<Vec<_>, _>>()?;
    Ok(cases.into_iter().filter(|c| compiled.iter().any(|p| p.matches(&c.id))).collect())
}
