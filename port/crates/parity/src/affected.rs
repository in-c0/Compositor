use crate::report::FeatureList;
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// The case globs a change since `base` can affect.
///
/// A changed case re-runs itself; a change to a feature's `paths` re-runs that feature. Anything
/// else under `port/` or `parity/` that no feature claims (shared engine code, the tool itself,
/// tolerances) re-runs everything, since it can move any pixel.
pub fn affected(base: &str, features: &FeatureList, corpus: &Path) -> Result<Vec<String>> {
    let output = Command::new("git").args(["diff", "--name-only", &format!("{base}...HEAD")]).output().context("running git diff")?;
    if !output.status.success() {
        bail!("git diff failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let changed: Vec<String> = String::from_utf8(output.stdout)?.lines().map(str::to_owned).collect();
    let corpus_prefix = format!("{}/", corpus.to_string_lossy().replace('\\', "/").trim_end_matches('/'));
    let mut globs = BTreeSet::new();
    for file in &changed {
        if file.ends_with(".md") {
            continue;
        }
        if let Some(rest) = file.strip_prefix(&corpus_prefix) {
            // `<feature>/<case>/...`: the case folder is the first two components.
            let parts: Vec<&str> = rest.split('/').collect();
            if parts.len() >= 3 {
                globs.insert(format!("{}/{}", parts[0], parts[1]));
                continue;
            }
        }
        let mut claimed = false;
        for f in &features.features {
            if f.paths.iter().any(|p| glob::Pattern::new(p).is_ok_and(|g| g.matches(file))) {
                // A feature without corpus cases (the UI) has nothing to re-run here; the workflow
                // decides on the UI comparison itself.
                if corpus.join(&f.key).is_dir() {
                    globs.insert(format!("{}/*", f.key));
                }
                claimed = true;
            }
        }
        if !claimed && (file.starts_with("port/") || file.starts_with("parity/")) {
            return Ok(vec!["*".into()]);
        }
    }
    Ok(globs.into_iter().collect())
}
