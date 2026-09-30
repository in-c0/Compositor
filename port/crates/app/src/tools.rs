//! The tool rail's tools and every tool header's settings, with the Mac's compiled defaults
//! (`ToolDefaults` with nothing persisted).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Move,
    Marquee,
    Lasso,
    Wand,
    Crop,
    Brush,
    SpotHealing,
    CloneStamp,
    Blur,
    Gradient,
    Shape,
    Type,
    Eyedropper,
    Hand,
    Zoom,
    /// No tool (A): no rail button.
    Idle,
}

impl Tool {
    /// Rail order.
    pub const RAIL: [Tool; 15] = [
        Tool::Move,
        Tool::Marquee,
        Tool::Lasso,
        Tool::Wand,
        Tool::Crop,
        Tool::Brush,
        Tool::SpotHealing,
        Tool::CloneStamp,
        Tool::Blur,
        Tool::Gradient,
        Tool::Shape,
        Tool::Type,
        Tool::Eyedropper,
        Tool::Hand,
        Tool::Zoom,
    ];

    /// `NavigationTool.label`: the rail tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Move => "Move / Transform (V)",
            Tool::Marquee => "Marquee (M)",
            Tool::Lasso => "Lasso (L)",
            Tool::Wand => "Magic (W) · Tab switches Wand and Object",
            Tool::Crop => "Crop (C)",
            Tool::Brush => "Brush (B) · Eraser (E)",
            Tool::SpotHealing => "Spot Healing Brush (J)",
            Tool::CloneStamp => "Clone Stamp (S) · Option-click sets the source",
            Tool::Blur => "Smear (R)",
            Tool::Gradient => "Gradient (G)",
            Tool::Shape => "Shape (U) · Shift-U switches Rectangle/Ellipse",
            Tool::Type => "Type (T)",
            Tool::Eyedropper => "Eyedropper (I)",
            Tool::Hand => "Hand (H)",
            Tool::Zoom => "Zoom (Z)",
            Tool::Idle => "No tool",
        }
    }

    pub fn key(self) -> eframe::egui::Key {
        use eframe::egui::Key;
        match self {
            Tool::Move => Key::V,
            Tool::Marquee => Key::M,
            Tool::Lasso => Key::L,
            Tool::Wand => Key::W,
            Tool::Crop => Key::C,
            Tool::Brush => Key::B,
            Tool::SpotHealing => Key::J,
            Tool::CloneStamp => Key::S,
            Tool::Blur => Key::R,
            Tool::Gradient => Key::G,
            Tool::Shape => Key::U,
            Tool::Type => Key::T,
            Tool::Eyedropper => Key::I,
            Tool::Hand => Key::H,
            Tool::Zoom => Key::Z,
            Tool::Idle => Key::A,
        }
    }
}

macro_rules! choice {
    ($name:ident { $($variant:ident = $text:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),* }
        #[allow(dead_code)]
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),*];
            pub fn title(self) -> &'static str { match self { $($name::$variant => $text),* } }
            pub fn index(self) -> usize { Self::ALL.iter().position(|v| *v == self).unwrap() }
        }
    };
}

choice!(BrushMode { Paint = "Paint", Erase = "Erase" });
choice!(SmearMode { Liquify = "Liquify", Blur = "Blur", Smudge = "Smudge" });
choice!(HealingType { ContentAware = "Content-Aware", CreateTexture = "Create Texture", Proximity = "Proximity Match" });
choice!(SampleLayers { This = "This Layer", All = "All Layers" });
choice!(MarqueeKind { Rectangle = "Rectangle", Ellipse = "Ellipse" });
choice!(WandMode { Wand = "Wand", Object = "Object" });
choice!(LassoKind { Freehand = "Freehand", Polygonal = "Polygonal" });
choice!(SelectionMode { New = "New", Add = "Add", Subtract = "Subtract" });
choice!(SampleSize { Point = "Point Sample", Three = "3 by 3 Average", Five = "5 by 5 Average" });
choice!(GradientKind { Linear = "Linear", Radial = "Radial" });
choice!(GradientColors { ToBackground = "Foreground to Background", ToTransparent = "Foreground to Transparent" });
choice!(ShapeKind { Rectangle = "Rectangle", Ellipse = "Ellipse", Line = "Line" });
choice!(CropRatio { Free = "Free", Original = "Original", Square = "1:1", FourThree = "4:3", ThreeFour = "3:4", Wide = "16:9", Tall = "9:16" });
choice!(Sampling { Nearest = "Nearest", Smooth = "Smooth", High = "High quality" });
choice!(Alignment { Left = "Left", Center = "Center", Right = "Right" });
choice!(MaskPaint { Hide = "Black · Hide", Reveal = "White · Reveal" });

/// A brush tip: size in px, hardness and opacity 0…1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tip {
    pub size: f64,
    pub hardness: f64,
    pub opacity: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolSettings {
    pub auto_select: bool,
    pub show_controls: bool,
    pub lock_aspect: bool,
    pub transform_sampling: Sampling,

    pub brush_mode: BrushMode,
    pub brush: Tip,
    pub smoothing: f64,
    pub clone: Tip,
    pub clone_aligned: bool,
    pub clone_sample: SampleLayers,
    pub smear: Tip,
    pub smear_mode: SmearMode,
    pub smear_radius: f64,
    pub healing: HealingType,
    pub mask_paint: MaskPaint,

    pub marquee: MarqueeKind,
    pub wand: WandMode,
    pub lasso: LassoKind,
    pub selection_mode: SelectionMode,
    pub tolerance: f64,
    pub sample_size: SampleSize,
    pub wand_layers: SampleLayers,
    pub contiguous: bool,
    pub object_layers: SampleLayers,
    pub object_edge: f64,
    pub anti_alias: bool,
    pub expand: f64,
    pub contract: f64,
    pub feather: f64,

    pub gradient: GradientKind,
    pub gradient_colors: GradientColors,
    pub gradient_reverse: bool,
    pub gradient_opacity: f64,

    pub font: String,
    pub font_size: f64,
    pub text_color: [f32; 3],
    pub alignment: Alignment,
    pub tracking: f64,
    /// 0 = Auto.
    pub leading: f64,

    pub shape: ShapeKind,
    pub line_width: f64,
    pub corner_radius: f64,

    pub crop_ratio: CropRatio,
    pub sample_ring: bool,

    pub foreground: [f32; 3],
    pub background: [f32; 3],
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            auto_select: false,
            show_controls: true,
            lock_aspect: true,
            transform_sampling: Sampling::High,
            brush_mode: BrushMode::Paint,
            brush: Tip { size: 40.0, hardness: 1.0, opacity: 1.0 },
            smoothing: 0.0,
            clone: Tip { size: 40.0, hardness: 0.0, opacity: 1.0 },
            clone_aligned: true,
            clone_sample: SampleLayers::This,
            smear: Tip { size: 40.0, hardness: 0.0, opacity: 1.0 },
            smear_mode: SmearMode::Liquify,
            smear_radius: 5.0,
            healing: HealingType::ContentAware,
            mask_paint: MaskPaint::Hide,
            marquee: MarqueeKind::Rectangle,
            wand: WandMode::Wand,
            lasso: LassoKind::Freehand,
            selection_mode: SelectionMode::New,
            tolerance: 32.0,
            sample_size: SampleSize::Point,
            wand_layers: SampleLayers::This,
            contiguous: true,
            object_layers: SampleLayers::All,
            object_edge: 0.0,
            anti_alias: true,
            expand: 1.0,
            contract: 1.0,
            feather: 2.0,
            gradient: GradientKind::Linear,
            gradient_colors: GradientColors::ToTransparent,
            gradient_reverse: false,
            gradient_opacity: 1.0,
            font: "Helvetica".into(),
            font_size: 72.0,
            text_color: [0.0; 3],
            alignment: Alignment::Left,
            tracking: 0.0,
            leading: 0.0,
            shape: ShapeKind::Rectangle,
            line_width: 4.0,
            corner_radius: 0.0,
            crop_ratio: CropRatio::Free,
            sample_ring: true,
            foreground: [0.0; 3],
            background: [1.0; 3],
        }
    }
}

/// The per-tool hint in the status bar (exact strings from `ContentView.statusBar`).
pub fn hint(tool: Tool, s: &ToolSettings) -> String {
    match tool {
        Tool::Marquee if s.marquee == MarqueeKind::Ellipse => "Drag an ellipse · Shift add · Option subtract · Shift again mid-drag circle · Drag inside to move · Delete clears · ⌘D deselect".into(),
        Tool::Marquee => "Drag a rectangle · Shift add · Option subtract · Shift again mid-drag square · Drag inside to move · ⌘-drag moves pixels · Delete clears · ⌘D deselect".into(),
        Tool::Wand if s.wand == WandMode::Object => "Click an object to select its outline · Tab for Wand · Shift add · Option subtract · Drag inside to move · ⌘-drag moves pixels · Delete clears · ⌘D deselect".into(),
        Tool::Wand => "Click to select similar colors · Tab for Object · Shift add · Option subtract · Drag inside to move · ⌘-drag moves pixels · Delete clears · ⌘D deselect".into(),
        Tool::Lasso if s.lasso == LassoKind::Freehand => "Drag to select · Drag inside to move · Shift add · Option subtract · Delete clears · ⌥⌫/⌘⌫ fill · ⌘D deselect".into(),
        Tool::Lasso => "Click corners · Click start, double-click or Enter to close · Delete removes corner · Escape cancel".into(),
        Tool::Brush => format!(
            "{} · [ ] size · Shift-[ ] hardness · 1–0 opacity · Escape cancel · Space to pan",
            if s.brush_mode == BrushMode::Erase { "Drag to erase" } else { "Drag to paint" }
        ),
        Tool::Blur => format!(
            "{} · [ ] size · Shift-[ ] hardness · 1–0 strength · Space to pan",
            match s.smear_mode {
                SmearMode::Blur => "Drag to soften",
                SmearMode::Smudge => "Drag to smudge",
                SmearMode::Liquify => "Drag to push pixels",
            }
        ),
        Tool::CloneStamp => "Option-click to set the source · Drag to clone · [ ] size · Shift-[ ] hardness · 1–0 opacity · Space to pan".into(),
        Tool::SpotHealing => "Drag over blemishes to heal · [ ] size · Shift-[ ] hardness · Escape cancel · Space to pan".into(),
        Tool::Type => "Drag a text box · Click text to edit · Drag box handles to resize · ⌘Return finish · Escape cancel".into(),
        Tool::Shape => format!(
            "Drag to draw a shape on a new layer · Shift {} · Option from center · Shift-U or Tab for the next shape · Escape cancel · Space to pan",
            match s.shape {
                ShapeKind::Line => "45°",
                ShapeKind::Rectangle => "square",
                ShapeKind::Ellipse => "circle",
            }
        ),
        Tool::Gradient => "Drag to draw · Drag ends to adjust · Shift 45° · 1–0 opacity · Enter apply · Escape cancel".into(),
        Tool::Crop => "Drag to crop · Enter apply · Escape cancel · Space to pan".into(),
        Tool::Move => "Drag to move · Handles to resize · Circle to rotate · 1–0 layer opacity · Space to pan".into(),
        Tool::Hand => "Drag to pan · Pinch to zoom".into(),
        Tool::Idle => "No tool selected · Press a tool's key to pick one · Space to pan".into(),
        Tool::Zoom => "Click to zoom in · Option-click to zoom out · Drag right or left to zoom smoothly · Space to pan".into(),
        // The Mac's chain of conditions has no Eyedropper case, so it falls through to Zoom's text.
        Tool::Eyedropper => "Click to zoom in · Option-click to zoom out · Drag right or left to zoom smoothly · Space to pan".into(),
    }
}

/// A tool name as `parity/ui/states.toml` spells it, with the mode it implies.
pub fn from_state_name(name: &str, settings: &mut ToolSettings) -> Option<Tool> {
    Some(match name {
        "move" => Tool::Move,
        "marquee" => Tool::Marquee,
        "lasso" => Tool::Lasso,
        "magic" | "wand" => Tool::Wand,
        "crop" => Tool::Crop,
        "brush" => Tool::Brush,
        "eraser" => {
            settings.brush_mode = BrushMode::Erase;
            Tool::Brush
        }
        "spot-healing" => Tool::SpotHealing,
        "clone-stamp" => Tool::CloneStamp,
        "blur" | "smear" => Tool::Blur,
        "gradient" => Tool::Gradient,
        "shape" => Tool::Shape,
        "type" => Tool::Type,
        "eyedropper" => Tool::Eyedropper,
        "hand" => Tool::Hand,
        "zoom" => Tool::Zoom,
        "idle" => Tool::Idle,
        _ => return None,
    })
}
