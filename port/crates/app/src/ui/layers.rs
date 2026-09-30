//! The Layers panel (`LayersPanel` and `NativeLayerList`, docs/port/ui-inventory.md §2.1–2.2).

use crate::app::App;
use crate::document::{self, Doc};
use crate::icons::{self, Icon};
use crate::theme::{self, color, metric, white_alpha};
use crate::widgets as w;
use comp_format::{BlendMode, LayerRecord, Project};
use eframe::egui::{self, Color32, ColorImage, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

const BLEND_GROUPS: [&[BlendMode]; 6] = [
    &[BlendMode::Normal],
    &[BlendMode::Darken, BlendMode::Multiply, BlendMode::ColorBurn, BlendMode::LinearBurn],
    &[BlendMode::Lighten, BlendMode::Screen, BlendMode::ColorDodge, BlendMode::LinearDodge],
    &[BlendMode::Overlay, BlendMode::SoftLight, BlendMode::HardLight, BlendMode::VividLight, BlendMode::LinearLight, BlendMode::PinLight, BlendMode::HardMix],
    &[BlendMode::Difference, BlendMode::Exclusion, BlendMode::Subtract, BlendMode::Divide],
    &[BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity],
];

pub const HEADER: f32 = 18.0 * 2.0 + 15.0;
pub const APPEARANCE: f32 = 12.0 + 22.0 + 8.0 + 22.0 + 12.0;
pub const FOOTER: f32 = 44.0;

pub fn panel(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().with_clip_rect(rect);
    // Header: "Layers" and the layer count.
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
    let count = app.doc().map_or(0, |d| d.project.manifest.layers.len());
    p.text(pos2(header.min.x + 18.0, header.center().y), egui::Align2::LEFT_CENTER, "Layers", theme::semibold(12.0), color::label());
    p.text(pos2(header.max.x - 18.0, header.center().y), egui::Align2::RIGHT_CENTER, count.to_string(), theme::regular(10.0), color::tertiary());
    super::hline(ui, rect.x_range().into(), header.max.y);
    let appearance = Rect::from_min_size(pos2(rect.min.x, header.max.y + 1.0), vec2(rect.width(), APPEARANCE));
    appearance_controls(app, ui, appearance);
    super::hline(ui, rect.x_range().into(), appearance.max.y);
    let footer = Rect::from_min_max(pos2(rect.min.x, rect.max.y - FOOTER), rect.max);
    super::hline(ui, rect.x_range().into(), footer.min.y - 1.0);
    let list = Rect::from_min_max(pos2(rect.min.x, appearance.max.y + 1.0), pos2(rect.max.x, footer.min.y - 1.0));
    if app.doc().is_some_and(|d| !d.project.manifest.layers.is_empty()) {
        layer_list(app, ui, list);
    } else {
        empty_state(app, ui, list);
    }
    footer_bar(app, ui, footer);
}

/// `LayerAppearanceControls`: blend mode and opacity of the active layer.
fn appearance_controls(app: &mut App, ui: &mut Ui, rect: Rect) {
    let inner = rect.shrink(12.0);
    let active = app.doc().and_then(|d| d.active_layer()).map(|l| (l.id.clone(), l.blend_mode(), l.opacity()));
    let enabled = active.is_some();
    let (mut mode, opacity) = active.as_ref().map_or((BlendMode::Normal, 1.0), |(_, m, o)| (*m, *o));
    let row1 = Rect::from_min_size(inner.min, vec2(inner.width(), 22.0));
    let mut changed_mode = false;
    w::row(ui, row1, 8.0, |ui| {
        w::text(ui, "Blend", theme::regular(10.0), color::label());
        let width = w::fill_width(ui, 0.0, 0);
        changed_mode = w::popup(ui, "blend-mode", &mut mode, &BLEND_GROUPS, BlendMode::name, Some(width), enabled);
    });
    let row2 = Rect::from_min_size(pos2(inner.min.x, row1.max.y + 8.0), vec2(inner.width(), 22.0));
    let mut pct = (opacity * 100.0).round();
    let mut dragging = false;
    let mut committed = false;
    w::row(ui, row2, 6.0, |ui| {
        let scrubbed = w::scrub_label(ui, "Opacity", theme::regular(10.0), color::label(), &mut pct, 1.0, 0.0..=100.0, enabled);
        dragging |= scrubbed.dragged();
        let percent = ui.painter().layout_no_wrap("%".into(), theme::regular(10.0), color::label()).size().x;
        let slider_width = w::fill_width(ui, 44.0 + 2.0 + percent, 1);
        let mut fraction = pct / 100.0;
        let slider = w::slider(ui, &mut fraction, 0.0..=1.0, slider_width, enabled);
        if slider.changed() {
            pct = fraction * 100.0;
        }
        dragging |= slider.dragged();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let field = w::number_field(ui, "layer-opacity", &mut pct, 0.0..=100.0, w::fmt_int, 44.0, false, enabled);
            committed |= field.lost_focus();
            w::text(ui, "%", theme::regular(10.0), color::label());
        });
    });
    let Some((id, _, _)) = active else { return };
    let Some(doc) = app.doc_mut() else { return };
    if changed_mode {
        doc.set_blend_mode(&id, mode);
    }
    let new = (pct / 100.0).clamp(0.0, 1.0);
    if (new - opacity).abs() > 1e-9 && (dragging || committed || (pct - (opacity * 100.0).round()).abs() >= 0.5) {
        doc.set_opacity(&id, new, true);
    }
    if !dragging && !ui.ctx().egui_is_using_pointer() {
        doc.end_coalescing();
    }
}

fn empty_state(app: &App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().with_clip_rect(rect);
    let c = color::secondary();
    let center = rect.center();
    icons::paint(&p, Icon::Symbol("square.3.layers.3d"), center - vec2(0.0, 30.0), 25.0, c);
    p.text(center, egui::Align2::CENTER_CENTER, "No layers yet", theme::medium(12.0), c);
    let hint = if app.doc().is_some() { "Import an image or add a blank layer." } else { "Create a canvas or import an image." };
    p.text(center + vec2(0.0, 22.0), egui::Align2::CENTER_CENTER, hint, theme::regular(10.0), c);
}

/// The footer: new layer, group, mask, effects, adjustment, and delete at the far end.
fn footer_bar(_app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().with_clip_rect(rect);
    let c = color::secondary().gamma_multiply(0.5);
    let mut x = rect.min.x + 8.0;
    let cy = rect.center().y;
    for (symbol, menu, help) in [
        ("plus.square", false, "New blank layer (⇧⌘N)"),
        ("folder.badge.plus", false, "Group selected layers (⌘G)"),
        ("rectangle.inset.filled", false, "Add layer mask (Option-click for a black mask)"),
        ("sparkles", true, "Layer effects: stroke and drop shadow"),
        ("circle.lefthalf.filled", true, "New adjustment layer"),
    ] {
        let width = 32.0 + if menu { 10.0 } else { 0.0 };
        let hit = Rect::from_min_size(pos2(x, rect.min.y + 4.0), vec2(width, rect.height() - 8.0));
        icons::paint(&p, Icon::Symbol(symbol), pos2(x + 16.0, cy), 13.0, c);
        if menu {
            icons::paint(&p, Icon::Symbol("chevron.down"), pos2(x + 30.0, cy + 1.0), 7.0, c);
        }
        ui.interact(hit, ui.id().with(("footer", symbol)), Sense::hover()).on_hover_text(help);
        x += width;
    }
    icons::paint(&p, Icon::Symbol("trash"), pos2(rect.max.x - 8.0 - 16.0, cy), 13.0, c);
}

/// Row pitch: the 52-point cell, 24 per effect, and the table's 2-point intercell spacing.
fn row_height(layer: &LayerRecord) -> f32 {
    metric::ROW + metric::EFFECT_ROW * document::effect_rows(layer).len() as f32 + metric::ROW_GAP
}

fn layer_list(app: &mut App, ui: &mut Ui, rect: Rect) {
    let focused = app.layers_focused;
    let dragging = app.dragging_layer.clone();
    let Some(doc) = app.current.and_then(|i| app.docs.get_mut(i)) else { return };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let mut actions = Vec::new();
    egui::ScrollArea::vertical().id_salt("layers").auto_shrink([false, false]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ensure_thumbnails(ui.ctx(), doc);
        let rows = document::rows(&doc.project.manifest.layers, &doc.collapsed);
        for row in &rows {
            let (rect, response) = ui.allocate_exact_size(vec2(rect.width(), row_height(row.layer)), Sense::click_and_drag());
            if ui.is_rect_visible(rect) {
                draw_row(ui, doc, row, rect, focused, &mut actions);
            }
            if response.clicked() || response.drag_started() || response.secondary_clicked() {
                actions.push(Action::Select(row.layer.id.clone()));
            }
            let menu = crate::menus::layer_context(&row_state(row));
            response.context_menu(|ui| {
                ui.set_min_width(200.0);
                if let Some(c) = crate::menus::menu_items(ui, &menu) {
                    actions.push(Action::Select(row.layer.id.clone()));
                    actions.push(Action::Run(c));
                }
            });
            if response.drag_started() {
                actions.push(Action::DragStart(row.layer.id.clone()));
            }
            if let Some(dragged) = &dragging {
                if dragged != &row.layer.id && ui.input(|i| i.pointer.any_released()) {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()).filter(|p| rect.contains(*p)) {
                        actions.push(Action::Drop(dragged.clone(), row.layer.id.clone(), pos.y < rect.center().y));
                    }
                }
                if dragged != &row.layer.id && ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p)) {
                    let above = ui.input(|i| i.pointer.hover_pos()).unwrap().y < rect.center().y;
                    let y = if above { rect.min.y } else { rect.max.y - 1.0 };
                    ui.painter().rect_filled(Rect::from_min_max(pos2(rect.min.x + 8.0, y - 1.0), pos2(rect.max.x - 8.0, y + 1.0)), 1.0, color::ACCENT);
                }
            }
        }
    });
    if ui.input(|i| i.pointer.any_released()) {
        actions.push(Action::DragEnd);
    }
    let mut commands = Vec::new();
    for action in actions {
        match action {
            Action::Run(c) => commands.push(c),
            Action::Select(id) => {
                doc.active = Some(id);
                app.layers_focused = true;
            }
            Action::ToggleVisible(id) => {
                if let Some(v) = doc.layer(&id).map(|l| l.is_visible) {
                    doc.set_visible(&id, !v);
                }
            }
            Action::ToggleCollapsed(id) => {
                if !doc.collapsed.remove(&id) {
                    doc.collapsed.insert(id);
                }
            }
            Action::DragStart(id) => app.dragging_layer = Some(id),
            // Rows lie top first, so "above" in the list is later in the array.
            Action::Drop(id, target, above) => doc.reorder(&id, &target, above),
            Action::DragEnd => app.dragging_layer = None,
        }
    }
    for c in commands {
        app.run(ui.ctx(), c);
    }
}

enum Action {
    Select(String),
    ToggleVisible(String),
    ToggleCollapsed(String),
    DragStart(String),
    Drop(String, String, bool),
    DragEnd,
    Run(crate::menus::Command),
}

pub fn row_state(row: &document::Row) -> crate::menus::RowState {
    let l = row.layer;
    crate::menus::RowState {
        is_folder: l.is_group(),
        clipped: row.clipped,
        visible: l.is_visible,
        mask: l.mask_file.as_ref().map(|_| l.mask_enabled()),
        mask_linked: l.mask_linked(),
    }
}

/// `LayerCell`: eye, indent, disclosure, thumbnail, mask, name and detail; effect rows below.
fn draw_row(ui: &mut Ui, doc: &Doc, row: &document::Row, rect: Rect, focused: bool, actions: &mut Vec<Action>) {
    let layer = row.layer;
    let selected = doc.active.as_deref() == Some(layer.id.as_str());
    let base = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if selected {
        base.rect_filled(rect, 0.0, if focused { color::ACCENT } else { color::UNEMPHASIZED_SELECTION });
    }
    let mut p = base.clone();
    if row.hidden_by_parent {
        p.set_opacity(0.35);
    }
    // The cell sits in the row, one point below its top (half the intercell spacing).
    let top = rect.min.y + metric::ROW_GAP / 2.0;
    let left = rect.min.x;
    let cy = top + 26.0;
    let label = color::label();
    // Eye.
    let eye = Rect::from_min_size(pos2(left + 8.0, cy - 16.0), vec2(20.0, 32.0));
    icons::paint(&p, Icon::Symbol(if layer.is_visible { "eye" } else { "eye.slash" }), eye.center(), 13.0, label);
    if ui.interact(eye, ui.id().with(("eye", &layer.id)), Sense::click()).clicked() {
        actions.push(Action::ToggleVisible(layer.id.clone()));
    }
    let indent = row.depth.min(8) as f32 * 24.0 + if row.clipped { 24.0 } else { 0.0 };
    let disclosure = Rect::from_min_size(pos2(eye.max.x + indent, cy - 12.0), vec2(16.0, 24.0));
    if layer.is_group() {
        let collapsed = doc.collapsed.contains(&layer.id);
        icons::paint(&p, Icon::Symbol(if collapsed { "chevron.right" } else { "chevron.down" }), disclosure.center(), 11.0, color::secondary());
        if ui.interact(disclosure, ui.id().with(("disclose", &layer.id)), Sense::click()).clicked() {
            actions.push(Action::ToggleCollapsed(layer.id.clone()));
        }
    }
    let slot = Rect::from_min_size(pos2(disclosure.max.x - 2.0, top), vec2(36.0, 52.0));
    let canvas = doc.size();
    let icon = if layer.text.is_some() {
        Some(("textformat", 16.0))
    } else if let Some(adj) = &layer.adjustment {
        Some((icons::adjustment_symbol(adj.kind), 16.0))
    } else if layer.is_group() {
        Some(("folder", 26.0))
    } else {
        None
    };
    let thumb = match icon {
        Some((symbol, size)) => {
            let r = Rect::from_center_size(slot.center(), vec2(36.0, 36.0));
            if matches!(layer.adjustment.as_ref().map(|a| a.kind), Some(comp_format::AdjustmentKind::Curves)) {
                icons::paint_rotated(&p, symbol, r.center(), size, label, std::f32::consts::FRAC_PI_2);
            } else {
                icons::paint(&p, Icon::Symbol(symbol), r.center(), size, label);
            }
            r
        }
        None => {
            let r = Rect::from_center_size(slot.center(), fit(canvas, 36.0));
            if let Some((_, tex)) = doc.thumbs.get(&layer.id) {
                p.add(egui::epaint::RectShape::filled(r, 3.0, Color32::WHITE).with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
            }
            r
        }
    };
    let single_active = selected;
    if single_active {
        p.rect_stroke(thumb, 3.0, Stroke::new(2.0, color::ACCENT), StrokeKind::Inside);
    }
    // Mask.
    let linkable = layer.mask_file.is_some() && layer.adjustment.is_none() && !layer.is_group();
    let gap = if linkable { 13.0 } else { 5.0 };
    let mask_w = if layer.mask_file.is_some() { 30.0 } else { 0.0 };
    let mask_slot = Rect::from_min_size(pos2(slot.max.x + gap, top), vec2(mask_w, 52.0));
    if layer.mask_file.is_some() {
        let r = Rect::from_center_size(mask_slot.center(), fit(canvas, 30.0));
        if let Some((_, tex)) = doc.thumbs.get(&format!("{}#mask", layer.id)) {
            p.add(egui::epaint::RectShape::filled(r, 3.0, Color32::WHITE).with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
        }
        if layer.mask_enabled == Some(false) {
            p.text(r.center(), egui::Align2::CENTER_CENTER, "╱", theme::medium(32.0), color::RED);
        }
        if linkable && layer.mask_linked() {
            icons::paint_rotated(&p, "link", pos2(mask_slot.min.x - 6.5, cy), 10.0, color::secondary(), std::f32::consts::FRAC_PI_4);
        }
    }
    // Name and detail.
    let name_x = mask_slot.max.x + 5.0;
    let text_clip = Rect::from_min_max(pos2(name_x, top), pos2(rect.max.x - 8.0, top + 52.0));
    let tp = p.with_clip_rect(text_clip.intersect(p.clip_rect()));
    let name = if layer.mask_source_id.is_some() { format!("↳ {}", layer.name) } else { layer.name.clone() };
    let name_color = if selected && focused { Color32::WHITE } else { label };
    let g = tp.layout_no_wrap(name, theme::regular(13.0), name_color);
    let name_h = g.size().y;
    tp.galley(pos2(name_x, top + 9.0), g, name_color);
    let detail = detail_line(doc, layer);
    let detail_color = if selected && focused { white_alpha(0.75) } else { color::secondary() };
    tp.text(pos2(name_x, top + 9.0 + name_h + 3.0), egui::Align2::LEFT_TOP, detail, theme::regular(10.0), detail_color);
    // Effect sub-rows.
    for (i, (kind, enabled)) in document::effect_rows(layer).into_iter().enumerate() {
        let y = top + 52.0 + 24.0 * i as f32;
        let eye = Rect::from_min_size(pos2(left + 38.0 + indent, y + 1.0), vec2(20.0, 22.0));
        icons::paint(&p, Icon::Symbol(if enabled { "eye" } else { "eye.slash" }), eye.center(), 11.0, color::secondary());
        p.text(pos2(eye.max.x + 8.0, y + 12.0), egui::Align2::LEFT_CENTER, kind, theme::regular(11.0), if enabled { label } else { color::secondary() });
    }
    // Row edge: one device pixel at the cell's bottom.
    let hair = 1.0 / ui.ctx().pixels_per_point();
    let bottom = rect.max.y - metric::ROW_GAP / 2.0;
    base.rect_filled(Rect::from_min_max(pos2(rect.min.x, bottom - hair), pos2(rect.max.x, bottom)), 0.0, white_alpha(0.06));
}

fn detail_line(doc: &Doc, layer: &LayerRecord) -> String {
    if let Some(source) = &layer.mask_source_id {
        let name = doc.layer(source).map_or("Missing source".to_string(), |l| l.name.clone());
        return format!("Clipped to {name}");
    }
    if layer.text.is_some() {
        return "Text · Double-click to edit".into();
    }
    if layer.adjustment.is_some() {
        return "Adjustment · Double-click to edit".into();
    }
    if layer.is_group() {
        return "Folder".into();
    }
    let t = &layer.transform;
    let text = format!("{} × {} px", t.size[0].round() as i64, t.size[1].round() as i64);
    let Some(pixels) = doc.project.images.get(&layer.id).map(|a| a.pixels.width()).filter(|w| *w > 0) else { return text };
    let percent = t.size[0] / pixels as f64 * 100.0;
    if (percent - 100.0).abs() < 0.05 {
        return text;
    }
    format!("{text} · {}%", w::fmt_trim1(percent))
}

/// The canvas's aspect fitted in a `side`-point square.
fn fit(canvas: egui::Vec2, side: f32) -> egui::Vec2 {
    let s = side / canvas.x.max(canvas.y).max(1.0);
    vec2((canvas.x * s).max(1.0), (canvas.y * s).max(1.0))
}

/// Builds or refreshes the thumbnail textures (`CanvasThumbnail`) for the current document.
fn ensure_thumbnails(ctx: &egui::Context, doc: &mut Doc) {
    let ppp = ctx.pixels_per_point();
    let version = doc.thumbs_version;
    let canvas = doc.size();
    let mut wanted = Vec::new();
    for layer in &doc.project.manifest.layers {
        if layer.text.is_none() && layer.adjustment.is_none() && !layer.is_group() {
            wanted.push((layer.id.clone(), false));
        }
        if layer.mask_file.is_some() {
            wanted.push((layer.id.clone(), true));
        }
    }
    for (id, mask) in wanted {
        let key = if mask { format!("{id}#mask") } else { id.clone() };
        if doc.thumbs.get(&key).is_some_and(|(v, _)| *v == version) {
            continue;
        }
        let layer = doc.project.manifest.layers.iter().find(|l| l.id == id).unwrap();
        let size = fit(canvas, if mask { 30.0 } else { 36.0 }) * ppp;
        let (tw, th) = (size.x.round().max(1.0) as usize, size.y.round().max(1.0) as usize);
        let image = if mask { mask_thumbnail(&doc.project, layer, tw, th) } else { layer_thumbnail(&doc.project, layer, tw, th, ppp) };
        let texture = ctx.load_texture(format!("thumb-{key}"), image, egui::TextureOptions::LINEAR);
        doc.thumbs.insert(key, (version, texture));
    }
}

/// Samples a placed image at document point (x, y): its transform's rectangle, flips honored.
fn placed(t: &comp_format::Transform, iw: u32, ih: u32, x: f32, y: f32) -> Option<(u32, u32)> {
    let (ox, oy, sw, sh) = (t.origin[0] as f32, t.origin[1] as f32, t.size[0] as f32, t.size[1] as f32);
    if sw <= 0.0 || sh <= 0.0 {
        return None;
    }
    let mut u = (x - ox) / sw;
    let mut v = (y - oy) / sh;
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    if t.flip_x {
        u = 1.0 - u;
    }
    if t.flip_y {
        v = 1.0 - v;
    }
    Some((((u * iw as f32) as u32).min(iw - 1), ((v * ih as f32) as u32).min(ih - 1)))
}

/// A layer on a canvas-shaped checkerboard (gray 0.22 with 0.32 squares, 6-point tiles).
fn layer_thumbnail(project: &Project, layer: &LayerRecord, tw: usize, th: usize, ppp: f32) -> ColorImage {
    let (cw, ch) = (project.manifest.width as f32, project.manifest.height as f32);
    let tile = 6.0 * ppp;
    let asset = project.images.get(&layer.id);
    let mut pixels = Vec::with_capacity(tw * th);
    const SS: usize = 4;
    for j in 0..th {
        for i in 0..tw {
            let checker = if ((i as f32 / tile) as usize + (j as f32 / tile) as usize) % 2 == 1 { 0.32 } else { 0.22 };
            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0, 0.0, 0.0);
            if let Some(asset) = asset {
                let (iw, ih) = asset.pixels.dimensions();
                for sy in 0..SS {
                    for sx in 0..SS {
                        let x = (i as f32 + (sx as f32 + 0.5) / SS as f32) / tw as f32 * cw;
                        let y = (j as f32 + (sy as f32 + 0.5) / SS as f32) / th as f32 * ch;
                        if let Some((px, py)) = placed(&layer.transform, iw, ih, x, y) {
                            let c = asset.pixels.get_pixel(px, py).0;
                            let al = c[3] as f32 / 255.0;
                            r += c[0] as f32 / 255.0 * al;
                            g += c[1] as f32 / 255.0 * al;
                            b += c[2] as f32 / 255.0 * al;
                            a += al;
                        }
                    }
                }
            }
            let n = (SS * SS) as f32;
            let (r, g, b, a) = (r / n, g / n, b / n, a / n);
            let out = |c: f32| ((c + checker * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8;
            pixels.push(Color32::from_rgb(out(r), out(g), out(b)));
        }
    }
    ColorImage::new([tw, th], pixels)
}

/// A mask on its background tone (white or black), placed where it sits on the canvas.
fn mask_thumbnail(project: &Project, layer: &LayerRecord, tw: usize, th: usize) -> ColorImage {
    let (cw, ch) = (project.manifest.width as f32, project.manifest.height as f32);
    let Some(mask) = project.masks.get(&layer.id) else { return ColorImage::new([tw, th], vec![Color32::WHITE; tw * th]) };
    let (iw, ih) = mask.pixels.dimensions();
    let background = if mask.pixels.get_pixel(0, 0).0[0] >= 128 { 1.0 } else { 0.0 };
    let placement = layer.mask_placement.unwrap_or(layer.transform);
    let mut pixels = Vec::with_capacity(tw * th);
    const SS: usize = 4;
    for j in 0..th {
        for i in 0..tw {
            let mut sum = 0.0f32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (i as f32 + (sx as f32 + 0.5) / SS as f32) / tw as f32 * cw;
                    let y = (j as f32 + (sy as f32 + 0.5) / SS as f32) / th as f32 * ch;
                    sum += match placed(&placement, iw, ih, x, y) {
                        Some((px, py)) => mask.pixels.get_pixel(px, py).0[0] as f32 / 255.0,
                        None => background,
                    };
                }
            }
            let v = (sum / (SS * SS) as f32 * 255.0).round() as u8;
            pixels.push(Color32::from_gray(v));
        }
    }
    ColorImage::new([tw, th], pixels)
}

