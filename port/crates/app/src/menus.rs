//! The menu bar, built from one description that the window draws, the keyboard dispatches and
//! `--render-ui` dumps to `menus.json` for comparison with the Mac's `NSApp.mainMenu`.
//! Items, order, separators and shortcuts follow docs/port/ui-inventory.md §3; ⌘ is Ctrl on Windows.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key: Key,
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    /// The Mac's Control key (⌃); only system items use it.
    pub control: bool,
}

const fn cmd(key: Key) -> Shortcut {
    Shortcut { key, command: true, shift: false, option: false, control: false }
}
const fn shift_cmd(key: Key) -> Shortcut {
    Shortcut { key, command: true, shift: true, option: false, control: false }
}
const fn opt_cmd(key: Key) -> Shortcut {
    Shortcut { key, command: true, shift: false, option: true, control: false }
}
const fn opt_shift_cmd(key: Key) -> Shortcut {
    Shortcut { key, command: true, shift: true, option: true, control: false }
}
const fn opt(key: Key) -> Shortcut {
    Shortcut { key, command: false, shift: false, option: true, control: false }
}
const fn shift(key: Key) -> Shortcut {
    Shortcut { key, command: false, shift: true, option: false, control: false }
}
const fn ctrl_cmd(key: Key) -> Shortcut {
    Shortcut { key, command: true, shift: false, option: false, control: true }
}

fn key_name(key: Key, mac: bool) -> &'static str {
    match key {
        Key::Backspace => if mac { "⌫" } else { "Backspace" },
        Key::Equals | Key::Plus => "=",
        Key::Minus => "-",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Space => "Space",
        other => other.symbol_or_name(),
    }
}

impl Shortcut {
    /// The Mac's notation, ⌃⌥⇧⌘ then the key: what `menus.json` records on both platforms.
    pub fn mac(&self) -> String {
        let mut s = String::new();
        for (on, glyph) in [(self.control, "⌃"), (self.option, "⌥"), (self.shift, "⇧"), (self.command, "⌘")] {
            if on {
                s.push_str(glyph);
            }
        }
        s + key_name(self.key, true)
    }

    /// What the menu shows on this platform.
    pub fn display(&self) -> String {
        if cfg!(target_os = "macos") {
            return self.mac();
        }
        let mut parts = Vec::new();
        if self.command || self.control {
            parts.push("Ctrl");
        }
        if self.option {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(key_name(self.key, false));
        parts.join("+")
    }

    fn egui(&self) -> KeyboardShortcut {
        let mut m = Modifiers::NONE;
        m.command = self.command;
        m.shift = self.shift;
        m.alt = self.option;
        if cfg!(target_os = "macos") {
            m.mac_cmd = self.command;
            m.ctrl = self.control;
        } else {
            m.ctrl = self.command || self.control;
        }
        KeyboardShortcut::new(m, self.key)
    }

    fn modifier_count(&self) -> usize {
        [self.command, self.shift, self.option, self.control].iter().filter(|b| **b).count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    NewCanvas,
    Open,
    OpenRecent(usize),
    ClearRecent,
    Save,
    SaveAs,
    ExportPng,
    Close,
    Undo,
    Redo,
    FitCanvas,
    ActualPixels,
    ZoomIn,
    ZoomOut,
    TogglePixelGrid,
    ToggleTransformControls,
    ToggleRulers,
    ToggleLayerVisibility,
    ToggleLayerMask,
    MoveLayerUp,
    MoveLayerDown,
    Minimize,
    Maximize,
    FullScreen,
    SelectTab(usize),
    Exit,
}

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub title: String,
    pub shortcut: Option<Shortcut>,
    pub command: Option<Command>,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub children: Vec<Item>,
    pub separator: bool,
    /// Supplied by the operating system on the Mac rather than declared by the app.
    pub system: bool,
}

impl Item {
    fn new(title: &str) -> Self {
        Self { title: title.into(), ..Default::default() }
    }
    fn key(mut self, s: Shortcut) -> Self {
        self.shortcut = Some(s);
        self
    }
    /// Wired to `command`, enabled when `enabled`.
    fn run(mut self, command: Command, enabled: bool) -> Self {
        self.command = Some(command);
        self.enabled = enabled;
        self
    }
    fn check(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
    fn system(mut self) -> Self {
        self.system = true;
        self
    }
    fn sub(mut self, children: Vec<Item>) -> Self {
        self.enabled = children.iter().any(|c| c.enabled || !c.children.is_empty()) || !children.is_empty();
        self.children = children;
        self
    }
    fn separator() -> Self {
        Self { separator: true, ..Default::default() }
    }
}

pub struct Menu {
    pub title: &'static str,
    pub items: Vec<Item>,
}

/// What the menus need to know about the app.
pub struct MenuState {
    pub has_document: bool,
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub move_tool: bool,
    pub show_controls: bool,
    pub pixel_grid: bool,
    pub rulers: bool,
    pub active_layer_visible: Option<bool>,
    pub active_layer_clipped: bool,
    pub can_move_up: bool,
    pub can_move_down: bool,
    pub recent: Vec<String>,
    pub tabs: Vec<(String, bool)>,
}

/// Items the port doesn't do yet: present, in place, disabled.
fn later(title: &str) -> Item {
    Item::new(title)
}

pub fn build(s: &MenuState) -> Vec<Menu> {
    let doc = s.has_document;
    let mut recent: Vec<Item> = s.recent.iter().enumerate().map(|(i, name)| Item::new(name).run(Command::OpenRecent(i), true)).collect();
    if !recent.is_empty() {
        recent.push(Item::separator());
    }
    recent.push(Item::new("Clear Menu").run(Command::ClearRecent, !s.recent.is_empty()));

    let file = vec![
        Item::new("New Canvas…").key(cmd(Key::N)).run(Command::NewCanvas, true),
        Item::new("Open Project…").key(cmd(Key::O)).run(Command::Open, true),
        Item::new("Open Recent").sub(recent),
        later("Import Images…"),
        Item::new("Save").key(cmd(Key::S)).run(Command::Save, doc),
        Item::new("Save As…").key(shift_cmd(Key::S)).run(Command::SaveAs, doc),
        Item::separator(),
        Item::new("Export PNG…").key(shift_cmd(Key::E)).run(Command::ExportPng, doc),
        later("Export JPEG…").key(opt_shift_cmd(Key::S)),
        Item::separator(),
        Item::new("Close Project").key(cmd(Key::W)).run(Command::Close, doc),
        Item::separator(),
        later("Page Setup…").key(shift_cmd(Key::P)).system(),
        later("Print…").key(cmd(Key::P)).system(),
        Item::separator().system(),
        // Quit lives in the Mac's app menu; Windows puts it at the foot of File.
        Item::new("Exit").run(Command::Exit, true).system(),
    ];

    let undo = match &s.undo {
        Some(action) => format!("Undo {action}"),
        None => "Undo".into(),
    };
    let redo = match &s.redo {
        Some(action) => format!("Redo {action}"),
        None => "Redo".into(),
    };
    let edit = vec![
        Item::new(&undo).key(cmd(Key::Z)).run(Command::Undo, s.undo.is_some()),
        Item::new(&redo).key(shift_cmd(Key::Z)).run(Command::Redo, s.redo.is_some()),
        later("Cut").key(cmd(Key::X)),
        later("Copy").key(cmd(Key::C)),
        later("Copy Merged").key(shift_cmd(Key::C)),
        later("Paste").key(cmd(Key::V)),
        Item::separator(),
        later("Keyboard Shortcuts…"),
        later("Fill with Foreground Color").key(opt(Key::Backspace)),
        later("Fill with Background Color").key(cmd(Key::Backspace)),
        later("Clear Selection Pixels"),
        later("Content-Aware Fill…").key(shift(Key::Backspace)),
    ];

    let select = vec![
        later("All").key(cmd(Key::A)),
        later("Deselect").key(cmd(Key::D)),
        later("Inverse").key(shift_cmd(Key::I)),
        later("Layer's Pixels"),
        later("Subject").key(opt_cmd(Key::A)),
        later("Color Range…"),
        later("Mask's Black Areas"),
        Item::separator(),
        later("Expand…"),
        later("Contract…"),
        later("Feather…"),
    ];

    let image = vec![
        later("Curves…").key(cmd(Key::M)),
        later("Levels…").key(cmd(Key::L)),
        later("Hue/Saturation…").key(cmd(Key::U)),
        later("Black & White…"),
        later("Color Balance…"),
        later("Exposure…"),
        later("Gradient Map…"),
        later("Grain…"),
        later("Invert").key(cmd(Key::I)),
        Item::separator(),
        later("Canvas Size…").key(opt_cmd(Key::C)),
        later("Image Size…").key(opt_cmd(Key::I)),
        later("Trim…"),
        Item::separator(),
        later("Flip Canvas Horizontal"),
        later("Flip Canvas Vertical"),
    ];

    let filter = [
        "Gaussian Blur…",
        "Motion Blur…",
        "Add Noise…",
        "Vignette…",
        "Bloom / Glow…",
        "Dither…",
        "Tonal Contrast…",
        "Lens Correction…",
        "Camera Raw Filter…",
        "Remove Background…",
    ]
    .iter()
    .map(|t| later(t))
    .collect();

    let adjustments = [
        "Hue/Saturation…",
        "Levels…",
        "Curves…",
        "Exposure…",
        "Gradient Map…",
        "Grain…",
        "Add Noise…",
        "Gaussian Blur…",
        "Motion Blur…",
        "Invert",
        "Black & White…",
        "Color Balance…",
    ]
    .iter()
    .map(|t| later(t))
    .collect();
    let visible = s.active_layer_visible;
    let layer = vec![
        Item::new("New Adjustment Layer").sub(adjustments),
        later("Edit Adjustment…"),
        Item::separator(),
        later("Transform Layer").key(cmd(Key::T)),
        later("Duplicate Layer").key(cmd(Key::J)),
        Item::separator(),
        later(if s.active_layer_clipped { "Release Clipping Mask" } else { "Create Clipping Mask" }).key(opt_cmd(Key::G)),
        Item::separator(),
        later("Group Selected Layers").key(cmd(Key::G)),
        later("Ungroup Layers").key(shift_cmd(Key::G)),
        later("Move Out of Folder"),
        later("New Blank Layer").key(shift_cmd(Key::N)),
        later("Rename Layer…"),
        Item::new(if visible == Some(false) { "Show Layer" } else { "Hide Layer" }).run(Command::ToggleLayerVisibility, visible.is_some()),
        Item::separator(),
        Item::new("Move Layer Up").key(cmd(Key::CloseBracket)).run(Command::MoveLayerUp, s.can_move_up),
        Item::new("Move Layer Down").key(cmd(Key::OpenBracket)).run(Command::MoveLayerDown, s.can_move_down),
        later("Merge Down").key(cmd(Key::E)),
        Item::separator(),
        later("Flip Layer Horizontal"),
        later("Flip Layer Vertical"),
        Item::separator(),
        later("Delete Layer"),
    ];

    let view = vec![
        Item::new("Fit Canvas").key(cmd(Key::Num0)).run(Command::FitCanvas, doc),
        Item::new("Actual Pixels").key(cmd(Key::Num1)).run(Command::ActualPixels, doc),
        Item::new("Zoom In").key(cmd(Key::Equals)).run(Command::ZoomIn, doc),
        Item::new("Zoom Out").key(cmd(Key::Minus)).run(Command::ZoomOut, doc),
        Item::new("Pixel Grid (800% and above)").run(Command::TogglePixelGrid, true).check(s.pixel_grid),
        later("Snap").check(true),
        Item::new("Show Transform Controls").key(cmd(Key::H)).run(Command::ToggleTransformControls, s.move_tool).check(s.show_controls),
        Item::separator(),
        Item::new("Show").sub(vec![later("Grid").key(cmd(Key::Quote)).check(false), later("Guides").key(cmd(Key::Semicolon)).check(true)]),
        later("Grid Settings…"),
        Item::new("Rulers").key(cmd(Key::R)).run(Command::ToggleRulers, doc).check(s.rulers),
        Item::separator(),
        later("Snap").key(shift_cmd(Key::Semicolon)).check(true),
        Item::new("Snap To").sub(vec![
            later("Guides").check(true),
            later("Grid").check(false),
            later("Layers").check(true),
            later("Document Bounds").check(true),
        ]),
        Item::separator(),
        later("Lock Guides").key(opt_cmd(Key::Semicolon)).check(false),
        later("Clear Guides"),
        Item::new("Enter Full Screen").key(ctrl_cmd(Key::F)).run(Command::FullScreen, true).system(),
    ];

    let mut window = vec![
        Item::new("Minimize").run(Command::Minimize, true).system(),
        Item::new("Zoom").run(Command::Maximize, true).system(),
        Item::separator().system(),
        later("Bring All to Front").system(),
    ];
    if !s.tabs.is_empty() {
        window.push(Item::separator().system());
        for (i, (name, active)) in s.tabs.iter().enumerate() {
            window.push(Item::new(name).run(Command::SelectTab(i), true).check(*active).system());
        }
    }

    let help = vec![later("Compositor Help").system()];

    vec![
        Menu { title: "File", items: file },
        Menu { title: "Edit", items: edit },
        Menu { title: "Select", items: select },
        Menu { title: "Image", items: image },
        Menu { title: "Filter", items: filter },
        Menu { title: "Layer", items: layer },
        Menu { title: "View", items: view },
        Menu { title: "Window", items: window },
        Menu { title: "Help", items: help },
    ]
}

/// What the Layers list's context menu needs about the row it opened on.
pub struct RowState {
    pub is_folder: bool,
    pub clipped: bool,
    pub visible: bool,
    /// `Some(enabled)` when the layer has a mask.
    pub mask: Option<bool>,
    pub mask_linked: bool,
}

/// `NativeLayerList.Coordinator.contextMenu(for:)`; no key equivalents are shown.
pub fn layer_context(r: &RowState) -> Vec<Item> {
    let mut items = vec![
        later("Duplicate Layer"),
        later("Rename…"),
        later(if r.mask.is_some() { "Delete Mask" } else { "Delete Layer" }),
        Item::separator(),
        later(if r.clipped { "Release Clipping Mask" } else { "Create Clipping Mask" }),
        later("Group Selected Layers"),
    ];
    if r.is_folder {
        items.push(later("Ungroup Layers"));
    }
    items.extend([
        later("Move Out of Folder"),
        later(if r.is_folder { "Merge Group" } else { "Merge Down" }),
        Item::separator(),
        Item::new("Add Mask").sub(vec![later("Reveal All (White)"), later("Hide All (Black)")]),
        Item::new(if r.mask == Some(false) { "Enable Mask" } else { "Disable Mask" }).run(Command::ToggleLayerMask, r.mask.is_some()),
        later("Delete Mask"),
        later(if r.mask.is_some() && !r.mask_linked { "Link Mask" } else { "Unlink Mask" }),
        Item::separator(),
        Item::new(if r.visible { "Hide Layer" } else { "Show Layer" }).run(Command::ToggleLayerVisibility, true),
    ]);
    items
}

/// Draws `items` as a context menu's contents; returns the command chosen.
pub fn menu_items(ui: &mut egui::Ui, list: &[Item]) -> Option<Command> {
    let mut chosen = None;
    items(ui, list, &mut chosen);
    chosen
}

/// Runs the first enabled item whose shortcut was pressed. Shortcuts with more modifiers go
/// first, since egui lets a plain ⌘Z match while Shift is held too.
pub fn shortcut_command(ctx: &egui::Context, menus: &[Menu]) -> Option<Command> {
    fn collect<'a>(items: &'a [Item], out: &mut Vec<&'a Item>) {
        for item in items {
            if item.shortcut.is_some() {
                out.push(item);
            }
            collect(&item.children, out);
        }
    }
    let mut items = Vec::new();
    for menu in menus {
        collect(&menu.items, &mut items);
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.shortcut.unwrap().modifier_count()));
    let typing = ctx.egui_wants_keyboard_input();
    for item in items {
        let shortcut = item.shortcut.unwrap();
        // Full screen is F11 on Windows; the Mac's ⌃⌘F has no Windows equivalent.
        if shortcut.control && !cfg!(target_os = "macos") {
            continue;
        }
        // A focused field keeps its own undo and editing keys.
        if typing && matches!(item.command, Some(Command::Undo | Command::Redo)) {
            continue;
        }
        let mut hit = ctx.input_mut(|i| i.consume_shortcut(&shortcut.egui()));
        if !hit && shortcut.key == Key::Equals {
            hit = ctx.input_mut(|i| i.consume_shortcut(&Shortcut { key: Key::Plus, ..shortcut }.egui()));
        }
        if hit && item.enabled {
            if let Some(c) = item.command {
                return Some(c);
            }
        }
    }
    if !cfg!(target_os = "macos") && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F11)) {
        return Some(Command::FullScreen);
    }
    None
}

/// Draws the menu bar; returns the command chosen.
pub fn bar(ui: &mut egui::Ui, menus: &[Menu]) -> Option<Command> {
    let mut chosen = None;
    egui::MenuBar::new().ui(ui, |ui| {
        for menu in menus {
            ui.menu_button(menu.title, |ui| {
                ui.set_min_width(220.0);
                items(ui, &menu.items, &mut chosen);
            });
        }
    });
    chosen
}

fn items(ui: &mut egui::Ui, list: &[Item], chosen: &mut Option<Command>) {
    for item in list {
        if item.separator {
            ui.separator();
            continue;
        }
        let title = match item.checked {
            Some(true) => format!("✓  {}", item.title),
            Some(false) => format!("     {}", item.title),
            None => item.title.clone(),
        };
        if !item.children.is_empty() {
            ui.menu_button(title, |ui| {
                ui.set_min_width(180.0);
                items(ui, &item.children, chosen);
            });
            continue;
        }
        let mut button = egui::Button::new(title);
        if let Some(s) = item.shortcut {
            button = button.shortcut_text(s.display());
        }
        if ui.add_enabled(item.enabled, button).clicked() {
            *chosen = item.command;
            ui.close();
        }
    }
}

/// The menu tree as JSON (schema in parity/README.md, "UI states").
pub fn to_json(menus: &[Menu]) -> Value {
    Value::Array(menus.iter().map(|m| json!({ "title": m.title, "children": items_json(&m.items) })).collect())
}

pub fn items_json(items: &[Item]) -> Value {
    fn item(i: &Item) -> Value {
        if i.separator {
            let mut v = json!({ "separator": true });
            if i.system {
                v["system"] = json!(true);
            }
            return v;
        }
        let mut v = json!({
            "title": i.title,
            "shortcut": i.shortcut.map(|s| s.mac()),
            "enabled": i.enabled,
        });
        if let Some(c) = i.checked {
            v["checked"] = json!(c);
        }
        if !i.children.is_empty() {
            v["children"] = Value::Array(i.children.iter().map(item).collect());
        }
        if i.system {
            v["system"] = json!(true);
        }
        v
    }
    Value::Array(items.iter().map(item).collect())
}
