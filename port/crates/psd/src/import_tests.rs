//! Import checks for what the corpus doesn't reach; the corpus itself is checked against the
//! Mac's projects by `parity compare-projects`.

use super::*;
use image::{Luma, Rgba};

fn solid(w: u32, h: u32, color: [u8; 4]) -> RgbaImage {
    RgbaImage::from_pixel(w, h, Rgba(color))
}

fn written(doc: &Document) -> Vec<u8> {
    write(doc, &WriteOptions::psd()).unwrap()
}

#[test]
fn folders_clipping_and_masks() {
    let mut doc = Document::new(8, 8);
    doc.push(Group::new(
        "Folder",
        vec![
            Layer::pixels("Base", 1, 1, solid(4, 4, [255, 0, 0, 128])).into(),
            Layer::pixels("Clipped", 0, 0, solid(8, 8, [0, 0, 255, 255])).clipped().mask(Mask::new(2, 2, GrayImage::from_pixel(2, 2, Luma([7]))).default_color(0)).into(),
        ],
    ));
    let imported = import(&written(&doc), "file").unwrap();
    let layers = &imported.project.manifest.layers;
    assert_eq!(layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Base", "Clipped", "Folder"]);
    assert_eq!(layers[0].parent_id.as_ref(), Some(&layers[2].id));
    assert_eq!(layers[1].mask_source_id.as_ref(), Some(&layers[0].id));
    assert_eq!(layers[0].transform.origin, [1.0, 1.0]);
    // Premultiplied on import, then unpremultiplied as the project's PNG stores it.
    assert_eq!(imported.project.images[&layers[0].id].pixels.get_pixel(0, 0).0, [255, 0, 0, 128]);
    let mask = &imported.project.masks[&layers[1].id].pixels;
    assert_eq!((mask.get_pixel(1, 1)[0], mask.get_pixel(2, 2)[0], mask.get_pixel(3, 3)[0], mask.get_pixel(4, 4)[0]), (0, 7, 7, 0));
    assert_eq!(imported.project.manifest.active_layer_id.as_ref(), Some(&layers[2].id));
}

#[test]
fn layers_past_the_budget_are_cropped_to_the_canvas() {
    let mut doc = Document::new(8, 8);
    doc.push(Layer::pixels("Big", -4, -4, solid(16, 16, [9, 9, 9, 255])));
    let read = reader::read(&written(&doc), 100).unwrap();
    let layer = &read.layers[0];
    assert!(layer.cropped_to_canvas);
    assert_eq!((layer.bounds.x, layer.bounds.y, layer.bounds.width, layer.bounds.height), (0.0, 0.0, 8.0, 8.0));
    assert!(matches!(reader::read(&written(&doc), 10), Err(ImportError::TooLarge)));
}

#[test]
fn live_shapes_are_reported_as_not_ported() {
    // A solid color fill (`SoCo`) with a rectangle origination (`vogk`).
    let mut soco = Vec::new();
    for (key, value) in [(b"Rd  ", 255.0f64), (b"Grn ", 0.0), (b"Bl  ", 0.0)] {
        soco.extend_from_slice(key);
        soco.extend_from_slice(b"doub");
        soco.extend_from_slice(&value.to_be_bytes());
    }
    let mut vogk = b"keyOriginTypelong".to_vec();
    vogk.extend_from_slice(&1i32.to_be_bytes());
    vogk.extend_from_slice(b"keyOriginShapeBBox");
    for (key, value) in [(b"Left", 1.0f64), (b"Top ", 1.0), (b"Rght", 5.0), (b"Btom", 6.0)] {
        vogk.extend_from_slice(key);
        vogk.extend_from_slice(b"UntF#Pxl");
        vogk.extend_from_slice(&value.to_be_bytes());
    }
    let mut doc = Document::new(8, 8);
    doc.push(Layer::empty("Shape").extra(*b"SoCo", soco).extra(*b"vogk", vogk));
    let read = reader::read(&written(&doc), 1000).unwrap();
    assert_eq!((read.layers[0].bounds.width, read.layers[0].bounds.height), (4.0, 5.0));
    assert!(matches!(import(&written(&doc), "file"), Err(ImportError::NotPorted(_))));
}
