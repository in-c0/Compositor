//! The editor window: `ContentView.editorStack` and the toolbar above it.

pub mod headers;
pub mod layers;
pub mod sheets;

use crate::app::App;
use crate::document::{CRISP_ZOOM, PIXEL_GRID_ZOOM};
use crate::icons::{self, Icon};
use crate::menus::{self, Command};
use crate::theme::{self, color, gray, metric, white_alpha};
use crate::tools::{self, BrushMode, LassoKind, MarqueeKind, Tool, WandMode};
use crate::widgets::{self, ButtonStyle, SwatchStyle};
use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

/// A 1-point separator line across `rect`'s top (horizontal) or left (vertical) edge.
pub fn hline(ui: &Ui, x: std::ops::RangeInclusive<f32>, y: f32) {
    ui.painter().rect_filled(Rect::from_min_max(pos2(*x.start(), y), pos2(*x.end(), y + 1.0)), 0.0, color::separator());
}

pub fn vline(ui: &Ui, x: f32, y: std::ops::RangeInclusive<f32>) {
    ui.painter().rect_filled(Rect::from_min_max(pos2(x, *y.start()), pos2(x + 1.0, *y.end())), 0.0, color::separator());
}

/// The whole window for the live app: menu bar, toolbar, editor.
pub fn window(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    app.handle_keys(&ctx);
    let menus = menus::build(&app.menu_state());
    let mut chosen = None;
    egui::Panel::top("menu").frame(egui::Frame::new().fill(gray(0.17)).inner_margin(egui::Margin::symmetric(4, 2))).show(ui, |ui| {
        chosen = menus::bar(ui, &menus);
    });
    egui::Panel::top("toolbar").exact_size(metric::TOOLBAR).frame(egui::Frame::new().fill(gray(0.17))).show(ui, |ui| {
        let rect = ui.max_rect();
        if let Some(c) = toolbar(app, ui, rect) {
            chosen = Some(c);
        }
    });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(color::EDITOR)).show(ui, |ui| {
        let rect = ui.max_rect();
        hline(ui, rect.x_range().into(), rect.min.y);
        editor(app, ui, rect.with_min_y(rect.min.y + 1.0));
    });
    if let Some(c) = chosen {
        app.run(&ctx, c);
    }
    dialogs(app, &ctx);
    // Title shows the project, with an edited mark, as `representedURL`/`isDocumentEdited` do.
    let title = match app.doc() {
        Some(d) => format!("{}{} — Compositor", d.name, if d.modified { " •" } else { "" }),
        None => "Compositor".into(),
    };
    ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
}

fn dialogs(app: &mut App, ctx: &egui::Context) {
    if let Some(alert) = &app.alert {
        let (title, message) = (alert.title.clone(), alert.message.clone());
        let mut close = false;
        egui::Modal::new(egui::Id::new("alert")).show(ctx, |ui| {
            ui.set_width(320.0);
            widgets::text(ui, &title, theme::bold(13.0), color::label());
            ui.add_space(6.0);
            ui.label(egui::RichText::new(message).size(11.0).color(color::secondary()));
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, "OK", 13.0, ButtonStyle::Prominent, true).clicked() {
                    close = true;
                }
            });
        });
        if close {
            app.alert = None;
        }
    }
    if let Some(i) = app.confirm_close {
        let name = app.docs.get(i).map(|d| d.name.clone()).unwrap_or_default();
        let mut choice = None;
        egui::Modal::new(egui::Id::new("confirm-close")).show(ctx, |ui| {
            ui.set_width(340.0);
            widgets::text(ui, format!("Save changes to {name}?"), theme::bold(13.0), color::label());
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Your changes will be lost if you don’t save them.").size(11.0).color(color::secondary()));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, "Don’t Save", 13.0, ButtonStyle::Bordered, true).clicked() {
                    choice = Some(2);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button(ui, "Save", 13.0, ButtonStyle::Prominent, true).clicked() {
                        choice = Some(0);
                    }
                    if widgets::button(ui, "Cancel", 13.0, ButtonStyle::Bordered, true).clicked() {
                        choice = Some(1);
                    }
                });
            });
        });
        match choice {
            Some(0) => {
                app.confirm_close = None;
                app.current = Some(i);
                app.run(ctx, Command::Save);
                if !app.docs[i].modified {
                    app.close_tab(i, true);
                }
            }
            Some(1) => app.confirm_close = None,
            Some(2) => {
                app.confirm_close = None;
                app.close_tab(i, true);
            }
            _ => {}
        }
    }
}

/// The toolbar row: New canvas, the project tabs, then Fit, 100% and zoom.
fn toolbar(app: &mut App, ui: &mut Ui, rect: Rect) -> Option<Command> {
    let mut chosen = None;
    widgets::row(ui, rect.shrink2(vec2(10.0, 0.0)), 8.0, |ui| {
        if toolbar_icon(ui, "plus", true).on_hover_text("New canvas (Ctrl+N)").clicked() {
            chosen = Some(Command::NewCanvas);
        }
        ui.add_space(8.0);
        let tabs_width = (ui.available_width() - 150.0).max(200.0);
        let (strip, _) = ui.allocate_exact_size(vec2(tabs_width, 34.0), Sense::hover());
        tab_strip(app, ui, strip);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let doc = app.doc().is_some();
            if toolbar_icon(ui, "minus.magnifyingglass", doc).on_hover_text("Zoom out (Ctrl+−)").clicked() {
                chosen = Some(Command::ZoomOut);
            }
            ui.add_space(-8.0);
            if toolbar_icon(ui, "plus.magnifyingglass", doc).on_hover_text("Zoom in (Ctrl++)").clicked() {
                chosen = Some(Command::ZoomIn);
            }
            if toolbar_text(ui, "100%", doc).on_hover_text("Actual pixels (Ctrl+1)").clicked() {
                chosen = Some(Command::ActualPixels);
            }
            if toolbar_text(ui, "Fit", doc).on_hover_text("Fit canvas in window (Ctrl+0)").clicked() {
                chosen = Some(Command::FitCanvas);
            }
        });
    });
    chosen
}

fn toolbar_icon(ui: &mut Ui, symbol: &'static str, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    if response.hovered() && enabled {
        ui.painter().rect_filled(rect, CornerRadius::same(14), white_alpha(0.08));
    }
    let c = if enabled { color::label() } else { color::tertiary() };
    icons::paint(ui.painter(), Icon::Symbol(symbol), rect.center(), 14.0, c);
    response
}

fn toolbar_text(ui: &mut Ui, s: &str, enabled: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(s.into(), theme::regular(13.0), color::label());
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 28.0), if enabled { Sense::click() } else { Sense::hover() });
    if response.hovered() && enabled {
        ui.painter().rect_filled(rect, CornerRadius::same(14), white_alpha(0.08));
    }
    let c = if enabled { color::label() } else { color::tertiary() };
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, c);
    response
}

/// `ProjectTabStrip`: capsule pills, the oldest dropped behind an overflow pill when they don't fit.
fn tab_strip(app: &mut App, ui: &mut Ui, strip: Rect) {
    struct Tab {
        index: usize,
        label: String,
        width: f32,
        label_width: f32,
    }
    let tabs: Vec<Tab> = app
        .docs
        .iter()
        .enumerate()
        .map(|(index, d)| {
            let font = if Some(index) == app.current { theme::semibold(12.0) } else { theme::medium(12.0) };
            let text = ui.painter().layout_no_wrap(d.name.clone(), font, color::label()).size().x.ceil();
            let label_width = (text + if d.modified { 10.0 } else { 0.0 }).clamp(35.0, 155.0);
            Tab { index, label: d.name.clone(), width: label_width + 40.0, label_width }
        })
        .collect();
    // Drop tabs from the front until the rest and the overflow pill fit; never the selected one.
    let mut shown: Vec<&Tab> = tabs.iter().collect();
    let total = |v: &[&Tab]| v.iter().map(|t| t.width).sum::<f32>() + 6.0 * v.len().saturating_sub(1) as f32;
    let mut hidden = 0usize;
    let overflow_width = |n: usize| {
        let text = if n == 1 { "1 more tab".to_string() } else { format!("{n} more tabs") };
        ui.painter().layout_no_wrap(text, theme::medium(12.0), color::label()).size().x.ceil() + 11.0 + 4.0 + 10.0 + 11.0
    };
    while !shown.is_empty() && total(&shown) + if hidden > 0 { overflow_width(hidden) + 6.0 } else { 0.0 } > strip.width() {
        let Some(pos) = shown.iter().position(|t| Some(t.index) != app.current) else { break };
        shown.remove(pos);
        hidden += 1;
    }
    let y = strip.min.y + 3.0;
    let mut x = strip.min.x;
    let p = ui.painter().clone();
    if hidden > 0 {
        let w = overflow_width(hidden);
        let r = Rect::from_min_size(pos2(x, y), vec2(w, 28.0));
        p.rect_filled(r, CornerRadius::same(14), white_alpha(0.035));
        p.rect_stroke(r, CornerRadius::same(14), Stroke::new(1.0, white_alpha(0.08)), StrokeKind::Inside);
        let text = if hidden == 1 { "1 more tab".to_string() } else { format!("{hidden} more tabs") };
        let g = p.layout_no_wrap(text, theme::medium(12.0), color::label());
        let gw = g.size().x;
        p.galley(pos2(r.min.x + 11.0, r.center().y - g.size().y / 2.0), g, color::label());
        icons::paint(&p, Icon::Symbol("chevron.down"), pos2(r.min.x + 11.0 + gw + 4.0 + 5.0, r.center().y), 9.0, color::label());
        let response = ui.interact(r, ui.id().with("overflow"), Sense::click());
        let hidden_tabs: Vec<(usize, String)> = tabs.iter().filter(|t| !shown.iter().any(|s| s.index == t.index)).map(|t| (t.index, t.label.clone())).collect();
        egui::Popup::menu(&response).show(|ui| {
            for (i, name) in hidden_tabs {
                let modified = app.docs[i].modified;
                if ui.button(if modified { format!("• {name}") } else { name }).clicked() {
                    app.current = Some(i);
                }
            }
        });
        x += w + 6.0;
    }
    let mut close = None;
    for tab in shown {
        let active = Some(tab.index) == app.current;
        let r = Rect::from_min_size(pos2(x, y), vec2(tab.width, 28.0));
        let response = ui.interact(r, ui.id().with(("tab", tab.index)), Sense::click());
        p.rect_filled(r, CornerRadius::same(14), white_alpha(if active { 0.12 } else { 0.035 }));
        p.rect_stroke(r, CornerRadius::same(14), Stroke::new(1.0, white_alpha(if active { 0.22 } else { 0.08 })), StrokeKind::Inside);
        let mut lx = r.min.x + 11.0;
        let doc = &app.docs[tab.index];
        if doc.modified {
            p.circle_filled(pos2(lx + 2.5, r.center().y), 2.5, color::label());
            lx += 10.0;
        }
        let font = if active { theme::semibold(12.0) } else { theme::medium(12.0) };
        let g = p.layout_no_wrap(tab.label.clone(), font, color::label());
        let label_rect = Rect::from_min_size(pos2(lx, r.min.y), vec2(tab.label_width - if doc.modified { 10.0 } else { 0.0 }, 28.0));
        p.with_clip_rect(label_rect).galley(pos2(lx, r.center().y - g.size().y / 2.0), g, color::label());
        let close_rect = Rect::from_min_size(pos2(r.max.x - 5.0 - 16.0, r.min.y), vec2(16.0, 28.0));
        icons::paint(&p, Icon::Symbol("xmark"), close_rect.center(), 9.0, color::secondary());
        let close_response = ui.interact(close_rect, ui.id().with(("close", tab.index)), Sense::click());
        if close_response.clicked() {
            close = Some(tab.index);
        } else if response.clicked() {
            app.current = Some(tab.index);
        }
        x += tab.width + 6.0;
    }
    if let Some(i) = close {
        app.close_tab(i, false);
    }
}

/// `ContentView.editorStack` in `rect`: tool header, rail, canvas, Layers panel, status bar.
pub fn editor(app: &mut App, ui: &mut Ui, rect: Rect) {
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), metric::TOOL_HEADER));
    headers::tool_header(app, ui, header);
    hline(ui, rect.x_range().into(), header.max.y);
    let status = Rect::from_min_max(pos2(rect.min.x, rect.max.y - metric::STATUS), rect.max);
    status_bar(app, ui, status);
    hline(ui, rect.x_range().into(), status.min.y - 1.0);
    let middle = Rect::from_min_max(pos2(rect.min.x, header.max.y + 1.0), pos2(rect.max.x, status.min.y - 1.0));
    let rail = Rect::from_min_size(middle.min, vec2(metric::RAIL, middle.height()));
    tool_rail(app, ui, rail);
    vline(ui, rail.max.x, middle.y_range().into());
    let panel = Rect::from_min_max(pos2(middle.max.x - app.layers_width, middle.min.y), middle.max);
    let canvas = Rect::from_min_max(pos2(rail.max.x + 1.0, middle.min.y), pos2(panel.min.x - 1.0, middle.max.y));
    canvas_area(app, ui, canvas);
    resize_edge(app, ui, panel.min.x - 1.0, middle.y_range().into());
    layers::panel(app, ui, panel);
}

/// `PanelResizeEdge`: a divider with an 8-point drag strip that widens the panel to its right.
fn resize_edge(app: &mut App, ui: &mut Ui, x: f32, y: std::ops::RangeInclusive<f32>) {
    vline(ui, x, y.clone());
    let strip = Rect::from_min_max(pos2(x - 3.5, *y.start()), pos2(x + 4.5, *y.end()));
    let response = ui.interact(strip, ui.id().with("layers-resize"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeColumn).on_hover_text("Drag to resize the panel");
    if response.dragged() {
        app.layers_width = (app.layers_width - response.drag_delta().x).clamp(metric::LAYERS_MIN, metric::LAYERS_MAX);
    }
    if response.drag_stopped() {
        app.layers_width = app.layers_width.round();
    }
}

fn tool_symbol(app: &App, tool: Tool) -> Icon {
    let s = &app.settings;
    Icon::Symbol(match tool {
        Tool::Move => "arrow.up.left.and.arrow.down.right",
        Tool::Marquee => if s.marquee == MarqueeKind::Ellipse { "circle.dashed" } else { "rectangle.dashed" },
        Tool::Lasso if s.lasso == LassoKind::Polygonal => return Icon::PolygonalLassoTool,
        Tool::Lasso => "lasso",
        Tool::Wand if s.wand == WandMode::Object => return Icon::ObjectSelectionTool,
        Tool::Wand => "wand.and.stars",
        Tool::Crop => "crop",
        Tool::Brush => if s.brush_mode == BrushMode::Erase { "eraser" } else { "paintbrush.pointed" },
        Tool::SpotHealing => "bandage",
        Tool::CloneStamp => return Icon::CloneStampTool,
        Tool::Blur => "drop",
        Tool::Gradient => return Icon::GradientTool,
        Tool::Shape => "square.on.circle",
        Tool::Type => "textformat",
        Tool::Eyedropper => "eyedropper",
        Tool::Hand => "hand.draw",
        Tool::Zoom => "magnifyingglass",
        Tool::Idle => return Icon::Symbol(""),
    })
}

/// The tool rail: 36-point buttons 10 apart from 16 points down, then the color swatches.
pub fn tool_rail(app: &mut App, ui: &mut Ui, rect: Rect) {
    // Scrolls (without a scroller) when the window is too short for every tool.
    let content = 16.0 + Tool::RAIL.len() as f32 * 46.0 + 8.0 + 36.0 + 12.0;
    let overflow = (content - rect.height()).max(0.0);
    if ui.rect_contains_pointer(rect) {
        app.rail_scroll -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    app.rail_scroll = app.rail_scroll.clamp(0.0, overflow);
    let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let x = rect.center().x;
    let mut y = rect.min.y + 16.0 - app.rail_scroll;
    for tool in Tool::RAIL {
        let r = Rect::from_min_size(pos2(x - 18.0, y), vec2(36.0, 36.0));
        let response = ui.interact(r.intersect(rect), ui.id().with(("rail", tool as u8)), Sense::click()).on_hover_text(tool.label());
        if response.clicked() {
            app.select_tool(tool);
        }
        if app.tool == tool {
            p.rect_filled(r, CornerRadius::same(7), white_alpha(0.12));
            p.rect_stroke(r, CornerRadius::same(7), Stroke::new(1.0, white_alpha(0.14)), StrokeKind::Inside);
        }
        icons::paint(&p, tool_symbol(app, tool), r.center(), 17.0, color::label());
        y += 36.0 + 10.0;
    }
    palette(app, ui, &p, pos2(x - 18.0, y + 8.0));
}

/// `ColorPaletteControls`: background swatch under the foreground one, swap and reset buttons.
fn palette(app: &mut App, ui: &mut Ui, p: &egui::Painter, origin: Pos2) {
    let style = SwatchStyle { radius: 6.0, inner_white: 1.5, outer_black: 1.0 };
    let bg = Rect::from_min_size(origin + vec2(12.0, 12.0), vec2(24.0, 24.0));
    let fg = Rect::from_min_size(origin, vec2(24.0, 24.0));
    widgets::paint_swatch(p, bg, widgets::rgb(app.settings.background), style);
    widgets::paint_swatch(p, fg, widgets::rgb(app.settings.foreground), style);
    let swap = Rect::from_min_size(origin + vec2(27.0, -3.0), vec2(12.0, 12.0));
    icons::paint_rotated(p, "arrow.left.and.right", swap.center(), 9.0, color::secondary(), -std::f32::consts::FRAC_PI_4);
    if ui.interact(swap, ui.id().with("swap"), Sense::click()).on_hover_text("Swap foreground and background (X)").clicked() {
        std::mem::swap(&mut app.settings.foreground, &mut app.settings.background);
    }
    let reset = Rect::from_min_size(origin + vec2(-1.0, 27.0), vec2(12.0, 12.0));
    icons::paint(p, Icon::Symbol("arrow.counterclockwise"), reset.center(), 7.5, color::secondary());
    if ui.interact(reset, ui.id().with("reset"), Sense::click()).on_hover_text("Default colors (D)").clicked() {
        app.settings.foreground = [0.0; 3];
        app.settings.background = [1.0; 3];
    }
}

/// en-US percent with up to one decimal and grouping, as `.percent.precision(.fractionLength(0...1))`.
pub fn percent(zoom: f32) -> String {
    let v = (zoom as f64 * 1000.0).round() / 10.0;
    let whole = v.trunc() as i64;
    let digits = whole.abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let frac = ((v - v.trunc()) * 10.0).round() as i64;
    if frac == 0 { format!("{grouped}%") } else { format!("{grouped}.{frac}%") }
}

/// The status bar: zoom, size and color space on the left, the tool's hint on the right.
pub fn status_bar(app: &mut App, ui: &mut Ui, rect: Rect) {
    let font = theme::regular(11.0);
    let c = color::secondary();
    let inner = rect.shrink2(vec2(18.0, 0.0));
    let p = ui.painter().with_clip_rect(rect);
    let mut x = inner.min.x;
    let mut put = |s: &str, width: Option<f32>| {
        let g = p.layout_no_wrap(s.to_string(), font.clone(), c);
        let w = width.unwrap_or(g.size().x);
        p.galley(pos2(x, inner.center().y - g.size().y / 2.0), g, c);
        x += w + 16.0;
    };
    if let Some(d) = app.doc() {
        put(&percent(d.view.zoom), Some(62.0));
        put(&format!("{} × {} px", d.project.manifest.width, d.project.manifest.height), None);
        put("sRGB · Transparent", None);
    } else {
        put("Ready when you are", None);
    }
    let left_end = x;
    let hint = tools::hint(app.tool, &app.settings);
    let g = p.layout_no_wrap(hint, font.clone(), c);
    let hx = (inner.max.x - g.size().x).max(left_end);
    p.with_clip_rect(Rect::from_min_max(pos2(left_end, rect.min.y), pos2(inner.max.x, rect.max.y)))
        .galley(pos2(hx, inner.center().y - g.size().y / 2.0), g, c);
}

/// The canvas column: optional rulers around the canvas, or the welcome form with no document.
fn canvas_area(app: &mut App, ui: &mut Ui, rect: Rect) {
    let rulers = app.rulers && app.doc().is_some();
    let canvas = if rulers { Rect::from_min_max(rect.min + vec2(metric::RULER, metric::RULER), rect.max) } else { rect };
    canvas_view(app, ui, canvas);
    if rulers {
        ruler_corner(ui, Rect::from_min_size(rect.min, vec2(metric::RULER, metric::RULER)));
        let d = app.doc().unwrap();
        ruler(ui, d, Rect::from_min_max(pos2(canvas.min.x, rect.min.y), pos2(rect.max.x, canvas.min.y)), canvas, true);
        ruler(ui, d, Rect::from_min_max(pos2(rect.min.x, canvas.min.y), pos2(canvas.min.x, rect.max.y)), canvas, false);
    }
    if app.doc().is_none() {
        welcome_centered(app, ui, rect);
    }
}

fn ruler_corner(ui: &Ui, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, gray(0.2));
    p.line_segment([rect.min + vec2(5.0, 14.0), rect.min + vec2(14.0, 5.0)], Stroke::new(1.0, white_alpha(0.28)));
}

/// `CanvasRulerNSView`: ticks every tenth of the major step, labels at each major tick.
fn ruler(ui: &Ui, doc: &crate::document::Doc, rect: Rect, canvas: Rect, horizontal: bool) {
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0.0, gray(0.2));
    let view = &doc.view;
    let scale = view.points_per_pixel();
    let steps = [1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 2500.0, 5000.0, 10000.0, 20000.0, 25000.0];
    let step: f32 = steps.iter().copied().find(|s| s * scale >= 70.0).unwrap_or(25000.0);
    let minor = step / 10.0;
    let origin = view.document_rect(doc.size()).min + canvas.min.to_vec2();
    let hair = 1.0 / view.pixels_per_point;
    let (start, end) = if horizontal { ((rect.min.x - origin.x) / scale, (rect.max.x - origin.x) / scale) } else { ((rect.min.y - origin.y) / scale, (rect.max.y - origin.y) / scale) };
    let first = (start / minor).floor() * minor;
    let last = (end / minor).ceil() * minor;
    let tick = gray(0.62);
    let label = gray(0.78);
    let font = theme::regular(8.0);
    let mut value = first;
    let mut guard = 0;
    while value <= last + 0.001 && guard < 10_000 {
        guard += 1;
        let at = if horizontal { origin.x + value * scale } else { origin.y + value * scale };
        let rem = (value / step - (value / step).round()).abs() * step;
        let major = rem < 0.001;
        let half = step / 2.0;
        let mid = !major && ((value / half - (value / half).round()).abs() * half) < 0.001;
        let len = if major { 8.0 } else if mid { 5.0 } else { 3.0 };
        if horizontal {
            p.rect_filled(Rect::from_min_max(pos2(at - hair / 2.0, rect.max.y - len), pos2(at + hair / 2.0, rect.max.y)), 0.0, tick);
            if major {
                p.text(pos2(at + 2.0, rect.min.y), Align2::LEFT_TOP, format!("{}", value.round() as i64), font.clone(), label);
            }
        } else {
            p.rect_filled(Rect::from_min_max(pos2(rect.max.x - len, at - hair / 2.0), pos2(rect.max.x, at + hair / 2.0)), 0.0, tick);
            if major {
                let g = p.layout_no_wrap(format!("{}", value.round() as i64), font.clone(), label);
                let w = g.size().x;
                // Rotated to read downward, ending at the tick.
                let shape = egui::epaint::TextShape::new(pos2(rect.min.x + 1.0, at + 2.0 + w), g, label).with_angle(-std::f32::consts::FRAC_PI_2);
                p.add(shape);
            }
        }
        value += minor;
    }
    let edge = gray(0.08);
    if horizontal {
        p.rect_filled(Rect::from_min_max(pos2(rect.min.x, rect.max.y - hair), rect.max), 0.0, edge);
    } else {
        p.rect_filled(Rect::from_min_max(pos2(rect.max.x - hair, rect.min.y), rect.max), 0.0, edge);
    }
}

/// `EditorCanvas`: backdrop, drop shadow, checkerboard, the composite, pixel grid and edge.
fn canvas_view(app: &mut App, ui: &mut Ui, rect: Rect) {
    let ppp = ui.ctx().pixels_per_point();
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, 0.0, color::CANVAS);
    let tool = app.tool;
    let pixel_grid = app.pixel_grid;
    let gfx = app.gfx.clone();
    let Some(doc) = app.doc_mut() else { return };
    let size = doc.size();
    doc.view.resize(rect.size(), ppp, size);
    doc.refresh(&gfx);

    // Navigation: scroll pans, Ctrl-scroll or pinch zooms, Hand and Space drag, Zoom clicks.
    let response = ui.interact(rect, ui.id().with("canvas"), Sense::click_and_drag());
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    if response.hovered() {
        let (scroll, zoom, pointer) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        if zoom != 1.0 {
            if let Some(pos) = pointer {
                let z = doc.view.zoom * zoom;
                doc.view.set_zoom(z, pos - rect.min, size);
            }
        } else if scroll != egui::Vec2::ZERO {
            doc.view.pan += scroll;
            doc.view.follows_fit = false;
        }
    }
    let panning = tool == Tool::Hand || space || ui.input(|i| i.pointer.middle_down());
    if response.dragged() && panning {
        doc.view.pan += response.drag_delta();
        doc.view.follows_fit = false;
    }
    if tool == Tool::Zoom && response.clicked() && !space {
        if let Some(pos) = response.interact_pointer_pos() {
            let out = ui.input(|i| i.modifiers.alt);
            let target = doc.view.keyboard_zoom_target(if out { -1 } else { 1 });
            doc.view.set_zoom(target, pos - rect.min, size);
        }
    }
    if panning || tool == Tool::Hand {
        ui.ctx().set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
    }

    let doc_rect = doc.view.document_rect(size).translate(rect.min.to_vec2());
    let shadow = egui::epaint::Shadow { offset: [0, 3], blur: 14, spread: 0, color: theme::black_alpha(0.35) };
    p.add(shadow.as_shape(doc_rect, CornerRadius::ZERO));
    p.rect_filled(doc_rect, 0.0, gray(0.30));
    let visible = doc_rect.intersect(rect);
    if visible.is_positive() {
        let tile = 10.0;
        let cp = p.with_clip_rect(visible);
        let (x0, x1) = (((visible.min.x - doc_rect.min.x) / tile).floor() as i64, ((visible.max.x - doc_rect.min.x) / tile).ceil() as i64);
        let (y0, y1) = (((visible.min.y - doc_rect.min.y) / tile).floor() as i64, ((visible.max.y - doc_rect.min.y) / tile).ceil() as i64);
        let mut mesh = egui::Mesh::default();
        for row in y0..y1 {
            for col in x0..x1 {
                if (row + col) % 2 == 0 {
                    let r = Rect::from_min_size(doc_rect.min + vec2(col as f32 * tile, row as f32 * tile), vec2(tile, tile));
                    mesh.add_colored_rect(r, gray(0.35));
                }
            }
        }
        cp.add(egui::Shape::mesh(mesh));
        if let Some(canvas) = &doc.canvas {
            let texture = if doc.view.zoom >= CRISP_ZOOM { canvas.nearest } else { canvas.linear };
            cp.image(texture, doc_rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        if pixel_grid && doc.view.zoom >= PIXEL_GRID_ZOOM {
            let scale = doc.view.points_per_pixel();
            let hair = 1.0 / ppp;
            let grid = Color32::from_rgba_unmultiplied(140, 140, 140, 115);
            let mut mesh = egui::Mesh::default();
            let first_x = ((visible.min.x - doc_rect.min.x) / scale).ceil() as i64;
            let last_x = ((visible.max.x - doc_rect.min.x) / scale).floor() as i64;
            for column in first_x..=last_x {
                let x = doc_rect.min.x + column as f32 * scale;
                mesh.add_colored_rect(Rect::from_min_max(pos2(x - hair / 2.0, visible.min.y), pos2(x + hair / 2.0, visible.max.y)), grid);
            }
            let first_y = ((visible.min.y - doc_rect.min.y) / scale).ceil() as i64;
            let last_y = ((visible.max.y - doc_rect.min.y) / scale).floor() as i64;
            for row in first_y..=last_y {
                let y = doc_rect.min.y + row as f32 * scale;
                mesh.add_colored_rect(Rect::from_min_max(pos2(visible.min.x, y - hair / 2.0), pos2(visible.max.x, y + hair / 2.0)), grid);
            }
            cp.add(egui::Shape::mesh(mesh));
        }
    }
    p.rect_stroke(doc_rect, 0.0, Stroke::new(1.0 / ppp, white_alpha(0.13)), StrokeKind::Middle);
    if let Some(error) = &doc.render_error {
        let g = p.layout_no_wrap(error.clone(), theme::medium(12.0), Color32::WHITE);
        let badge = Rect::from_center_size(pos2(rect.center().x, rect.max.y - 14.0 - 13.0), vec2(g.size().x + 24.0, 26.0));
        p.rect_filled(badge, CornerRadius::same(13), theme::black_alpha(0.75));
        p.rect_stroke(badge, CornerRadius::same(13), Stroke::new(1.0, white_alpha(0.14)), StrokeKind::Inside);
        p.galley(badge.center() - g.size() / 2.0, g, Color32::WHITE);
    }
}

fn welcome_centered(app: &mut App, ui: &mut Ui, rect: Rect) {
    let width = rect.width().min(500.0);
    let height = welcome_height();
    let r = Rect::from_center_size(rect.center(), vec2(width, height));
    welcome(app, ui, r);
}

pub fn welcome_height() -> f32 {
    // padding 28 · title row 28 · 24 · fields (15 + 8 + 40) · 24 · note 15 · 24 · buttons 24 · padding 28
    28.0 + 28.0 + 24.0 + 63.0 + 24.0 + 15.0 + 24.0 + 24.0 + 28.0
}

/// `NewCanvasSheet`, shown on the empty canvas.
pub fn welcome(app: &mut App, ui: &mut Ui, rect: Rect) {
    let inner = rect.shrink(28.0);
    let p = ui.painter().clone();
    let mut y = inner.min.y;
    // Title and the preset menu.
    let title = p.layout_no_wrap("New canvas".into(), theme::semibold(17.0), color::label());
    p.galley(pos2(inner.min.x, y + 14.0 - title.size().y / 2.0), title, color::label());
    let dots = Rect::from_min_size(pos2(inner.max.x - 28.0, y), vec2(28.0, 28.0));
    for i in 0..3 {
        p.circle_filled(pos2(dots.max.x - 1.25, dots.center().y + (i as f32 - 1.0) * 5.0), 1.25, color::label());
    }
    let presets = ui.interact(dots, ui.id().with("presets"), Sense::click()).on_hover_text("Preset sizes for screens and common formats");
    egui::Popup::menu(&presets).show(|ui| {
        let groups: [&[(&str, i64, i64)]; 4] = [
            &[("4K", 3840, 2160), ("1440p", 2560, 1440), ("1080p", 1920, 1080)],
            &[("iPhone 18 Pro", 1206, 2622), ("iPhone 18 Pro Max", 1320, 2868), ("MacBook Pro 14\"", 3024, 1964), ("MacBook Pro 16\"", 3456, 2234), ("Studio Display", 5120, 2880)],
            &[("Instagram Square", 1080, 1080), ("Instagram Portrait", 1080, 1350), ("Instagram Story", 1080, 1920), ("YouTube Thumb", 1080, 608)],
            &[],
        ];
        let current = (app.form.width.trim().to_string(), app.form.height.trim().to_string());
        let custom = !groups.iter().flat_map(|g| g.iter()).any(|(_, w, h)| (w.to_string(), h.to_string()) == current);
        ui.add(egui::Button::new("Custom").selected(custom).frame_when_inactive(false));
        for group in groups.iter().filter(|g| !g.is_empty()) {
            ui.separator();
            for (name, w, h) in *group {
                let on = (w.to_string(), h.to_string()) == current;
                if ui.add(egui::Button::new(format!("{name} {w}×{h}")).selected(on).frame_when_inactive(false)).clicked() {
                    app.form.width = w.to_string();
                    app.form.height = h.to_string();
                }
            }
        }
    });
    y += 28.0 + 24.0;
    // Width × Height.
    let times = 16.0 + 12.0 + 16.0;
    let block_w = (inner.width() - times) / 2.0;
    let field = |ui: &mut Ui, x: f32, label: &str, value: &mut String, id: &str| {
        let g = p.layout_no_wrap(label.into(), theme::medium(12.0), color::label());
        p.galley(pos2(x, y), g, color::label());
        let bx = Rect::from_min_size(pos2(x, y + 15.0 + 8.0), vec2(block_w, 40.0));
        p.rect_filled(bx, 7.0, white_alpha(0.05));
        let px = p.layout_no_wrap("px".into(), theme::regular(13.0), color::secondary());
        let pxw = px.size().x;
        p.galley(pos2(bx.max.x - 12.0 - pxw, bx.center().y - px.size().y / 2.0), px, color::secondary());
        let edit = egui::TextEdit::singleline(value).id(ui.make_persistent_id(id)).frame(egui::Frame::NONE).font(theme::regular(13.0)).margin(egui::Margin::ZERO).vertical_align(egui::Align::Center);
        ui.put(Rect::from_min_max(pos2(bx.min.x + 12.0, bx.min.y + 8.0), pos2(bx.max.x - 12.0 - pxw - 8.0, bx.max.y - 8.0)), edit)
    };
    let mut w = std::mem::take(&mut app.form.width);
    let mut h = std::mem::take(&mut app.form.height);
    let enter_w = field(ui, inner.min.x, "Width", &mut w, "form-width");
    let times_center = pos2(inner.min.x + block_w + 16.0 + 6.0, y + 23.0 + 20.0);
    icons::paint(&p, Icon::Symbol("multiply"), times_center, 12.0, color::tertiary());
    let enter_h = field(ui, inner.min.x + block_w + times, "Height", &mut h, "form-height");
    app.form.width = w;
    app.form.height = h;
    y += 63.0 + 24.0;
    let valid = app.form_valid();
    let (note, note_color) = if valid { ("Transparent canvas · sRGB", color::secondary()) } else { ("Enter whole numbers from 1 to 30,000 pixels.", color::ORANGE) };
    let g = p.layout_no_wrap(note.into(), theme::regular(12.0), note_color);
    p.galley(pos2(inner.min.x, y), g, note_color);
    y += 15.0 + 24.0;
    let buttons = Rect::from_min_size(pos2(inner.min.x, y), vec2(inner.width(), 24.0));
    let submitted = (enter_w.lost_focus() || enter_h.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let mut open = false;
    let mut create = submitted && valid;
    widgets::row(ui, buttons, 10.0, |ui| {
        open = widgets::button(ui, "Open project", 13.0, ButtonStyle::Bordered, true).clicked();
        widgets::button(ui, "Import image", 13.0, ButtonStyle::Bordered, false);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, "Create canvas", 13.0, ButtonStyle::Prominent, valid).clicked() {
                create = true;
            }
        });
    });
    if open && !app.headless {
        if let Some(path) = crate::app::pick_project() {
            app.open_path(&path);
        }
    }
    if create {
        app.create_canvas();
    }
}
