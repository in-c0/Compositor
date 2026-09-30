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
struct State {
    id: String,
    view: String,
    document: Option<String>,
    tool: Option<String>,
    sheet: Option<String>,
    layer: Option<usize>,
}

/// The editor's size in the window state: the default window less the Mac's compact toolbar,
/// which lives in the title bar and isn't part of `ContentView`.
fn editor_size() -> egui::Vec2 {
    vec2(metric::WINDOW[0], metric::WINDOW[1] - metric::MAC_TOOLBAR)
}

struct Offscreen {
    gfx: Gfx,
    ctx: egui::Context,
}

impl Offscreen {
    fn new() -> Result<Self> {
        let gpu = Arc::new(Gpu::new()?);
        let options = egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: false, predictable_texture_filtering: false };
        let renderer = egui_wgpu::Renderer::new(&gpu.device, wgpu::TextureFormat::Rgba8Unorm, options);
        let gfx = Gfx::new(gpu, Arc::new(egui::mutex::RwLock::new(renderer)));
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::install_style(&ctx);
        Ok(Self { gfx, ctx })
    }

    /// Runs `draw` for a few passes (layout settles, fonts and textures load, animations finish),
    /// then paints the last one into a `size` image.
    fn render(&self, size: egui::Vec2, draw: impl FnMut(&mut egui::Ui, Rect)) -> Result<image::RgbaImage> {
        self.render_with_events(size, &[], draw)
    }

    /// `render`, feeding each `(pass, event)` to its pass: tests click through the UI this way.
    fn render_with_events(&self, size: egui::Vec2, events: &[(usize, egui::Event)], mut draw: impl FnMut(&mut egui::Ui, Rect)) -> Result<image::RgbaImage> {
        let (w, h) = (size.x.round() as u32, size.y.round() as u32);
        let mut output = None;
        for pass in 0..4 {
            let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)), time: Some(pass as f64 * 0.25), predicted_dt: 0.25, ..Default::default() };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
            input.events = events.iter().filter(|(p, _)| *p == pass).map(|(_, e)| e.clone()).collect();
            let mut out = self.ctx.run_ui(input, |ui| {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), size);
                ui.painter().rect_filled(rect, 0.0, theme::color::EDITOR);
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

fn app_for(off: &Offscreen, state: &State, corpus: &Path) -> Result<App> {
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
    for state in &states.state {
        let result = render_state(&off, state, corpus, out);
        let entry = match result {
            Ok(Some(size)) => json!({ "id": state.id, "status": "ok", "size": [size.0, size.1] }),
            Ok(None) => json!({ "id": state.id, "status": "pending", "reason": format!("the port has no `{}` sheet yet", state.sheet.as_deref().unwrap_or("?")) }),
            Err(e) => json!({ "id": state.id, "status": "error", "reason": format!("{e:#}") }),
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
    // Menus: at launch (no document) and with a document open and the Move tool.
    let welcome = App::new(off.gfx.clone());
    let mut document = App::new(off.gfx.clone());
    document.docs.push(crate::document::Doc::open(&corpus.join("blend/stack/input.comp"))?);
    document.current = Some(0);
    let menus = json!({
        "schema": "compositor-menus/1",
        "platform": if cfg!(target_os = "macos") { "macos-port" } else { "windows" },
        "states": [
            { "id": "welcome", "document": null, "menus": menus::to_json(&menus::build(&welcome.menu_state())) },
            {
                "id": "document", "document": "blend/stack", "tool": "move",
                "menus": menus::to_json(&menus::build(&document.menu_state())),
                // Right-clicking the top row.
                "layerContextMenu": menus::items_json(&menus::layer_context(&ui::layers::row_state(&crate::document::rows(&document.docs[0].project.manifest.layers, &Default::default())[0]))),
            },
        ],
    });
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("menus.json"), serde_json::to_string_pretty(&menus)?)?;
    std::fs::write(out.join("ui-info.json"), serde_json::to_string_pretty(&json!({ "scale": 1, "states": info }))?)?;
    Ok(())
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
        // Cropped from the whole editor, as the Mac harness does: the rail and status bar depend on
        // the window around them (the canvas's zoom, the tool's hint).
        "tool-rail" | "status-bar" => {
            let whole = off.render(editor, |ui, rect| ui::editor(&mut app, ui, rect))?;
            let (x, y, w, h) = if state.view == "tool-rail" {
                let top = metric::TOOL_HEADER as u32 + 1;
                (0, top, metric::RAIL as u32, editor.y as u32 - top - metric::STATUS as u32 - 1)
            } else {
                (0, (editor.y - metric::STATUS) as u32, editor.x as u32, metric::STATUS as u32)
            };
            image::imageops::crop_imm(&whole, x, y, w, h).to_image()
        }
        "tool-header" => off.render(vec2(editor.x, metric::TOOL_HEADER), |ui, rect| ui::headers::tool_header(&mut app, ui, rect))?,
        "layers-panel" => off.render(vec2(metric::LAYERS_DEFAULT, 600.0), |ui, rect| ui::layers::panel(&mut app, ui, rect))?,
        "sheet" => {
            let sheet = state.sheet.as_deref().unwrap_or("");
            let Some(width) = ui::sheets::width(sheet) else { return Ok(None) };
            // The sheet's natural height: lay it out once in a tall frame, then render at that size.
            let mut sheet_state = ui::sheets::SheetState::new(&app);
            let mut height = 0.0f32;
            off.render(vec2(width, 1600.0), |ui, rect| height = ui::sheets::draw(&mut app, &mut sheet_state, ui, rect, sheet))?;
            let mut sheet_state = ui::sheets::SheetState::new(&app);
            off.render(vec2(width, height.ceil()), |ui, rect| {
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
        assert_eq!(titles, ["File", "Edit", "Select", "Image", "Filter", "Layer", "View", "Window", "Help"]);
        let file: Vec<String> = menus[0].items.iter().filter(|i| !i.system).map(|i| if i.separator { "-".into() } else { i.title.clone() }).collect();
        assert_eq!(file, ["New Canvas…", "Open Project…", "Open Recent", "Import Images…", "Save", "Save As…", "-", "Export PNG…", "Export JPEG…", "-", "Close Project", "-"]);
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
