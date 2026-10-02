//! The tool header bar (options bar): one header per tool, 42 points tall, 12-point controls,
//! 18 points of padding at each end (docs/port/ui-inventory.md §2.3).

use crate::app::App;
use crate::icons::Icon;
use crate::theme::{self, color, white_alpha};
use crate::tools::*;
use crate::widgets::{self as w, ButtonStyle, SwatchStyle};
use eframe::egui::{self, Align, CornerRadius, Layout, Rect, Sense, Ui, pos2, vec2};

pub fn tool_header(app: &mut App, ui: &mut Ui, rect: Rect) {
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    ui.style_mut().text_styles.insert(egui::TextStyle::Button, theme::regular(12.0));
    let ui = &mut ui;
    let key = std::mem::discriminant(&app.tool);
    match app.tool {
        Tool::Move => transform(app, ui, rect),
        Tool::Brush | Tool::SpotHealing | Tool::CloneStamp | Tool::Blur => bar(ui, rect, 12.0, key, |ui| brush(app, ui)),
        Tool::Marquee | Tool::Lasso | Tool::Wand => bar(ui, rect, 12.0, key, |ui| selection(app, ui)),
        Tool::Gradient => bar(ui, rect, 12.0, key, |ui| gradient(app, ui)),
        Tool::Type => bar(ui, rect, 12.0, key, |ui| type_tool(app, ui)),
        Tool::Shape => bar(ui, rect, 12.0, key, |ui| shape(app, ui)),
        Tool::Crop => bar(ui, rect, 14.0, key, |ui| crop(app, ui)),
        Tool::Hand | Tool::Zoom => bar(ui, rect, 12.0, key, |ui| navigation(app, ui)),
        Tool::Eyedropper => bar(ui, rect, 16.0, key, |ui| {
            w::title(ui, "Eyedropper");
            w::checkbox(ui, &mut app.settings.sample_ring, "Sample Ring");
        }),
        Tool::Idle => bar(ui, rect, 16.0, key, |ui| {
            w::title(ui, "Select a tool");
        }),
    }
}

/// A header's HStack inside its 18-point padding. When its controls are wider than the bar,
/// SwiftUI centers the stack, so it overflows at both ends and the window clips it; the width
/// comes from the previous frame. (SwiftUI also squeezes some labels first, which isn't copied.)
fn bar(ui: &mut Ui, rect: Rect, spacing: f32, key: impl std::hash::Hash + std::fmt::Debug, content: impl FnOnce(&mut Ui)) {
    let pad = theme::metric::HEADER_PADDING;
    let inner = rect.shrink2(vec2(pad, 0.0));
    let id = ui.id().with(("header-width", key));
    let last: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let overflow = (last - inner.width()).max(0.0);
    let row = Rect::from_min_max(inner.min - vec2(overflow / 2.0, 0.0), inner.max + vec2(overflow / 2.0, 0.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(Layout::left_to_right(Align::Center)));
    child.spacing_mut().item_spacing = vec2(spacing, 0.0);
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    content(&mut child);
    let used = child.min_rect().width();
    ui.data_mut(|d| d.insert_temp(id, used));
}

/// A caption-sized, secondary scrub label, as the Transform fields use.
fn caption_scrub(ui: &mut Ui, s: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, enabled: bool) -> egui::Response {
    w::scrub_label(ui, s, theme::regular(10.0), color::secondary(), value, 1.0, range, enabled)
}

/// A 12-point scrub label in the label color.
fn scrub(ui: &mut Ui, s: &str, value: &mut f64, sensitivity: f64, range: std::ops::RangeInclusive<f64>) -> egui::Response {
    w::scrub_label(ui, s, theme::regular(12.0), color::label(), value, sensitivity, range, true)
}

/// Label, slider and a percent field for a 0…1 value.
fn percent_row(ui: &mut Ui, id: &str, title: &str, value: &mut f64, min: f64) {
    let mut pct = *value * 100.0;
    scrub(ui, title, &mut pct, 1.0, min * 100.0..=100.0);
    w::slider(ui, &mut pct, min * 100.0..=100.0, 100.0, true);
    w::number_field(ui, id, &mut pct, min * 100.0..=100.0, w::fmt_int, 42.0, false, true);
    w::unit(ui, "%");
    *value = pct / 100.0;
}

/// `TransformInspector`: Auto Select, Show Controls, then X, Y, W, H, lock, Scale, angle,
/// Sampling and the flips for the active layer in a horizontal ScrollView, and the Cancel and
/// Apply buttons, which keep their place while hidden. A value being typed or dragged is one
/// undo step.
fn transform(app: &mut App, ui: &mut Ui, rect: Rect) {
    // `canTransform`: an active pixel layer that is visible, its folders too.
    let doc_layer = app.doc().and_then(|d| {
        let l = d.active_layer()?;
        if l.is_group() || l.adjustment.is_some() || !d.effectively_visible(&l.id) {
            return None;
        }
        let pixels = d.project.images.get(&l.id).map(|a| [a.pixels.width() as f64, a.pixels.height() as f64])?;
        // A drag on the canvas shows its draft.
        let t = match &app.gesture {
            Some(crate::ui::canvas_tools::Gesture::Transform { layer, draft, .. }) if *layer == l.id => *draft,
            _ => l.transform,
        };
        Some((l.id.clone(), t, Some(pixels)))
    });
    let enabled = doc_layer.is_some();
    // With nothing to transform the fields show the active layer's box, or a unit box.
    let (mut x, mut y, mut wd, mut ht, mut scale, mut angle) = match &doc_layer {
        Some((_, t, pixels)) => (t.origin[0], t.origin[1], t.size[0], t.size[1], pixels.map_or(100.0, |p| t.size[0] / p[0].max(1.0) * 100.0), t.rotation),
        None => match app.doc().and_then(|d| d.active_layer()) {
            Some(l) => (l.transform.origin[0], l.transform.origin[1], l.transform.size[0], l.transform.size[1], 100.0, l.transform.rotation),
            None => (0.0, 0.0, 1.0, 1.0, 100.0, 0.0),
        },
    };
    let (x0, y0, w0, h0, scale0, angle0) = (x, y, wd, ht, scale, angle);
    let mut sampling = doc_layer.as_ref().map_or(Sampling::High, |(_, t, _)| match t.sampling {
        comp_format::Sampling::Nearest => Sampling::Nearest,
        comp_format::Sampling::Smooth => Sampling::Smooth,
        comp_format::Sampling::High => Sampling::High,
    });
    let sampling0 = sampling;
    let mut flip = None;
    // The Cancel and Apply buttons at the trailing end, transparent until an edit waits on them.
    let font = theme::regular(12.0);
    let buttons: f32 = ["Cancel", "Apply"].iter().map(|t| ui.painter().layout_no_wrap(t.to_string(), font.clone(), color::label()).size().x.ceil() + 26.0).sum::<f32>() + 12.0;
    let bar = rect.with_max_x(rect.max.x - 18.0);
    let mut scroll_start = 0.0;
    w::row(ui, bar.with_min_x(bar.min.x + 18.0), 12.0, |ui| {
        w::title(ui, "Transform");
        w::checkbox(ui, &mut app.settings.auto_select, "Auto Select");
        w::checkbox(ui, &mut app.settings.show_controls, "Show Controls");
        scroll_start = ui.cursor().min.x;
    });
    let scroll = Rect::from_min_max(pos2(scroll_start, rect.min.y), pos2(bar.max.x - buttons - 12.0, rect.max.y));
    // The ScrollView's content: an HStack 12 apart with 18 points of padding, clipped.
    let caption = |ui: &mut Ui, s: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>| {
        caption_scrub(ui, s, value, range, enabled);
    };
    w::row(ui, scroll.shrink2(vec2(18.0, 0.0)).with_max_x(rect.max.x + 400.0), 12.0, |ui| {
        ui.set_clip_rect(scroll.intersect(rect));
        for (label, value, range) in [
            ("X", &mut x, -30000.0..=30000.0),
            ("Y", &mut y, -30000.0..=30000.0),
            ("W", &mut wd, 1.0..=30000.0),
            ("H", &mut ht, 1.0..=30000.0),
        ] {
            // `.frame(width: 85)` around HStack(spacing: 4) { label, field }.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let start = ui.cursor().min.x;
                caption(ui, label, value, range.clone());
                let field = 85.0 - (ui.cursor().min.x - start);
                w::number_field(ui, ("transform", label), value, range, w::fmt_whole_or_2, field, false, enabled);
            });
        }
        lock_toggle(ui, &mut app.settings.lock_aspect, enabled);
        ui.horizontal(|ui| {
            // 110 points for the caption, the field and its caption-sized unit.
            ui.spacing_mut().item_spacing.x = 4.0;
            let start = ui.cursor().min.x;
            caption(ui, "Scale", &mut scale, 0.1..=30000.0);
            let percent = ui.painter().layout_no_wrap("%".into(), theme::regular(10.0), color::secondary()).size().x.ceil();
            let field = 110.0 - (ui.cursor().min.x - start) - 4.0 - percent;
            w::number_field(ui, ("transform", "scale"), &mut scale, 0.1..=30000.0, w::fmt_whole_or_2, field, false, enabled);
            w::text(ui, "%", theme::regular(10.0), color::secondary());
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let start = ui.cursor().min.x;
            caption(ui, "°", &mut angle, -360.0..=360.0);
            let field = 75.0 - (ui.cursor().min.x - start);
            w::number_field(ui, ("transform", "angle"), &mut angle, -360.0..=360.0, w::fmt_whole_or_2, field, false, enabled);
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            w::labeled_popup(ui, "Sampling", &mut sampling, &[Sampling::ALL], Sampling::title, 170.0, enabled);
        });
        if w::button(ui, "Flip H", 12.0, ButtonStyle::Bordered, enabled).clicked() {
            flip = Some(true);
        }
        if w::button(ui, "Flip V", 12.0, ButtonStyle::Bordered, enabled).clicked() {
            flip = Some(false);
        }
    });
    app.settings.transform_sampling = sampling;
    if let Some((id, t, pixels)) = doc_layer {
        let pixels = pixels.unwrap_or([1.0, 1.0]);
        let lock = app.settings.lock_aspect;
        let mut next = t;
        // Typed values are used as they are (only dragging rounds).
        if x != x0 || y != y0 {
            next.origin = [x, y];
        }
        if wd != w0 || ht != h0 {
            let c = crate::geometry::center(&t);
            let (mut nw, mut nh) = (wd, ht);
            if lock {
                if wd != w0 {
                    nh = wd * t.size[1] / t.size[0];
                } else {
                    nw = ht * t.size[0] / t.size[1];
                }
            }
            next.size = [nw.max(1.0), nh.max(1.0)];
            next.origin = [c[0] - next.size[0] / 2.0, c[1] - next.size[1] / 2.0];
        }
        if scale != scale0 {
            next = crate::geometry::scaled_to_percent(&t, scale, pixels);
        }
        if angle != angle0 {
            next.rotation = angle;
        }
        if sampling != sampling0 {
            next.sampling = match sampling {
                Sampling::Nearest => comp_format::Sampling::Nearest,
                Sampling::Smooth => comp_format::Sampling::Smooth,
                Sampling::High => comp_format::Sampling::High,
            };
        }
        if let Some(h) = flip {
            if let Some(d) = app.doc_mut() {
                crate::layer_ops::flip_layer(d, h);
            }
        } else if next != t && crate::geometry::is_valid(&next) {
            if let Some(d) = app.doc_mut() {
                let project = crate::ui::canvas_tools::placed(&d.project, &id, next);
                d.edit("Transform Layer", true, |m| *m = project.manifest);
            }
        } else if let Some(d) = app.doc_mut() {
            if !ui.ctx().egui_is_using_pointer() && !ui.ctx().egui_wants_keyboard_input() {
                d.end_coalescing();
            }
        }
    }
}

/// The lock-aspect toggle, a `.button`-style toggle showing the `link` symbol: an accent
/// capsule when on, measured 41 × 24 on the Mac.
fn lock_toggle(ui: &mut Ui, on: &mut bool, enabled: bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(41.0, theme::metric::CONTROL_HEIGHT), if enabled { Sense::click() } else { Sense::hover() });
    if response.clicked() {
        *on = !*on;
    }
    let dim = if enabled { 1.0 } else { 0.44 };
    let fill = if *on { color::ACCENT } else { color::control() };
    ui.painter().rect_filled(rect, CornerRadius::same(12), fill.gamma_multiply(dim));
    let symbol = if *on { egui::Color32::WHITE } else { color::label() };
    crate::icons::paint(ui.painter(), Icon::Symbol("link"), rect.center(), 12.0, symbol.gamma_multiply(if enabled { 1.0 } else { 0.55 }));
}

fn tip_mut(app: &mut App) -> &mut Tip {
    match app.tool {
        Tool::CloneStamp => &mut app.settings.clone,
        Tool::Blur => &mut app.settings.smear,
        _ => &mut app.settings.brush,
    }
}

/// `BrushControls`: Brush, Eraser, Spot Healing, Clone Stamp and Smear.
fn brush(app: &mut App, ui: &mut Ui) {
    let tool = app.tool;
    let s = &mut app.settings;
    let title = match tool {
        Tool::SpotHealing => "Spot Healing",
        Tool::CloneStamp => "Clone Stamp",
        Tool::Blur => "Smear",
        _ if s.brush_mode == BrushMode::Erase => "Eraser",
        _ => "Brush",
    };
    w::title(ui, title);
    match tool {
        Tool::Brush => {
            w::segmented(ui, &mut s.brush_mode, BrushMode::ALL, BrushMode::title);
        }
        Tool::Blur => {
            w::segmented(ui, &mut s.smear_mode, SmearMode::ALL, SmearMode::title);
        }
        Tool::SpotHealing => {
            w::segmented(ui, &mut s.healing, HealingType::ALL, HealingType::title);
        }
        Tool::CloneStamp => {
            w::checkbox(ui, &mut s.clone_aligned, "Aligned");
            w::segmented(ui, &mut s.clone_sample, SampleLayers::ALL, SampleLayers::title);
        }
        _ => {}
    }
    let smear_mode = s.smear_mode;
    let brush_mode = s.brush_mode;
    let foreground = s.foreground;
    let tip = tip_mut(app);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        scrub(ui, "Size", &mut tip.size, 1.0, 1.0..=2000.0);
        w::number_field(ui, "brush-size", &mut tip.size, 1.0..=2000.0, w::fmt_int, 48.0, false, true);
        w::unit(ui, "px");
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        percent_row(ui, "brush-hardness", "Hardness", &mut tip.hardness, 0.0);
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        percent_row(ui, "brush-opacity", if tool == Tool::Blur { "Strength" } else { "Opacity" }, &mut tip.opacity, 0.01);
    });
    let s = &mut app.settings;
    if tool == Tool::Blur && smear_mode == SmearMode::Blur {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            scrub(ui, "Radius", &mut s.smear_radius, 0.1, 0.5..=50.0);
            let mut slider = s.smear_radius.min(20.0);
            if w::slider(ui, &mut slider, 0.5..=20.0, 100.0, true).changed() {
                s.smear_radius = slider;
            }
            w::number_field(ui, "smear-radius", &mut s.smear_radius, 0.5..=50.0, w::fmt_trim1, 42.0, false, true);
            w::unit(ui, "px");
        });
    }
    if tool == Tool::Brush {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            scrub(ui, "Smoothing", &mut s.smoothing, 1.0, 0.0..=100.0);
            w::slider(ui, &mut s.smoothing, 0.0..=100.0, 100.0, true);
            w::number_field(ui, "smoothing", &mut s.smoothing, 0.0..=100.0, w::fmt_int, 42.0, false, true);
        });
    }
    let mask = app.doc().is_some_and(|d| d.mask_target && d.active_layer().is_some_and(|l| l.mask_file.is_some()));
    let s = &mut app.settings;
    if mask {
        // On a mask the brush paints black or white.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            w::labeled_popup(ui, "Paint", &mut s.mask_paint, &[MaskPaint::ALL], MaskPaint::title, 180.0, true);
        });
    } else if matches!(tool, Tool::Brush | Tool::SpotHealing) {
        let mut open_picker = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            w::label(ui, "Color");
            open_picker = w::swatch(ui, w::rgb(foreground), vec2(34.0, 18.0), SwatchStyle { radius: 4.0, inner_white: 1.0, outer_black: 1.0 }).clicked();
        });
        if open_picker {
            crate::ui::dialogs::open_color_picker(app, false);
        }
    }
    let _ = brush_mode;
    let source = app.doc().is_some_and(|d| d.clone_source.is_some());
    if tool == Tool::CloneStamp && !source {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            w::secondary(ui, "Option-click to set the source", 12.0);
        });
    } else if mask {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            w::secondary(ui, "Mask", 12.0);
        });
    }
}

/// `LassoControls`: Marquee, Lasso and Magic.
fn selection(app: &mut App, ui: &mut Ui) {
    let tool = app.tool;
    let s = &mut app.settings;
    w::title(ui, match tool {
        Tool::Marquee => "Marquee",
        Tool::Lasso => "Lasso",
        _ => "Magic",
    });
    match tool {
        Tool::Marquee => {
            w::segmented(ui, &mut s.marquee, MarqueeKind::ALL, MarqueeKind::title);
        }
        Tool::Wand => {
            w::segmented(ui, &mut s.wand, WandMode::ALL, WandMode::title);
        }
        _ => {
            w::segmented(ui, &mut s.lasso, LassoKind::ALL, LassoKind::title);
        }
    }
    w::segmented(ui, &mut s.selection_mode, SelectionMode::ALL, SelectionMode::title);
    if tool == Tool::Wand && s.wand == WandMode::Wand {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            scrub(ui, "Tolerance", &mut s.tolerance, 1.0, 0.0..=255.0);
            w::number_field(ui, "tolerance", &mut s.tolerance, 0.0..=255.0, w::fmt_int, 44.0, true, true);
        });
        w::popup(ui, "sample-size", &mut s.sample_size, &[SampleSize::ALL], SampleSize::title, None, true);
        w::segmented(ui, &mut s.wand_layers, SampleLayers::ALL, SampleLayers::title);
        w::checkbox(ui, &mut s.contiguous, "Contiguous");
    }
    if tool == Tool::Wand && s.wand == WandMode::Object {
        w::segmented(ui, &mut s.object_layers, SampleLayers::ALL, SampleLayers::title);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            scrub(ui, "Edge", &mut s.object_edge, 1.0, -10.0..=10.0);
            w::number_field(ui, "edge", &mut s.object_edge, -10.0..=10.0, w::fmt_int, 40.0, false, true);
            w::unit(ui, "px");
        });
    }
    if tool == Tool::Lasso || tool == Tool::Wand || s.marquee == MarqueeKind::Ellipse {
        w::checkbox(ui, &mut s.anti_alias, "Anti-alias");
    }
    w::vdivider(ui, 18.0);
    // Disabled without a selection, as on the Mac.
    let has_selection = app.doc().is_some_and(|d| d.selection.is_some());
    let s = &mut app.settings;
    let mut chosen = None;
    for (kind, (title, value, max, width)) in [("Expand", &mut s.expand, 500.0, 40.0), ("Contract", &mut s.contract, 500.0, 40.0), ("Feather", &mut s.feather, 250.0, 48.0)].into_iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            if w::button(ui, title, 12.0, ButtonStyle::Bordered, has_selection).clicked() {
                chosen = Some((kind as u8, *value));
            }
            // Right-aligned. Expand and Contract disable their whole group without a selection;
            // Feather only its button.
            w::number_field(ui, ("amount", title), value, 1.0..=max, w::fmt_int, width, true, has_selection || title == "Feather");
            w::unit(ui, "px");
        });
    }
    let mut deselect = false;
    if has_selection {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            deselect = w::button(ui, "Deselect", 12.0, ButtonStyle::Bordered, true).clicked();
        });
    }
    if let Some((kind, amount)) = chosen {
        crate::ui::selection::modify(app, kind, amount.round().max(1.0));
    }
    if deselect {
        crate::ui::selection::command(app, crate::menus::Command::Deselect);
    }
}

/// `GradientControls`.
fn gradient(app: &mut App, ui: &mut Ui) {
    let s = &mut app.settings;
    w::title(ui, "Gradient");
    w::segmented(ui, &mut s.gradient, GradientKind::ALL, GradientKind::title);
    // Preview: a 4-point checkerboard (gray 45% over white) under the gradient of the current colors.
    let (rect, _) = ui.allocate_exact_size(vec2(56.0, 18.0), Sense::hover());
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 3.0, egui::Color32::WHITE);
    for row in 0..5 {
        for col in 0..14 {
            if (row + col) % 2 == 0 {
                let r = Rect::from_min_size(rect.min + vec2(col as f32 * 4.0, row as f32 * 4.0), vec2(4.0, 4.0));
                p.rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(128, 128, 128, 115));
            }
        }
    }
    let fg = w::rgb(s.foreground);
    let end = match s.gradient_colors {
        GradientColors::ToBackground => w::rgb(s.background),
        GradientColors::ToTransparent => egui::Color32::TRANSPARENT,
    };
    let (a, b) = if s.gradient_reverse { (end, fg) } else { (fg, end) };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), a);
    mesh.colored_vertex(rect.right_top(), b);
    mesh.colored_vertex(rect.right_bottom(), b);
    mesh.colored_vertex(rect.left_bottom(), a);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::black_alpha(0.5)), egui::StrokeKind::Inside);
    w::popup(ui, "gradient-colors", &mut s.gradient_colors, &[GradientColors::ALL], GradientColors::title, None, true);
    w::checkbox(ui, &mut s.gradient_reverse, "Reverse");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        percent_row(ui, "gradient-opacity", "Opacity", &mut s.gradient_opacity, 0.0);
    });
}

/// `TypeControls`.
fn type_tool(app: &mut App, ui: &mut Ui) {
    let editable = app.doc().and_then(|d| d.active_layer()).is_some_and(|l| l.text.is_some());
    let s = &mut app.settings;
    w::title(ui, "Type");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // The font list is the system's; Helvetica stands in until text rendering exists.
        let mut font = 0usize;
        w::popup(ui, "font", &mut font, &[&[0usize]], |_| "Helvetica", Some(210.0), true);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            w::number_field(ui, "font-size", &mut s.font_size, 1.0..=2000.0, w::fmt_int, 52.0, false, true);
            w::unit(ui, "px");
        });
        w::swatch(ui, w::rgb(s.text_color), vec2(36.0, 18.0), SwatchStyle { radius: 3.0, inner_white: 0.0, outer_black: 0.5 });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (align, symbol) in [(Alignment::Left, "text.alignleft"), (Alignment::Center, "text.aligncenter"), (Alignment::Right, "text.alignright")] {
                let (rect, response) = ui.allocate_exact_size(vec2(30.0, 26.0), Sense::click());
                if s.alignment == align {
                    ui.painter().rect_filled(rect, 4.0, white_alpha(0.14));
                }
                crate::icons::paint(ui.painter(), Icon::Symbol(symbol), rect.center(), 13.0, color::label());
                if response.clicked() {
                    s.alignment = align;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            scrub(ui, "Tracking", &mut s.tracking, 1.0, -100.0..=1000.0);
            w::number_field(ui, "tracking", &mut s.tracking, -100.0..=1000.0, w::fmt_int, 45.0, false, true);
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            scrub(ui, "Leading", &mut s.leading, 1.0, 0.0..=5000.0);
            if s.leading == 0.0 {
                let mut text = String::new();
                w::text_field(ui, "leading-auto", &mut text, 52.0, "Auto", theme::regular(12.0));
                if let Ok(v) = text.trim().parse::<f64>() {
                    s.leading = v.clamp(0.0, 5000.0);
                }
            } else {
                w::number_field(ui, "leading", &mut s.leading, 0.0..=5000.0, w::fmt_int, 52.0, false, true);
            }
        });
    });
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        w::button(ui, "Edit Text", 12.0, ButtonStyle::Bordered, editable);
    });
}

/// `ShapeControls`.
fn shape(app: &mut App, ui: &mut Ui) {
    let s = &mut app.settings;
    w::title(ui, "Shape");
    w::segmented(ui, &mut s.shape, ShapeKind::ALL, ShapeKind::title);
    match s.shape {
        ShapeKind::Line => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                scrub(ui, "Width", &mut s.line_width, 1.0, 1.0..=5000.0);
                let mut slider = s.line_width.min(100.0);
                if w::slider(ui, &mut slider, 1.0..=100.0, 100.0, true).changed() {
                    s.line_width = slider.round();
                }
                w::number_field(ui, "line-width", &mut s.line_width, 1.0..=5000.0, w::fmt_int, 48.0, true, true);
                w::unit(ui, "px");
            });
        }
        ShapeKind::Rectangle => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                scrub(ui, "Radius", &mut s.corner_radius, 1.0, 0.0..=5000.0);
                let mut slider = s.corner_radius.min(200.0);
                if w::slider(ui, &mut slider, 0.0..=200.0, 100.0, true).changed() {
                    s.corner_radius = slider.round();
                }
                w::number_field(ui, "corner-radius", &mut s.corner_radius, 0.0..=5000.0, w::fmt_int, 48.0, true, true);
                w::unit(ui, "px");
            });
        }
        ShapeKind::Ellipse => {}
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        w::label(ui, "Fill");
        w::swatch(ui, w::rgb(s.foreground), vec2(36.0, 18.0), SwatchStyle { radius: 3.0, inner_white: 0.0, outer_black: 0.5 });
    });
}

/// `CropControls`.
fn crop(app: &mut App, ui: &mut Ui) {
    let size = app.doc().and_then(|d| d.visible_crop()).map(|c| (c[2] as i64, c[3] as i64));
    let pending = app.doc().is_some_and(|d| d.crop.is_some());
    w::title(ui, "Crop");
    let before = app.settings.crop_ratio;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        w::labeled_popup(ui, "Ratio", &mut app.settings.crop_ratio, &[CropRatio::ALL], CropRatio::title, 170.0, true);
    });
    // `changeCropRatio`: the frame takes the new ratio around its middle.
    if app.settings.crop_ratio != before {
        if let Some(ratio) = app.crop_ratio() {
            if let Some(d) = app.doc_mut() {
                if let Some(r) = d.visible_crop() {
                    let height = r[2] / ratio;
                    let next = crate::geometry::crop_snapped([r[0], r[1] + r[3] / 2.0 - height / 2.0, r[2], height]);
                    if crate::geometry::crop_valid(next) {
                        d.crop = Some(next);
                    }
                }
            }
        }
    }
    if let Some((wd, ht)) = size {
        w::label(ui, &format!("{wd} × {ht} px"));
    }
    let mut apply = false;
    let mut cancel = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        apply = w::button(ui, "Apply Crop", 12.0, ButtonStyle::Bordered, pending).clicked();
        cancel = w::button(ui, "Cancel", 12.0, ButtonStyle::Bordered, pending).clicked();
    });
    if apply {
        crate::ui::canvas_tools::apply_crop(app);
    }
    if cancel {
        if let Some(d) = app.doc_mut() {
            d.crop = None;
        }
    }
}

/// `NavigationToolHeader`: Pan, or Zoom with the zoom field.
fn navigation(app: &mut App, ui: &mut Ui) {
    if app.tool == Tool::Hand {
        w::title(ui, "Pan");
        return;
    }
    w::title(ui, "Zoom");
    let zoom = app.doc().map(|d| d.view.zoom as f64 * 100.0);
    let mut value = zoom.unwrap_or(100.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let response = w::number_field(ui, "zoom-field", &mut value, 0.1..=3200.0, w::fmt_trim2, 72.0, true, zoom.is_some());
        w::unit(ui, "%");
        if response.lost_focus() && zoom.is_some_and(|z| z != value) {
            app.zoom_to((value / 100.0) as f32);
        }
    });
}
