//! What a project looks like after a round through the Mac app's `EditorSession`: opened with
//! `installProject` and written back with `projectSnapshot`. Every optional appearance field comes
//! out explicit, and the resolution defaults to 72.

use comp_format::{BlendMode, Project};

pub fn normalize(project: &Project) -> Project {
    let mut p = project.clone();
    let m = &mut p.manifest;
    m.resolution = Some(m.resolution.unwrap_or(72.0));
    if m.guides.as_ref().is_some_and(|g| g.is_empty()) {
        m.guides = None;
    }
    for layer in &mut m.layers {
        layer.is_group = Some(layer.is_group());
        layer.opacity = Some(layer.opacity());
        layer.blend_mode = Some(layer.blend_mode.unwrap_or(BlendMode::Normal));
        if layer.mask_file.is_some() {
            layer.mask_enabled = Some(layer.mask_enabled.unwrap_or(true));
            layer.mask_linked = Some(layer.mask_linked());
        } else {
            layer.mask_enabled = None;
            layer.mask_linked = None;
            layer.mask_placement = None;
        }
    }
    p
}
