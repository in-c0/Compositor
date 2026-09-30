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
        let gfx = Gfx { gpu, renderer: Arc::new(egui::mutex::RwLock::new(renderer)) };
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::install_style(&ctx);
        Ok(Self { gfx, ctx })
    }

    /// Runs `draw` for a few passes (layout settles, fonts and textures load, animations finish),
    /// then paints the last one into a `size` image.
    fn render(&self, size: egui::Vec2, mut draw: impl FnMut(&mut egui::Ui, Rect)) -> Result<image::RgbaImage> {
        let (w, h) = (size.x.round() as u32, size.y.round() as u32);
        let mut output = None;
        for pass in 0..4 {
            let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)), time: Some(pass as f64), predicted_dt: 1.0, ..Default::default() };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
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
            { "id": "document", "document": "blend/stack", "tool": "move", "menus": menus::to_json(&menus::build(&document.menu_state())) },
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
        "sheet" => match state.sheet.as_deref() {
            Some("new-canvas") => off.render(vec2(500.0, ui::welcome_height()), |ui, rect| ui::welcome(&mut app, ui, rect))?,
            _ => return Ok(None),
        },
        other => anyhow::bail!("unknown view `{other}`"),
    };
    for doc in &mut app.docs {
        doc.release(&off.gfx);
    }
    save(&image, out, &state.id)?;
    Ok(Some(image.dimensions()))
}
