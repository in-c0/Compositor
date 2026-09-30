//! AppKit's controls in the dark appearance, drawn with egui: checkboxes, segmented pickers,
//! sliders, rounded-border fields, capsule buttons and pop-ups, color swatches, scrubbable labels.

use crate::icons::{self, Icon};
use crate::theme::{self, black_alpha, color, metric, white_alpha};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::hash::Hash;
use std::ops::RangeInclusive;

pub fn rgb(c: [f32; 3]) -> Color32 {
    Color32::from_rgb((c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8)
}

/// A non-interactive label.
pub fn text(ui: &mut Ui, s: impl Into<String>, font: FontId, color: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(s.into(), font, color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
    response
}

/// A header's title: 13 pt semibold.
pub fn title(ui: &mut Ui, s: &str) -> Response {
    text(ui, s, theme::semibold(13.0), color::label())
}

/// A control label at the header size (12 pt).
pub fn label(ui: &mut Ui, s: &str) -> Response {
    text(ui, s, theme::regular(12.0), color::label())
}

pub fn secondary(ui: &mut Ui, s: &str, size: f32) -> Response {
    text(ui, s, theme::regular(size), color::secondary())
}

/// A label that changes `value` when dragged sideways (`scrubbable`), `sensitivity` per point.
pub fn scrub_label(ui: &mut Ui, s: &str, font: FontId, color: Color32, value: &mut f64, sensitivity: f64, range: RangeInclusive<f64>, enabled: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(s.to_string(), font, color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), if enabled { Sense::drag() } else { Sense::hover() });
    ui.painter().galley(rect.min, galley, if enabled { color } else { color.gamma_multiply(0.45) });
    if enabled {
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if response.dragged() {
            let dx = response.drag_delta().x as f64;
            *value = (*value + dx * sensitivity).clamp(*range.start(), *range.end());
        }
        return response;
    }
    response
}

/// The Mac's checkbox: 14 pt rounded box, accent when on, then the title.
pub fn checkbox(ui: &mut Ui, value: &mut bool, title: &str) -> Response {
    let font = theme::regular(12.0);
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, color::label());
    let size = vec2(14.0 + 6.0 + galley.size().x, metric::CONTROL_HEIGHT);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let p = ui.painter();
    let bx = Rect::from_center_size(pos2(rect.min.x + 7.0, rect.center().y), vec2(14.0, 14.0));
    if *value {
        p.rect_filled(bx, 3.5, color::ACCENT);
        let s = Stroke::new(1.8, Color32::WHITE);
        p.line(vec![pos2(bx.min.x + 3.5, bx.center().y + 0.2), pos2(bx.min.x + 6.0, bx.max.y - 3.8), pos2(bx.max.x - 3.2, bx.min.y + 3.6)], s);
    } else {
        p.rect_filled(bx, 3.5, white_alpha(0.08));
        p.rect_stroke(bx, 3.5, Stroke::new(1.0, white_alpha(0.22)), StrokeKind::Inside);
    }
    p.galley(pos2(bx.max.x + 6.0, rect.center().y - galley.size().y / 2.0), galley, color::label());
    response
}

/// A segmented picker (`.pickerStyle(.segmented)`), capsule track with a lighter selected pill.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, value: &mut T, all: &[T], title: impl Fn(T) -> &'static str) -> Response {
    let font = theme::regular(12.0);
    let galleys: Vec<_> = all.iter().map(|v| ui.painter().layout_no_wrap(title(*v).to_string(), font.clone(), color::label())).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 22.0).collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (rect, mut response) = ui.allocate_exact_size(vec2(total, metric::CONTROL_HEIGHT), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(11), white_alpha(0.10));
    let mut x = rect.min.x + 2.0;
    let selected = all.iter().position(|v| *v == *value);
    for (i, (g, w)) in galleys.into_iter().zip(&widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.min.y + 2.0), vec2(*w, rect.height() - 4.0));
        if Some(i) == selected {
            p.rect_filled(seg, CornerRadius::same(9), white_alpha(0.26));
        } else if i > 0 && Some(i - 1) != selected {
            p.line_segment([pos2(x, rect.min.y + 6.0), pos2(x, rect.max.y - 6.0)], Stroke::new(1.0, white_alpha(0.12)));
        }
        p.galley(seg.center() - g.size() / 2.0, g, color::label());
        if response.clicked() && response.interact_pointer_pos().is_some_and(|pos| seg.contains(pos)) && Some(i) != selected {
            *value = all[i];
            response.mark_changed();
        }
        x += w;
    }
    response
}

/// A horizontal slider, `width` wide: 4 pt track filled with the accent up to the knob.
pub fn slider(ui: &mut Ui, value: &mut f64, range: RangeInclusive<f64>, width: f32, enabled: bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, metric::CONTROL_HEIGHT), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let (lo, hi) = (*range.start(), *range.end());
    let knob_r = 8.0;
    let track = Rect::from_min_max(pos2(rect.min.x + knob_r, rect.center().y - 2.0), pos2(rect.max.x - knob_r, rect.center().y + 2.0));
    if enabled && (response.dragged() || response.clicked()) {
        if let Some(pos) = response.interact_pointer_pos() {
            // A track click jumps the knob there (`SliderSnap`), as does a drag.
            let t = ((pos.x - track.min.x) / track.width()).clamp(0.0, 1.0) as f64;
            let v = lo + t * (hi - lo);
            if v != *value {
                *value = v;
                response.mark_changed();
            }
        }
    }
    let t = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let p = ui.painter();
    let dim = if enabled { 1.0 } else { 0.45 };
    p.rect_filled(track, 2.0, white_alpha(0.2 * dim));
    let knob_x = track.min.x + t * track.width();
    p.rect_filled(Rect::from_min_max(track.min, pos2(knob_x, track.max.y)), 2.0, color::ACCENT.gamma_multiply(dim));
    let center = pos2(knob_x, rect.center().y);
    p.circle_filled(center + vec2(0.0, 0.5), knob_r, black_alpha(0.25));
    p.circle_filled(center, knob_r - 0.5, theme::gray(0.84).gamma_multiply(dim));
    response
}

/// Whole numbers.
pub fn fmt_int(v: f64) -> String {
    let r = v.round();
    if r == 0.0 { "0".into() } else { format!("{r}") }
}

/// Up to `n` decimals, trailing zeros trimmed.
fn trimmed(v: f64, n: usize) -> String {
    let s = format!("{v:.n$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s }
}

pub fn fmt_trim1(v: f64) -> String {
    trimmed(v, 1)
}

pub fn fmt_trim2(v: f64) -> String {
    trimmed(v, 2)
}

/// The Transform bar's fields: an integer when whole, else two decimals.
pub fn fmt_whole_or_2(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { fmt_int(v) } else { format!("{v:.2}") }
}

/// A rounded-border field holding a number, clamped to `range`, shown through `fmt`.
pub fn number_field(ui: &mut Ui, id_salt: impl Hash + std::fmt::Debug, value: &mut f64, range: RangeInclusive<f64>, fmt: fn(f64) -> String, width: f32, right: bool, enabled: bool) -> Response {
    let id = ui.make_persistent_id(id_salt);
    let buf_id = id.with("text");
    let focused = ui.memory(|m| m.has_focus(id));
    let mut buf = if focused { ui.data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| fmt(*value)) } else { fmt(*value) };
    let response = field_frame(ui, width, enabled, |ui, rect| {
        let edit = egui::TextEdit::singleline(&mut buf)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(theme::regular(12.0))
            .horizontal_align(if right { Align::Max } else { Align::Min })
            .vertical_align(Align::Center)
            .margin(egui::Margin::symmetric(0, 0));
        ui.add_enabled_ui(enabled, |ui| ui.put(rect.shrink2(vec2(6.0, 2.0)), edit)).inner
    });
    let mut response = response;
    if response.changed() {
        if let Ok(v) = buf.trim().trim_end_matches(['%', '°']).trim().parse::<f64>() {
            *value = v.clamp(*range.start(), *range.end());
        }
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(buf_id, buf));
    } else {
        ui.data_mut(|d| d.remove::<String>(buf_id));
    }
    if response.lost_focus() {
        response.mark_changed();
    }
    response
}

/// A rounded-border text field for free text, with an optional placeholder.
pub fn text_field(ui: &mut Ui, id_salt: impl Hash + std::fmt::Debug, value: &mut String, width: f32, placeholder: &str, font: FontId) -> Response {
    let id = ui.make_persistent_id(id_salt);
    field_frame(ui, width, true, |ui, rect| {
        let edit = egui::TextEdit::singleline(value)
            .id(id)
            .frame(egui::Frame::NONE)
            .font(font)
            .hint_text(placeholder)
            .vertical_align(Align::Center)
            .margin(egui::Margin::symmetric(0, 0));
        ui.put(rect.shrink2(vec2(6.0, 2.0)), edit)
    })
}

fn field_frame(ui: &mut Ui, width: f32, enabled: bool, content: impl FnOnce(&mut Ui, Rect) -> Response) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, metric::CONTROL_HEIGHT), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 5.0, color::field());
    p.rect_stroke(rect, 5.0, Stroke::new(1.0, color::field_border()), StrokeKind::Inside);
    let response = content(ui, rect);
    if !enabled {
        ui.painter().rect_filled(rect, 5.0, theme::gray(0.14).gamma_multiply(0.4));
    }
    response
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    Bordered,
    Prominent,
}

/// A bordered push button with the capsule shape `.roundedControls()` gives every button.
pub fn button(ui: &mut Ui, title: &str, size: f32, style: ButtonStyle, enabled: bool) -> Response {
    let font = theme::regular(size);
    let text_color = match style {
        ButtonStyle::Prominent => Color32::WHITE,
        ButtonStyle::Bordered => color::label(),
    };
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, text_color);
    let height = if size > 12.0 { 24.0 } else { metric::CONTROL_HEIGHT };
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 22.0, height), if enabled { Sense::click() } else { Sense::hover() });
    let pressed = response.is_pointer_button_down_on();
    let fill = match style {
        ButtonStyle::Prominent => if pressed { color::ACCENT.gamma_multiply(0.8) } else { color::ACCENT },
        ButtonStyle::Bordered => if pressed { color::control_pressed() } else { color::control() },
    };
    let dim = if enabled { 1.0 } else { 0.4 };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((height / 2.0) as u8), fill.gamma_multiply(dim));
    p.galley(rect.center() - galley.size() / 2.0, galley, text_color.gamma_multiply(dim));
    response
}

/// A pop-up button (`NSPopUpButton` / `.pickerStyle(.menu)`): capsule, title, up-down chevrons.
/// `width` fixes the button's width; `None` fits the title. Returns true when the value changed.
pub fn popup<T: Copy + PartialEq>(ui: &mut Ui, id_salt: impl Hash + std::fmt::Debug, value: &mut T, groups: &[&[T]], title: impl Fn(T) -> &'static str, width: Option<f32>, enabled: bool) -> bool {
    let font = theme::regular(12.0);
    let galley = ui.painter().layout_no_wrap(title(*value).to_string(), font.clone(), color::label());
    let w = width.unwrap_or(galley.size().x + 34.0);
    let (rect, response) = ui.allocate_exact_size(vec2(w, metric::CONTROL_HEIGHT), if enabled { Sense::click() } else { Sense::hover() });
    let response = response.on_hover_cursor(egui::CursorIcon::Default);
    let dim = if enabled { 1.0 } else { 0.4 };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(11), color::control().gamma_multiply(dim));
    let text_clip = Rect::from_min_max(rect.min, pos2(rect.max.x - 22.0, rect.max.y));
    p.with_clip_rect(text_clip).galley(pos2(rect.min.x + 10.0, rect.center().y - galley.size().y / 2.0), galley, color::label().gamma_multiply(dim));
    icons::paint(p, Icon::Symbol("chevron.up.chevron.down"), pos2(rect.max.x - 12.0, rect.center().y), 10.0, color::label().gamma_multiply(dim));
    let mut changed = false;
    let _ = id_salt;
    egui::Popup::menu(&response).width(w.max(120.0)).show(|ui| {
        for (gi, group) in groups.iter().enumerate() {
            if gi > 0 {
                ui.separator();
            }
            for option in *group {
                let checked = *option == *value;
                let text = egui::RichText::new(title(*option)).font(theme::regular(13.0));
                if ui.add(egui::Button::new(text).selected(checked).frame_when_inactive(false)).clicked() {
                    *value = *option;
                    changed = true;
                }
            }
        }
    });
    changed
}

/// A labeled menu picker: "Label" then the pop-up, `width` wide in all.
pub fn labeled_popup<T: Copy + PartialEq>(ui: &mut Ui, label_text: &str, value: &mut T, groups: &[&[T]], title: impl Fn(T) -> &'static str, width: f32, enabled: bool) -> bool {
    let before = ui.cursor().min.x;
    label(ui, label_text);
    let used = ui.cursor().min.x - before;
    popup(ui, label_text, value, groups, title, Some((width - used).max(60.0)), enabled)
}

#[derive(Clone, Copy)]
pub struct SwatchStyle {
    pub radius: f32,
    /// A white inner ring inset by one point, this wide (0 for none).
    pub inner_white: f32,
    /// The outer border's black alpha.
    pub outer_black: f32,
}

/// Color swatch buttons: `RoundedRectangle` fill, optional white inner ring, black outer border.
pub fn swatch(ui: &mut Ui, fill: Color32, size: egui::Vec2, style: SwatchStyle) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    paint_swatch(ui.painter(), rect, fill, style);
    response
}

pub fn paint_swatch(p: &egui::Painter, rect: Rect, fill: Color32, style: SwatchStyle) {
    p.rect_filled(rect, style.radius, fill);
    if style.inner_white > 0.0 {
        p.rect_stroke(rect.shrink(1.0), (style.radius - 1.0).max(0.0), Stroke::new(style.inner_white, Color32::WHITE), StrokeKind::Inside);
    }
    p.rect_stroke(rect, style.radius, Stroke::new(1.0, black_alpha(style.outer_black)), StrokeKind::Inside);
}

/// `Divider().frame(height:)` inside a row.
pub fn vdivider(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color::separator());
}

/// A field followed by its unit, two points apart (`unitSuffix`).
pub fn unit(ui: &mut Ui, s: &str) {
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 2.0;
    text(ui, s, theme::regular(12.0), color::label());
    ui.spacing_mut().item_spacing.x = spacing;
}

/// Lays `content` out left to right in `rect`, centered vertically, `spacing` apart.
pub fn row<R>(ui: &mut Ui, rect: Rect, spacing: f32, content: impl FnOnce(&mut Ui) -> R) -> R {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(Align::Center)));
    child.spacing_mut().item_spacing = vec2(spacing, 0.0);
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    content(&mut child)
}
