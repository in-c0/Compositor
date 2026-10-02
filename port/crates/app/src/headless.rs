//! `compositor --render-ui parity/ui/states.toml --corpus parity/corpus --out <dir>`: renders
//! each UI state offscreen at 1x through the same egui code and wgpu renderer the window uses,
//! writes `<dir>/<id>.png`, `<dir>/menus.json` and `<dir>/ui-info.json` (which states rendered and
//! which are pending). Schema and conventions: parity/README.md, "UI states".

use crate::app::App;
use crate::gfx::Gfx;
use crate::theme::{self, metric};
use crate::{menus, tools, ui};
use anyhow::{Context, Result};
use eframe::egui::{self, Rect, pos2, vec2};
use eframe::egui_wgpu;
use engine::gpu::Gpu;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

#[derive(Deserialize)]
struct States {
    state: Vec<State>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct State {
    pub(crate) id: String,
    pub(crate) view: String,
    pub(crate) document: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) sheet: Option<String>,
    pub(crate) layer: Option<usize>,
}

/// The editor's size in the window state: `ContentView` at the window scene's default size, as the
/// Mac harness hosts it (`editorContentSize`). The Mac's toolbar lives in the title bar, outside it.
pub(crate) fn editor_size() -> egui::Vec2 {
    vec2(metric::WINDOW[0], metric::WINDOW[1])
}

/// What a state is flattened over where its views draw nothing. The Mac harness composites each
/// capture over its window's background: the editor's own gray fills the editor, and a standalone
/// panel or sheet shows `windowBackgroundColor`.
fn background(view: &str) -> egui::Color32 {
    match view {
        "layers-panel" | "sheet" => theme::color::WINDOW_BACKGROUND,
        _ => theme::color::EDITOR,
    }
}

pub(crate) struct Offscreen {
    pub(crate) gfx: Gfx,
    ctx: egui::Context,
    /// egui's clock, which must keep going forward from one render to the next.
    clock: std::cell::Cell<f64>,
}

impl Offscreen {
    pub(crate) fn new() -> Result<Self> {
        let gpu = Arc::new(Gpu::new()?);
        let options = egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: false, predictable_texture_filtering: false };
        let renderer = egui_wgpu::Renderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, options);
        let gfx = Gfx::new(gpu, Arc::new(egui::mutex::RwLock::new(renderer)));
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::install_style(&ctx);
        Ok(Self { gfx, ctx, clock: std::cell::Cell::new(0.0) })
    }

    /// Runs `draw` for a few passes (layout settles, fonts and textures load, animations finish),
    /// then paints the last one into a `size` image.
    fn render(&self, size: egui::Vec2, draw: impl FnMut(&mut egui::Ui, Rect)) -> Result<image::RgbaImage> {
        self.render_over(size, theme::color::EDITOR, &[], draw)
    }

    /// `render`, feeding each `(pass, event)` to its pass: tests click through the UI this way.
    #[cfg(test)]
    pub(crate) fn render_with_events(&self, size: egui::Vec2, events: &[(usize, egui::Event)], draw: impl FnMut(&mut egui::Ui, Rect)) -> Result<image::RgbaImage> {
        self.render_over(size, theme::color::EDITOR, events, draw)
    }

    /// `render` over `fill` instead of the editor's gray.
    fn render_over(&self, size: egui::Vec2, fill: egui::Color32, events: &[(usize, egui::Event)], mut draw: impl FnMut(&mut egui::Ui, Rect)) -> Result<image::RgbaImage> {
        let (w, h) = (size.x.round() as u32, size.y.round() as u32);
        let mut output = None;
        for pass in 0..4 {
            let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)), time: Some(self.clock.get() + pass as f64 * 0.25), predicted_dt: 0.25, ..Default::default() };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
            input.events = events.iter().filter(|(p, _)| *p == pass).map(|(_, e)| e.clone()).collect();
            let mut out = self.ctx.run_ui(input, |ui| {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), size);
                ui.painter().rect_filled(rect, 0.0, fill);
                draw(ui, rect);
            });
            let mut renderer = self.gfx.renderer.write();
            for (id, delta) in &out.textures_delta.set {
                for delta in delta {
                    renderer.update_texture(&self.gfx.gpu.device, &self.gfx.gpu.queue, *id, delta);
                }
            }
            // Freed textures aren't in this pass's shapes, so they can go before it's painted.
            for id in &out.textures_delta.free {
                renderer.free_texture(id);
            }
            out.textures_delta.clear();
            drop(renderer);
            output = Some(out);
        }
        self.clock.set(self.clock.get() + 1.0);
        let out = output.unwrap();
        let primitives = self.ctx.tessellate(out.shapes, 1.0);
        let device = &self.gfx.gpu.device;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut renderer = self.gfx.renderer.write();
            let extra = renderer.update_buffers(device, &self.gfx.gpu.queue, &mut encoder, &primitives, &screen);
            self.gfx.gpu.queue.submit(extra);
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            renderer.render(&mut pass.forget_lifetime(), &primitives, &screen);
        }
        let stride = (w * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui-readback"),
            size: (stride * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.gfx.gpu.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let data = slice.get_mapped_range()?;
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let start = (row * stride) as usize;
            pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        buffer.unmap();
        Ok(image::RgbaImage::from_raw(w, h, pixels).expect("image size"))
    }
}

pub(crate) fn app_for(off: &Offscreen, state: &State, corpus: &Path) -> Result<App> {
    let mut app = App::new(off.gfx.clone());
    app.headless = true;
    if let Some(case) = &state.document {
        let path = corpus.join(case).join("input.comp");
        let doc = crate::document::Doc::open(&path).with_context(|| format!("opening {}", path.display()))?;
        app.docs.push(doc);
        app.current = Some(0);
    }
    if let Some(tool) = &state.tool {
        app.tool = tools::from_state_name(tool, &mut app.settings).with_context(|| format!("unknown tool `{tool}`"))?;
    }
    if let (Some(index), Some(doc)) = (state.layer, app.doc_mut()) {
        doc.active = doc.project.manifest.layers.get(index).map(|l| l.id.clone());
    }
    Ok(app)
}

pub fn render_ui(states: &Path, corpus: &Path, out: &Path) -> Result<()> {
    let text = std::fs::read_to_string(states).with_context(|| format!("reading {}", states.display()))?;
    let states: States = toml::from_str(&text)?;
    let off = Offscreen::new()?;
    let mut info = Vec::new();
    // The same fields the Mac harness writes to its ui-info.json (UIStateResult), plus `reason`
    // for a pending state.
    for state in &states.state {
        let result = render_state(&off, state, corpus, out);
        let notes: Vec<String> = state.layer.and_then(|i| layer_note(corpus, state, i)).into_iter().collect();
        let entry = match result {
            Ok(Some((w, h))) => json!({ "id": state.id, "status": "ok", "output": format!("{}.png", state.id), "width": w, "height": h, "backingScale": 1, "notes": notes }),
            Ok(None) => json!({ "id": state.id, "status": "pending", "reason": format!("the port has no `{}` sheet yet", state.sheet.as_deref().unwrap_or("?")), "notes": notes }),
            Err(e) => json!({ "id": state.id, "status": "error", "error": format!("{e:#}"), "notes": notes }),
        };
        println!("{:<28} {}", state.id, entry["status"].as_str().unwrap_or(""));
        info.push(entry);
    }
    // Not a parity state: the Windows window as a whole (menu bar, toolbar with two project tabs,
    // editor), for looking at the port's own chrome.
    let mut whole = App::new(off.gfx.clone());
    whole.headless = true;
    for case in ["blend/stack", "masks/folder-mask"] {
        whole.docs.push(crate::document::Doc::open(&corpus.join(case).join("input.comp"))?);
    }
    whole.docs[1].modified = true;
    whole.rulers = true;
    whole.current = Some(0);
    let image = off.render(vec2(metric::WINDOW[0], metric::WINDOW[1]), |ui, _| ui::window(&mut whole, ui))?;
    for doc in &mut whole.docs {
        doc.release(&off.gfx);
    }
    save(&image, out, "port/window")?;
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("menus.json"), serde_json::to_string_pretty(&menus_json(&off, corpus)?)?)?;
    let editor = editor_size();
    let info = json!({
        "platform": if cfg!(target_os = "macos") { "macos-port" } else { "windows" },
        "adapter": off.gfx.gpu.adapter.get_info().name,
        "scale": 1,
        "editorSize": [editor.x, editor.y],
        "states": info,
    });
    std::fs::write(out.join("ui-info.json"), serde_json::to_string_pretty(&info)?)?;
    Ok(())
}

/// "Layer 0 is “Photo”.", the note the Mac harness writes when a state selects a layer.
fn layer_note(corpus: &Path, state: &State, index: usize) -> Option<String> {
    let project = comp_format::load(&corpus.join(state.document.as_deref()?).join("input.comp")).ok()?;
    Some(format!("Layer {index} is “{}”.", project.manifest.layers.get(index)?.name))
}

/// menus.json in the Mac harness's form (parity/harness/Sources/MenuDump.swift): `mainMenu`, the
/// menu bar at launch with no document open, and `layerContextMenus`, the Layers list's menu for
/// every row of the same documents the Mac right-clicks. Shortcuts are written the Mac's way,
/// Ctrl as ⌘ (see `menus::items_json`).
fn menus_json(off: &Offscreen, corpus: &Path) -> Result<serde_json::Value> {
    let welcome = App::new(off.gfx.clone());
    let mut rows = Vec::new();
    for case in menus::CONTEXT_MENU_DOCUMENTS {
        let mut doc = crate::document::Doc::open(&corpus.join(case).join("input.comp")).with_context(|| format!("opening {case}"))?;
        let layers = doc.project.manifest.layers.clone();
        for (row, r) in crate::document::rows(&layers, &Default::default()).iter().enumerate() {
            // A right-click selects the row, then the menu is built for it, as in the app.
            doc.active = Some(r.layer.id.clone());
            let items = menus::layer_context(&ui::layers::row_state_in(&doc, r));
            rows.push(json!({ "document": case, "row": row, "layer": r.layer.name, "items": menus::items_json(&items) }));
        }
    }
    Ok(json!({
        "mainMenu": menus::to_json(&menus::build(&welcome.menu_state())),
        "mainMenuCapture": {
            "platform": if cfg!(target_os = "macos") { "macos-port" } else { "windows" },
            "shortcuts": "Ctrl is written as command (⌘), Alt as option (⌥), Shift as shift (⇧); `windowsShortcut` is what the Windows menu shows",
        },
        "layerContextMenus": rows,
    }))
}

/// Opens the live sheet a state names as the app opens it (the menu command, the swatch, the
/// effects menu); false for a sheet only `ui::sheets` draws, or one the port doesn't have.
fn open_live_sheet(app: &mut App, sheet: &str) -> bool {
    use ui::dialogs::{self, Sheet};
    const FILTERS: [&str; 15] = [
        "Gaussian Blur", "Motion Blur", "Add Noise", "Vignette", "Bloom / Glow", "Dither", "Tonal Contrast", "Lens Correction",
        "Remove Background", "Curves", "Exposure", "Gradient Map", "Grain", "Black & White", "Color Balance",
    ];
    match sheet {
        "canvas-size" => dialogs::open_canvas_size(app),
        "image-size" => dialogs::open_image_size(app),
        "trim" => app.sheet = Some(Sheet::Trim(Default::default())),
        "export-jpeg" => dialogs::open_export_jpeg(app),
        "levels" => dialogs::open_filter(app, "Levels"),
        "hue-saturation" => dialogs::open_filter(app, "Hue/Saturation"),
        "color-range" => ui::selection::open_color_range(app),
        "color-picker" => dialogs::open_color_picker(app, false),
        "keyboard-shortcuts" => app.sheet = Some(Sheet::Shortcuts(String::new())),
        // Double-clicking the selected layer's first effect.
        "layer-effects" => {
            let first = app.doc().and_then(|d| d.active_layer()).and_then(|l| crate::document::effect_rows(l).first().map(|(kind, _)| *kind));
            if let Some(kind) = first {
                dialogs::open_effect(app, kind);
            }
        }
        _ => match sheet.strip_prefix("filter:").and_then(|k| FILTERS.iter().find(|f| **f == k)) {
            Some(kind) => dialogs::open_filter(app, kind),
            None => return false,
        },
    }
    app.sheet.is_some()
}

fn save(image: &image::RgbaImage, out: &Path, id: &str) -> Result<()> {
    let path = out.join(format!("{id}.png"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image.save(&path).with_context(|| format!("writing {}", path.display()))
}

fn render_state(off: &Offscreen, state: &State, corpus: &Path, out: &Path) -> Result<Option<(u32, u32)>> {
    let mut app = app_for(off, state, corpus)?;
    let editor = editor_size();
    let image = match state.view.as_str() {
        "window" => off.render(editor, |ui, rect| ui::editor(&mut app, ui, rect))?,
        // Cropped from the whole editor, as the Mac harness does: the rail, header and status bar
        // depend on the window around them (the canvas's zoom, the tool's hint).
        "tool-rail" | "status-bar" | "tool-header" => {
            let whole = off.render(editor, |ui, rect| ui::editor(&mut app, ui, rect))?;
            let top = metric::TOOL_HEADER as u32 + 1;
            let (x, y, w, h) = match state.view.as_str() {
                "tool-rail" => (0, top, metric::RAIL as u32, editor.y as u32 - top - metric::STATUS as u32 - 1),
                "status-bar" => (0, (editor.y - metric::STATUS) as u32, editor.x as u32, metric::STATUS as u32),
                _ => (0, 0, editor.x as u32, metric::TOOL_HEADER as u32),
            };
            image::imageops::crop_imm(&whole, x, y, w, h).to_image()
        }
        "layers-panel" => off.render_over(vec2(metric::LAYERS_DEFAULT, 600.0), background("layers-panel"), &[], |ui, rect| ui::layers::panel(&mut app, ui, rect))?,
        "sheet" if open_live_sheet(&mut app, state.sheet.as_deref().unwrap_or("")) => {
            // The live sheet, drawn alone at the origin: once to learn its size, then at that size.
            let capture = |ui: &mut egui::Ui, app: &mut App| {
                let ctx = ui.ctx().clone();
                ctx.data_mut(|d| d.insert_temp(egui::Id::new(ui::dialogs::CAPTURE), pos2(0.0, 0.0)));
                ui::dialogs::show(app, &ctx);
            };
            off.render_over(vec2(1400.0, 1600.0), background("sheet"), &[], |ui, _| capture(ui, &mut app))?;
            let size: egui::Vec2 = off.ctx.data(|d| d.get_temp(egui::Id::new(ui::dialogs::CAPTURED_SIZE))).context("the sheet didn't draw")?;
            let image = off.render_over(vec2(size.x.round(), size.y.ceil()), background("sheet"), &[], |ui, _| capture(ui, &mut app))?;
            off.ctx.data_mut(|d| {
                d.remove::<egui::Pos2>(egui::Id::new(ui::dialogs::CAPTURE));
                d.remove::<egui::Vec2>(egui::Id::new(ui::dialogs::CAPTURED_SIZE));
            });
            app.sheet = None;
            image
        }
        "sheet" => {
            let sheet = state.sheet.as_deref().unwrap_or("");
            let Some(width) = ui::sheets::width(sheet) else { return Ok(None) };
            // The sheet's natural height: lay it out once in a tall frame, then render at that size.
            let mut sheet_state = ui::sheets::SheetState::new(&app);
            let mut height = 0.0f32;
            off.render_over(vec2(width, 1600.0), background("sheet"), &[], |ui, rect| height = ui::sheets::draw(&mut app, &mut sheet_state, ui, rect, sheet))?;
            let mut sheet_state = ui::sheets::SheetState::new(&app);
            off.render_over(vec2(width, height.ceil()), background("sheet"), &[], |ui, rect| {
                ui::sheets::draw(&mut app, &mut sheet_state, ui, rect, sheet);
            })?
        }
        other => anyhow::bail!("unknown view `{other}`"),
    };
    for doc in &mut app.docs {
        doc.release(&off.gfx);
    }
    save(&image, out, &state.id)?;
    Ok(Some(image.dimensions()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::metric;

    fn corpus() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../parity/corpus")
    }

    fn click(at: egui::Pos2) -> Vec<(usize, egui::Event)> {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        vec![(1, egui::Event::PointerMoved(at)), (1, button(true)), (2, button(false))]
    }

    /// The flattened canvas, straight alpha, as Export PNG writes it.
    fn flatten(off: &Offscreen, app: &App) -> Vec<u8> {
        let project = &app.doc().unwrap().project;
        let canvas = engine::composite::Compositor::new(&off.gfx.gpu, project).render().unwrap();
        off.gfx.gpu.download(&engine::blend::unpremultiply(&off.gfx.gpu, &canvas)).unwrap()
    }

    #[test]
    fn a_layer_eye_hides_the_layer_and_the_canvas_redraws() {
        let off = Offscreen::new().unwrap();
        let state = State { id: "test".into(), view: "window".into(), document: Some("blend/stack".into()), tool: Some("move".into()), sheet: None, layer: None };
        let mut app = app_for(&off, &state, &corpus()).unwrap();
        let before = flatten(&off, &app);
        let editor = editor_size();
        // The second row (Hard Mix): the list starts under the tool header, the panel's header and
        // its blend and opacity controls; rows are 54 points apart and their eyes 8 points in.
        let panel_x = editor.x - metric::LAYERS_DEFAULT;
        let list_top = metric::TOOL_HEADER + 1.0 + ui::layers::HEADER + 1.0 + ui::layers::APPEARANCE + 1.0;
        let eye = pos2(panel_x + 18.0, list_top + 54.0 + 1.0 + 26.0);
        let target = app.doc().unwrap().project.manifest.layers[3].id.clone();
        assert_eq!(app.doc().unwrap().layer(&target).unwrap().name, "Hard Mix");
        off.render_with_events(editor, &click(eye), |ui, rect| ui::editor(&mut app, ui, rect)).unwrap();
        let doc = app.doc().unwrap();
        assert!(!doc.layer(&target).unwrap().is_visible, "the eye click hid the layer");
        assert!(doc.render_error.is_none() && doc.canvas.is_some(), "the canvas re-rendered");
        assert_ne!(before, flatten(&off, &app), "hiding a layer changes the composite");
        assert_eq!(doc.undo_title(), Some("Hide Layer"));
    }

    #[test]
    fn menus_have_the_mac_order() {
        let off = Offscreen::new().unwrap();
        let app = App::new(off.gfx.clone());
        let menus = menus::build(&app.menu_state());
        let titles: Vec<&str> = menus.iter().map(|m| m.title).collect();
        assert_eq!(titles, ["File", "Edit", "View", "Select", "Image", "Filter", "Layer", "Window", "Help"]);
        let file: Vec<String> = menus[0].items.iter().filter(|i| !i.system).map(|i| if i.separator { "-".into() } else { i.title.clone() }).collect();
        assert_eq!(file, ["New Canvas…", "Open Project…", "Open Recent", "Import Images…", "-", "Save", "Save As…", "-", "Export PNG…", "Export JPEG…", "-", "Close Project"]);
    }

    #[test]
    fn menus_json_takes_the_mac_harness_form() {
        let off = Offscreen::new().unwrap();
        let menus = menus_json(&off, &corpus()).unwrap();
        let bar: Vec<&str> = menus["mainMenu"].as_array().unwrap().iter().map(|m| m["title"].as_str().unwrap()).collect();
        assert_eq!(bar, ["File", "Edit", "View", "Select", "Image", "Filter", "Layer", "Window", "Help"]);
        let save_as = &menus["mainMenu"][0]["items"].as_array().unwrap().iter().find(|i| i["title"] == "Save As…").unwrap();
        assert_eq!((save_as["key"].as_str(), save_as["shortcut"].as_str()), (Some("s"), Some("⇧⌘S")));
        assert_eq!(save_as["modifiers"], serde_json::json!(["shift", "command"]));
        let rows = menus["layerContextMenus"].as_array().unwrap();
        // blend/stack's five rows, the top one first, as the Mac right-clicks them.
        assert_eq!(rows.iter().filter(|r| r["document"] == "blend/stack").count(), 5);
        assert_eq!((rows[0]["row"].as_u64(), rows[0]["layer"].as_str()), (Some(0), Some("Hidden")));
    }

    #[test]
    fn panel_edits_rerender_and_save_round_trips() {
        let off = Offscreen::new().unwrap();
        let state = State { id: "test".into(), view: "window".into(), document: Some("blend/stack".into()), tool: None, sheet: None, layer: None };
        let mut app = app_for(&off, &state, &corpus()).unwrap();
        let gfx = off.gfx.clone();
        let mut last = flatten(&off, &app);
        let doc = app.doc_mut().unwrap();
        let screen = doc.project.manifest.layers[1].id.clone();
        let edits: [&dyn Fn(&mut crate::document::Doc); 3] = [
            &|d| d.set_blend_mode(&screen, comp_format::BlendMode::Multiply),
            &|d| d.set_opacity(&screen, 0.5, false),
            &|d| assert!(d.move_layer(&screen, false)),
        ];
        for edit in edits {
            let doc = app.doc_mut().unwrap();
            edit(doc);
            doc.refresh(&gfx);
            assert!(doc.render_error.is_none());
            let now = flatten(&off, &app);
            assert_ne!(now, last);
            last = now;
        }
        let doc = app.doc_mut().unwrap();
        doc.undo();
        assert_eq!(doc.project.manifest.layers[1].id, screen, "undo puts the layer back");
        doc.redo();
        let dir = std::env::temp_dir().join(format!("compositor-app-test-{}", std::process::id()));
        let path = dir.join("saved.comp");
        doc.save(&path).unwrap();
        assert!(!doc.modified);
        let reloaded = comp_format::load(&path).unwrap();
        assert_eq!(reloaded.manifest, engine::session::normalize(&doc.project).manifest);
        std::fs::remove_dir_all(&dir).unwrap();
        doc.release(&gfx);
    }
}
