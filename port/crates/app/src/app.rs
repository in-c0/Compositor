//! The editor's state and the actions menus, keys and panels run on it.

use crate::document::Doc;
use crate::gfx::Gfx;
use crate::menus::{self, Command, MenuState};
use crate::theme::metric;
use crate::tools::{Tool, ToolSettings};
use eframe::egui;
use std::path::{Path, PathBuf};

pub struct NewCanvasForm {
    pub width: String,
    pub height: String,
}

pub struct Alert {
    pub title: String,
    pub message: String,
}

pub struct App {
    pub gfx: Gfx,
    pub docs: Vec<Doc>,
    pub current: Option<usize>,
    pub tool: Tool,
    pub settings: ToolSettings,
    pub layers_width: f32,
    /// How far the tool rail is scrolled when the window is too short for every tool.
    pub rail_scroll: f32,
    pub pixel_grid: bool,
    pub rulers: bool,
    pub form: NewCanvasForm,
    pub recent: Vec<PathBuf>,
    pub alert: Option<Alert>,
    /// A tab waiting on "Save changes?".
    pub confirm_close: Option<usize>,
    /// Whether the Layers list has keyboard focus (accent selection) or not (gray).
    pub layers_focused: bool,
    /// A layer row being dragged to a new place.
    pub dragging_layer: Option<String>,
    untitled: usize,
    /// When true the app is rendering for `--render-ui`: no dialogs, no window commands.
    pub headless: bool,
}

impl App {
    pub fn new(gfx: Gfx) -> Self {
        Self {
            gfx,
            docs: Vec::new(),
            current: None,
            tool: Tool::Move,
            settings: ToolSettings::default(),
            layers_width: metric::LAYERS_DEFAULT,
            rail_scroll: 0.0,
            pixel_grid: true,
            rulers: false,
            form: NewCanvasForm { width: "1920".into(), height: "1080".into() },
            recent: Vec::new(),
            alert: None,
            confirm_close: None,
            layers_focused: false,
            dragging_layer: None,
            untitled: 0,
            headless: false,
        }
    }

    pub fn doc(&self) -> Option<&Doc> {
        self.current.and_then(|i| self.docs.get(i))
    }

    pub fn doc_mut(&mut self) -> Option<&mut Doc> {
        self.current.and_then(|i| self.docs.get_mut(i))
    }

    pub fn open_path(&mut self, path: &Path) {
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(path)) {
            self.current = Some(i);
            return;
        }
        match Doc::open(path) {
            Ok(doc) => {
                self.recent.retain(|p| p != path);
                self.recent.insert(0, path.to_path_buf());
                self.recent.truncate(10);
                self.add_doc(doc);
            }
            Err(e) => self.alert("Couldn’t open the project", format!("{e:#}")),
        }
    }

    fn add_doc(&mut self, doc: Doc) {
        // An untouched empty tab is reused, as the Mac's workspace does.
        self.docs.push(doc);
        self.current = Some(self.docs.len() - 1);
    }

    fn next_untitled(&mut self) -> String {
        self.untitled += 1;
        if self.untitled == 1 { "Untitled".into() } else { format!("Untitled {}", self.untitled) }
    }

    pub fn create_canvas(&mut self) {
        let parse = |s: &str| s.trim().parse::<i64>().ok().filter(|v| (1..=30_000).contains(v));
        if let (Some(w), Some(h)) = (parse(&self.form.width), parse(&self.form.height)) {
            let name = self.next_untitled();
            self.add_doc(Doc::blank(name, w, h));
        }
    }

    pub fn form_valid(&self) -> bool {
        let parse = |s: &str| s.trim().parse::<i64>().ok().filter(|v| (1..=30_000).contains(v));
        parse(&self.form.width).is_some() && parse(&self.form.height).is_some()
    }

    pub fn alert(&mut self, title: &str, message: String) {
        self.alert = Some(Alert { title: title.into(), message });
    }

    pub fn select_tool(&mut self, tool: Tool) {
        use crate::tools::*;
        // Pressing a tool's key again cycles its mode, as on the Mac.
        if tool == self.tool {
            match tool {
                Tool::Marquee => {
                    self.settings.marquee = if self.settings.marquee == MarqueeKind::Rectangle { MarqueeKind::Ellipse } else { MarqueeKind::Rectangle }
                }
                Tool::Lasso => {
                    self.settings.lasso = if self.settings.lasso == LassoKind::Freehand { LassoKind::Polygonal } else { LassoKind::Freehand }
                }
                _ => {}
            }
        }
        self.tool = tool;
    }

    pub fn menu_state(&self) -> MenuState {
        let doc = self.doc();
        let active = doc.and_then(|d| d.active_layer());
        MenuState {
            has_document: doc.is_some(),
            undo: doc.and_then(|d| d.undo_title().map(String::from)),
            redo: doc.and_then(|d| d.redo_title().map(String::from)),
            move_tool: self.tool == Tool::Move,
            show_controls: self.settings.show_controls,
            pixel_grid: self.pixel_grid,
            rulers: self.rulers,
            active_layer_visible: active.map(|l| l.is_visible),
            active_layer_clipped: active.is_some_and(|l| l.mask_source_id.is_some()),
            can_move_up: doc.zip(active).is_some_and(|(d, l)| d.can_move(&l.id, true)),
            can_move_down: doc.zip(active).is_some_and(|(d, l)| d.can_move(&l.id, false)),
            recent: self.recent.iter().map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()).collect(),
            tabs: self.docs.iter().enumerate().map(|(i, d)| (d.name.clone(), Some(i) == self.current)).collect(),
        }
    }

    pub fn run(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::NewCanvas => {
                // The welcome form is the New Canvas sheet: with no document selected it shows.
                self.current = None;
            }
            Command::Open => {
                if let Some(path) = pick_project() {
                    self.open_path(&path);
                }
            }
            Command::OpenRecent(i) => {
                if let Some(path) = self.recent.get(i).cloned() {
                    self.open_path(&path);
                }
            }
            Command::ClearRecent => self.recent.clear(),
            Command::Save => self.save(false),
            Command::SaveAs => self.save(true),
            Command::ExportPng => self.export_png(),
            Command::Close => {
                if let Some(i) = self.current {
                    self.close_tab(i, false);
                }
            }
            Command::Undo => {
                if let Some(d) = self.doc_mut() {
                    d.undo();
                }
            }
            Command::Redo => {
                if let Some(d) = self.doc_mut() {
                    d.redo();
                }
            }
            Command::FitCanvas => {
                if let Some(d) = self.doc_mut() {
                    let size = d.size();
                    d.view.fit(size);
                }
            }
            Command::ActualPixels => self.zoom_to(1.0),
            Command::ZoomIn | Command::ZoomOut => {
                if let Some(d) = self.doc() {
                    let target = d.view.keyboard_zoom_target(if command == Command::ZoomIn { 1 } else { -1 });
                    self.zoom_to(target);
                }
            }
            Command::TogglePixelGrid => self.pixel_grid = !self.pixel_grid,
            Command::ToggleTransformControls => self.settings.show_controls = !self.settings.show_controls,
            Command::ToggleRulers => self.rulers = !self.rulers,
            Command::ToggleLayerVisibility => {
                if let Some(d) = self.doc_mut() {
                    if let Some(l) = d.active_layer() {
                        let (id, visible) = (l.id.clone(), l.is_visible);
                        d.set_visible(&id, !visible);
                    }
                }
            }
            Command::ToggleLayerMask => {
                if let Some(d) = self.doc_mut() {
                    if let Some(l) = d.active_layer().filter(|l| l.mask_file.is_some()) {
                        let (id, on) = (l.id.clone(), l.mask_enabled());
                        d.edit(if on { "Disable Mask" } else { "Enable Mask" }, false, |m| {
                            if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
                                l.mask_enabled = Some(!on);
                            }
                        });
                    }
                }
            }
            Command::MoveLayerUp | Command::MoveLayerDown => {
                if let Some(d) = self.doc_mut() {
                    if let Some(id) = d.active.clone() {
                        d.move_layer(&id, command == Command::MoveLayerUp);
                    }
                }
            }
            Command::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            Command::Maximize => {
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            Command::FullScreen => {
                let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
            Command::SelectTab(i) => {
                if i < self.docs.len() {
                    self.current = Some(i);
                }
            }
            Command::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Zooms around the canvas center (`EditorSession.zoom(to:)`).
    pub fn zoom_to(&mut self, zoom: f32) {
        if let Some(d) = self.doc_mut() {
            let size = d.size();
            let center = d.view.view_size / 2.0;
            d.view.set_zoom(zoom, center, size);
        }
    }

    fn save(&mut self, choose: bool) {
        let Some(i) = self.current else { return };
        let path = match (&self.docs[i].path, choose) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("Compositor project", &["comp"]).set_file_name(format!("{}.comp", self.docs[i].name)).save_file(),
        };
        let Some(path) = path else { return };
        let path = if path.extension().is_some_and(|e| e == "comp") { path } else { path.with_extension("comp") };
        if let Err(e) = self.docs[i].save(&path) {
            self.alert("Couldn’t save the project", format!("{e:#}"));
        }
    }

    fn export_png(&mut self) {
        let Some(i) = self.current else { return };
        let Some(path) = rfd::FileDialog::new().add_filter("PNG image", &["png"]).set_file_name(format!("{}.png", self.docs[i].name)).save_file() else { return };
        if let Err(e) = self.docs[i].export_png(&self.gfx, &path) {
            self.alert("Couldn’t export PNG", format!("{e:#}"));
        }
    }

    /// Closes tab `i`, asking first when it has unsaved changes (unless `discard`).
    pub fn close_tab(&mut self, i: usize, discard: bool) {
        if i >= self.docs.len() {
            return;
        }
        if self.docs[i].modified && !discard {
            self.confirm_close = Some(i);
            return;
        }
        let mut doc = self.docs.remove(i);
        doc.release(&self.gfx);
        self.current = match self.current {
            _ if self.docs.is_empty() => None,
            Some(c) if c > i => Some(c - 1),
            Some(c) if c == i => Some(i.min(self.docs.len() - 1)),
            other => other,
        };
    }

    /// Keys that aren't menu shortcuts: tool letters, as on the Mac.
    pub fn handle_keys(&mut self, ctx: &egui::Context) {
        let menus = menus::build(&self.menu_state());
        if let Some(command) = menus::shortcut_command(ctx, &menus) {
            self.run(ctx, command);
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.tool == Tool::Move {
            self.move_tool_keys(ctx);
        }
        let tools: Vec<Tool> = Tool::RAIL.iter().copied().chain([Tool::Idle]).collect();
        for tool in tools {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, tool.key()) || i.consume_key(egui::Modifiers::SHIFT, tool.key())) {
                self.select_tool(tool);
                if tool == Tool::Brush {
                    self.settings.brush_mode = crate::tools::BrushMode::Paint;
                }
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::E)) {
            self.tool = Tool::Brush;
            self.settings.brush_mode = crate::tools::BrushMode::Erase;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::X)) {
            std::mem::swap(&mut self.settings.foreground, &mut self.settings.background);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::D)) {
            self.settings.foreground = [0.0; 3];
            self.settings.background = [1.0; 3];
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
            use crate::tools::*;
            match self.tool {
                Tool::Wand => self.settings.wand = if self.settings.wand == WandMode::Wand { WandMode::Object } else { WandMode::Wand },
                Tool::Shape => {
                    self.settings.shape = ShapeKind::ALL[(self.settings.shape.index() + 1) % ShapeKind::ALL.len()];
                }
                _ => {}
            }
        }
    }
}

impl App {
    /// With the Move tool: digits set the active layer's opacity (1 = 10% … 0 = 100%) and the
    /// arrows nudge it by a pixel, ten with Shift.
    fn move_tool_keys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let digits = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        let mut opacity = None;
        for (d, key) in digits.iter().enumerate() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, *key)) {
                opacity = Some(if d == 0 { 1.0 } else { d as f64 / 10.0 });
            }
        }
        let mut nudge = egui::Vec2::ZERO;
        for (key, dir) in [(Key::ArrowLeft, egui::vec2(-1.0, 0.0)), (Key::ArrowRight, egui::vec2(1.0, 0.0)), (Key::ArrowUp, egui::vec2(0.0, -1.0)), (Key::ArrowDown, egui::vec2(0.0, 1.0))] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                nudge += dir;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, key)) {
                nudge += dir * 10.0;
            }
        }
        let Some(doc) = self.doc_mut() else { return };
        let Some(layer) = doc.active_layer() else { return };
        let (id, group, adjustment) = (layer.id.clone(), layer.is_group(), layer.adjustment.is_some());
        if let Some(o) = opacity {
            doc.set_opacity(&id, o, false);
        }
        if nudge != egui::Vec2::ZERO && !group && !adjustment {
            doc.edit("Nudge", true, |m| {
                if let Some(l) = m.layers.iter_mut().find(|l| l.id == id) {
                    l.transform.origin[0] += nudge.x as f64;
                    l.transform.origin[1] += nudge.y as f64;
                }
            });
        }
    }
}

pub fn pick_project() -> Option<PathBuf> {
    // A .comp project is a folder (a package on the Mac).
    rfd::FileDialog::new().set_title("Open Project").pick_folder()
}
