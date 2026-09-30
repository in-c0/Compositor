//! Compositor for Windows (and the Mac, through Metal): the editor shell around the engine.
//!
//!     compositor [project.comp]
//!     compositor --render-ui parity/ui/states.toml --corpus parity/corpus --out <dir>

mod app;
mod document;
mod geometry;
mod gfx;
mod headless;
mod icons;
mod layer_ops;
mod menus;
#[cfg(test)]
mod tests;
mod theme;
mod tools;
mod ui;
mod widgets;

use clap::Parser;
use eframe::egui;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "compositor", about = "Compositor image editor")]
struct Args {
    /// A .comp project to open.
    project: Option<PathBuf>,
    /// Render the UI states in this TOML file to PNGs instead of opening a window.
    #[arg(long, value_name = "STATES")]
    render_ui: Option<PathBuf>,
    /// The parity corpus the states' documents come from.
    #[arg(long, default_value = "parity/corpus")]
    corpus: PathBuf,
    /// Where --render-ui writes its PNGs and JSON.
    #[arg(long)]
    out: Option<PathBuf>,
}

struct Shell(app::App);

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::window(&mut self.0, ui);
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if let Some(states) = &args.render_ui {
        let out = args.out.clone().unwrap_or_else(|| PathBuf::from("ui-renders"));
        return headless::render_ui(states, &args.corpus, &out);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Compositor")
            .with_inner_size(theme::metric::WINDOW)
            .with_min_inner_size(theme::metric::WINDOW_MIN),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: gfx::wgpu_options(),
        ..Default::default()
    };
    let project = args.project.clone();
    eframe::run_native(
        "Compositor",
        options,
        Box::new(move |cc| {
            theme::install_fonts(&cc.egui_ctx);
            theme::install_style(&cc.egui_ctx);
            let state = cc.wgpu_render_state.as_ref().ok_or("the wgpu renderer didn't start")?;
            let mut app = app::App::new(gfx::Gfx::from_render_state(state));
            if let Some(path) = project {
                app.open_path(&path);
            }
            Ok(Box::new(Shell(app)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
