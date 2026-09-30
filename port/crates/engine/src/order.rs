//! Which layers draw, in what order, and how clipping groups them: the parts of the Mac app's
//! export (`ImageExporter.render`) that decide structure rather than pixels.

use comp_format::{BlendMode, LayerRecord};
use std::collections::{HashMap, HashSet};

/// `LayerHierarchy.visibleLayers`: a depth-first walk from the root, siblings in array order
/// (bottom to top), skipping folders themselves and anything under a hidden folder.
pub fn visible_layers(layers: &[LayerRecord]) -> Vec<&LayerRecord> {
    let mut children: HashMap<Option<&str>, Vec<&LayerRecord>> = HashMap::new();
    for l in layers {
        children.entry(l.parent_id.as_deref()).or_default().push(l);
    }
    let mut out = Vec::new();
    fn visit<'a>(
        parent: Option<&str>,
        depth: usize,
        visible: bool,
        children: &HashMap<Option<&str>, Vec<&'a LayerRecord>>,
        out: &mut Vec<&'a LayerRecord>,
    ) {
        if depth > 64 {
            return;
        }
        for &layer in children.get(&parent).map(Vec::as_slice).unwrap_or(&[]) {
            let shown = visible && layer.is_visible;
            if layer.is_group() {
                visit(Some(&layer.id), depth + 1, shown, children, out);
            } else if shown {
                out.push(layer);
            }
        }
    }
    visit(None, 0, true, &children, &mut out);
    out
}

/// `LayerOpacity.effective`: a layer's opacity times every enclosing folder's.
pub fn effective_opacity(layer: &LayerRecord, by_id: &HashMap<&str, &LayerRecord>) -> f64 {
    let mut opacity = layer.opacity();
    let mut parent = layer.parent_id.as_deref();
    let mut depth = 0;
    while let (Some(id), true) = (parent, depth < 64) {
        let Some(folder) = by_id.get(id) else { break };
        opacity *= folder.opacity();
        parent = folder.parent_id.as_deref();
        depth += 1;
    }
    opacity
}

/// The enclosing folders of `layer`, innermost first.
pub fn folders<'a>(layer: &LayerRecord, by_id: &HashMap<&str, &'a LayerRecord>) -> Vec<&'a LayerRecord> {
    let mut out = Vec::new();
    let mut parent = layer.parent_id.as_deref();
    while let Some(id) = parent {
        let Some(folder) = by_id.get(id) else { break };
        if out.len() >= 64 {
            break;
        }
        out.push(*folder);
        parent = folder.parent_id.as_deref();
    }
    out
}

/// `LiveMaskRenderer.prepareStacks`: a base followed directly by siblings clipped to it forms a
/// stack, drawn together on a surface and then blended in the base's mode.
pub struct Stacks {
    pub stacks: HashMap<String, Vec<String>>,
    pub stacked: HashSet<String>,
    pub modes: HashMap<String, BlendMode>,
}

pub fn prepare_stacks(visible: &[&LayerRecord]) -> Stacks {
    let mut s = Stacks { stacks: HashMap::new(), stacked: HashSet::new(), modes: HashMap::new() };
    for (index, base) in visible.iter().enumerate() {
        if base.mask_source_id.is_some() || base.adjustment.is_some() {
            continue;
        }
        let mut children = Vec::new();
        for child in &visible[index + 1..] {
            if child.mask_source_id.as_deref() != Some(base.id.as_str()) || child.parent_id != base.parent_id {
                break;
            }
            children.push(child.id.clone());
        }
        if children.is_empty() {
            continue;
        }
        s.stacked.extend(children.iter().cloned());
        s.modes.insert(base.id.clone(), base.blend_mode());
        s.stacks.insert(base.id.clone(), children);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use comp_format::Transform;

    fn layer(id: &str, parent: Option<&str>, group: bool, visible: bool) -> LayerRecord {
        LayerRecord {
            id: id.into(),
            name: id.into(),
            is_visible: visible,
            transform: Transform::at(0.0, 0.0, 1.0, 1.0),
            image_file: None,
            parent_id: parent.map(Into::into),
            is_group: group.then_some(true),
            opacity: None,
            blend_mode: None,
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

    #[test]
    fn walks_folders_depth_first_in_sibling_order() {
        // The array lists a folder's child before the folder and a root layer between them;
        // drawing still follows the tree: a, then the folder's b, then c.
        let layers = vec![
            layer("a", None, false, true),
            layer("b", Some("f"), false, true),
            layer("f", None, true, true),
            layer("c", None, false, true),
            layer("h", None, true, false),
            layer("x", Some("h"), false, true),
        ];
        let ids: Vec<&str> = visible_layers(&layers).iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn stacks_need_contiguous_siblings() {
        let mut layers = vec![layer("base", None, false, true), layer("c1", None, false, true), layer("mid", None, false, true), layer("c2", None, false, true)];
        layers[1].mask_source_id = Some("base".into());
        layers[3].mask_source_id = Some("base".into());
        let visible = visible_layers(&layers);
        let s = prepare_stacks(&visible);
        assert_eq!(s.stacks["base"], vec!["c1".to_string()]);
        assert!(!s.stacked.contains("c2"));
    }
}
