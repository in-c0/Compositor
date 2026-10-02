# Compositor UI inventory (for the egui/eframe rebuild and PNG parity checks)

Source: `D:\Projects\Compositor\Compositor` (read-only survey, 2026-09-30). Paths below are relative to that folder.
Units are SwiftUI/AppKit points. "px" in labels means document pixels. `⌘ ⌥ ⌃ ⇧` = Command, Option, Control, Shift.
Where a value comes from system chrome and is not set in code, it is marked **(system)**.

---

## 1. Window layout and global style

### 1.1 Scene and window

| Item | Value | Source |
|---|---|---|
| Scene | single `Window("Compositor", id: "editor")` hosting `ProjectWorkspaceView(applicationDelegate:)` + `.roundedControls()` | `CompositorApp.swift` |
| Default size | 1180 × 780 (`.defaultSize`); first launch uses `defaultWindowPlacement` = display `visibleRect` (fills screen, not full-screen); later launches restore last frame | `CompositorApp.swift` |
| Min size | 800 × 520 (`.frame(minWidth:minHeight:)` on the editor) | `ContentView.swift` |
| Toolbar style | `.unifiedCompact(showsTitle: false)`; window `titleVisibility = .hidden`; `representedURL` = project URL; `isDocumentEdited` = session.isModified (dot in close button) | `CompositorApp.swift`, `UI/ProjectWindowBridge.swift` |
| Appearance | Always dark: `NSApp.appearance = .darkAqua` in `applicationWillFinishLaunching`; `ContentView` also `.preferredColorScheme(.dark)` | `IO/CompositorApplicationDelegate.swift` |
| Accent color | `AccentColor.colorset` is empty, so the **system accent** (user setting; default blue ≈ #0A84FF in dark) is used everywhere `Color.accentColor` / `NSColor.controlAccentColor` appears | `Assets.xcassets` |
| Control shape | `.roundedControls()` = `.buttonBorderShape(.capsule)` on every bordered button and pop-up; AppKit pop-ups set `borderShape = .capsule` explicitly | `ContentView.swift` |
| Slider behavior | `SliderSnap.install()` swizzles `NSSliderCell.startTracking` so a track click jumps the knob (no glide) | `UI/SliderSnap.swift` |
| Fonts | System font (SF Pro) only. No custom fonts. Monospaced digits used in status bar, tab widths, readouts | all |

### 1.2 Vertical/horizontal structure (`ContentView.editorStack`)

```
Window (title bar = unified compact toolbar, 1 row)
 ├─ Toolbar: [traffic lights] [+ New canvas] [fixed spacer] [ProjectTabStrip w=max(200, winW−352) h=34] [flex spacer] [Fit] [100%] [⊕ ⊖]
 └─ Content (background Color(white: 0.14))
    ├─ Tool header bar (h=42) — one of the headers in §2.3, then Divider
    ├─ HStack(spacing 0)
    │   ├─ Tool rail (w=56, IndicatorlessScrollView, no scroller)
    │   ├─ Divider (1pt, system separator)
    │   ├─ VStack: [ruler corner 18×18 + horizontal ruler h=18]? / [vertical ruler w=18]? + ZStack{EditorCanvas, welcome NewCanvasSheet if no document, MaskAloneBadge at bottom +14}
    │   ├─ PanelResizeEdge (Divider + 8pt invisible drag strip, cursor .columnResize, help "Drag to resize the panel")
    │   └─ LayersPanel (width = @AppStorage "layersPanelWidth", default 252, range 202…352)
    ├─ Divider
    └─ Status bar (h=30, fixedSize vertically)
```

- Rulers show only when `session.showsRulers && document != nil`.
- Drop highlight: `RoundedRectangle(cornerRadius: 8).strokeBorder(accent, lineWidth: 3)` inset 3 from the canvas frame while a valid drag hovers.
- There are **no docked right-side panels other than Layers**. All tool dialogs are floating `NSPanel`s (§1.5) or window sheets (§2.5).

### 1.3 Toolbar items (`ContentView.toolbar`)

| Placement | Control | Symbol / label | Help text |
|---|---|---|---|
| navigation | Button, Label("New canvas") | `plus` | "New canvas (⌘N)"; while dragging files: "Open in a new project tab" / "New canvas (⌘N) · Drop images here for new tabs"; drop outline RoundedRectangle r=6, accent 2pt |
| navigation | `ToolbarSpacer(.fixed)` | | |
| navigation | `ProjectTabStrip` (only when a workspace exists) `.sharedBackgroundVisibility(.hidden)` | | |
| navigation | `ToolbarSpacer(.flexible)` | | |
| primaryAction | Button "Fit" `.padding(.horizontal, 4)` | text | "Fit canvas in window (⌘0)" |
| primaryAction | Button "100%" | text | "Actual pixels (⌘1)" |
| primaryAction | HStack(spacing 0) of 2 buttons | `plus.magnifyingglass`, `minus.magnifyingglass` | "Zoom in (⌘+)", "Zoom out (⌘−)" |

`ToolbarSpacer` and `sharedBackgroundVisibility` are macOS 26 APIs; the toolbar is Liquid Glass (system material).

### 1.4 Project tabs (`UI/ProjectTabs.swift`, `UI/ProjectTabLayout.swift`)

| Element | Geometry / style |
|---|---|
| Strip | GeometryReader, height 34; tabs laid out in a ZStack at y offset 3; remaining width is a `TitleBarDragView` (window drag) |
| Tab pill | height 28, `Capsule` background white 12% (active) / 3.5% (inactive); border white 22% / 8%, 1pt; drop-targeted: accent 30% fill, accent 2pt border |
| Tab label | `system 12`, `.semibold` active / `.medium` inactive, 1 line; label width = clamp(ceil(textWidth) + (modified ? 10 : 0), 35, 155); leading pad 11, trailing 8 |
| Modified dot | `Circle` 5×5, 5pt gap before title |
| Close button | `xmark` 9pt semibold, `.secondary`, frame 16×28 + trailing 5 |
| Pill width | labelWidth + 40 |
| Spacing | 6 between pills (`projectTabSpacing`) |
| Overflow pill | at x=0: "N more tabs" / "1 more tab" 12pt medium + `chevron.down` 9pt medium, HStack spacing 4, h-pad 11, height 28, white 3.5% capsule, border white 8%; width = ceil(text) + 11 + 4 + 10 + 11; opens NSMenu listing hidden tabs ("• " prefix if modified) |
| New drop slot (only during external drag) | Label("New", `plus`) 12pt medium, h-pad 14, height 28, capsule white 4% with dashed `secondary` border [4,3] 1pt; targeted: accent 30% + solid accent 2pt |
| Overflow rule | tabs dropped from the front (oldest) until pill + tabs fit; selected tab is never hidden |
| Reorder | drag ≥3pt; others animate `.easeOut(0.15)` |
| Default names | "Untitled", then "Untitled 2", "Untitled 3"… |

### 1.5 Floating panels (`UI/FloatingPanel.swift`)

`FloatingPanelController(name:)` wraps an `NSPanel(styleMask: [.titled, .closable])`, `isFloatingPanel = true`, `hidesOnDeactivate = true`. Content = `NSHostingView(rootView: AnyView(content.roundedControls()))` sized once to `fittingSize`. The title-bar close button acts as Cancel (`onClose`).

| Placement | Behavior |
|---|---|
| `.automatic` | first open centered on the canvas (`CanvasView` center in screen), afterwards at the last top-left for that panel name (per app session) |
| `.dockedToMainWindowRight` | Camera Raw only: width `dockedWidth = 440`, full height of the document window, flush right; follows window move/resize |

Panel instances (name → title → content):

| name | Title | Content |
|---|---|---|
| levelsPanel | "Levels" | `LevelsSheet` |
| colorRangePanel | "Color Range" | `ColorRangeSheet` |
| adjustmentPanel | "Hue/Saturation" | `HueSaturationSheet` |
| effectsPanel | effect kind raw value, e.g. "Drop Shadow" | `EffectsSheet` |
| selectionAmountPanel | "Expand Selection" / "Contract Selection" / "Feather Selection" | `SelectionAmountSheet` |
| filterPanel | `FilterKind.rawValue` (e.g. "Gaussian Blur", "Camera Raw Filter") | `FilterSheet` |
| colorPickerPanel | `ColorPickerTarget.title` (see §2.5.12) | `ColorPickerSheet` |
| keyboardShortcuts | "Keyboard Shortcuts" | `KeyboardShortcutsSheet` |

### 1.6 Shared metrics and colors

| Constant | Value | Source |
|---|---|---|
| Tool header height | 42 | `ToolHeaderStyle.height` |
| Tool header title font | `system 13 semibold` | `ToolHeaderStyle.titleFont` |
| Tool header control font | `system 12`, `.controlSize(.regular)` | `ToolHeaderStyle.controlFont` |
| Tool header h-padding | 18 | each header |
| Unit suffix | HStack spacing 2 (field + "px"/"%" text) | `unitSuffix` |
| Editor background | `Color(white: 0.14)` | ContentView |
| Canvas backdrop | white 0.105 | EditorCanvas.draw |
| Document shadow | offset (0, 3), blur 14, black 35% | EditorCanvas.draw |
| Checkerboard | base white 0.30, alternate squares white 0.35, 10pt tiles from the doc's top-left | EditorCanvas.draw |
| Document edge | white 13%, hairline (1/backingScale) | EditorCanvas.draw |
| Fit margin | 48 per side (`viewSize − 96`) | `CanvasViewport.fit` |
| Keyboard zoom steps | 12.5, 16.7, 25, 33.3, 50, 66.7, 100, 125, 150, 200, 300, 400, 500, 600, 800, 1200, 1600 % | `CanvasViewport` |
| Zoom range | 0.1%…3200% | `CanvasViewport.zoomRange` |
| Crisp pixels from | 200%; pixel grid from 800% | `CanvasView.crispZoom`, `pixelGridZoom` |
| Ruler thickness | 18; bg white 0.2; ticks white 0.62 (major 8, mid 5, minor 3 long); labels white 0.78 `monospacedDigitSystemFont 8`; inner edge white 0.08 hairline; major step = first of 1,2,5,10,20,25,50,100,200,250,500,1000,2000,2500,5000,10000,20000,25000 px ≥ 70pt; vertical labels rotated −90° | `UI/CanvasRulers.swift` |
| Ruler corner | 18×18 white 0.2 with a diagonal line (5,14)→(14,5) white 28% | `CanvasRulerCorner` |
| Selection (marching ants) | 1pt white line + black dash [4,4] animated phase | `Rendering/TransformOverlay.swift` |
| Transform box | accent 1pt; handles 7×7 white fill, accent stroke | TransformOverlay |
| Snap lines | accent 1pt | TransformOverlay |
| Crop | outside dim black 60%; frame white 1pt; thirds white 40%; handles 8×8 white/black | TransformOverlay |
| Lasso draft | black 80% 2pt under white 1pt; polygonal first-point handle 8×8 | TransformOverlay |
| Grid | preset color × opacity (default Light Gray 0.7, 45%); style Lines/Dashed [4,3]/Dots [1,2]; subdivisions dotted, fainter | `Document/Guides.swift` |
| Mask-alone badge | NSView h=26, corner 13, bg black 75%, border white 14% 1pt; stack spacing 7, insets L11 R8: `rectangle.inset.filled` 11pt white, "Layer Mask" 12 semibold white, layer name 12 white 60% (max 220, truncating), `xmark` 9pt bold button | `UI/LayerMaskMenu.swift` |

### 1.7 Tool rail (`ContentView.toolRail`)

Width 56, `VStack(spacing: 10)`, top padding 16, bottom 12. Each button 36×36, `.buttonStyle(.plain)`, `.foregroundStyle(.primary)`, symbol `.font(.system(size: 17))`; selected: fill white 12% + border white 14%, `RoundedRectangle(cornerRadius: 7)`. Tooltip = `NavigationTool.label`. After the tools, `ColorPaletteControls` with top padding 8.

| # | Tool (`NavigationTool`) | Symbol (state-dependent) | Tooltip | Key |
|---|---|---|---|---|
| 1 | move | `arrow.up.left.and.arrow.down.right` | Move / Transform (V) | V |
| 2 | marquee | `rectangle.dashed`; `circle.dashed` when Ellipse | Marquee (M) | M (cycles shape) |
| 3 | lasso | `lasso`; custom `PolygonalLassoToolIcon` 18×18 when Polygonal | Lasso (L) | L (cycles) |
| 4 | wand | `wand.and.stars`; custom `ObjectSelectionToolIcon` 18×18 when Object | Magic (W) · Tab switches Wand and Object | W |
| 5 | crop | `crop` | Crop (C) | C |
| 6 | brush | `paintbrush.pointed`; `eraser` when mode Erase | Brush (B) · Eraser (E) | B / E |
| 7 | spotHealing | `bandage` | Spot Healing Brush (J) | J |
| 8 | cloneStamp | custom `CloneStampToolIcon` 18×18 (SF has `seal` as fallback symbol) | Clone Stamp (S) · Option-click sets the source | S |
| 9 | blur | `drop` | Smear (R) | R |
| 10 | gradient | custom `GradientToolIcon` 18×18 (16×16 Floyd–Steinberg dither in rounded rect r=3.5, stroke 1.4) | Gradient (G) | G |
| 11 | shape | `square.on.circle` | Shape (U) · Shift-U switches Rectangle/Ellipse | U |
| 12 | type | `textformat` | Type (T) | T |
| 13 | eyedropper | `eyedropper` | Eyedropper (I) | I |
| 14 | hand | `hand.draw` | Hand (H) | H |
| 15 | zoom | `magnifyingglass` | Zoom (Z) | Z |
| — | idle (no button) | — | "No tool" | A |

Default tool: `.move`. Custom icons are SwiftUI `Canvas` paths (exact coordinates in `UI/BrushControls.swift`, `UI/LassoControls.swift`, `UI/GradientControls.swift`).

**Color palette** (`UI/ColorPaletteControls.swift`), frame 36×36: background swatch 24×24 offset (12,12) under foreground swatch 24×24 at (0,0); each `RoundedRectangle(cornerRadius: 6, .continuous)` fill color, inner white 1.5pt stroke inset 1, outer black 1pt. Swap button `arrow.left.and.right` 9pt medium rotated 45°, frame 12×12 at (27, −3), `.secondary`, help "Swap foreground and background (X)". Reset `arrow.counterclockwise` 7.5pt medium, 12×12 at (−1, 27), help "Default colors (D)". Defaults: foreground black, background white. When a mask is selected, clicking a swatch shows a popover: headline "Mask foreground"/"Mask background", buttons "Black · Hide", "White · Reveal" (padding 16, spacing 12).

### 1.8 Status bar (`ContentView.statusBar`)

HStack spacing 16, h-padding 18, height 30, `.font(.system(size: 11).monospacedDigit())`, `.foregroundStyle(.secondary)`.

- With document: zoom percent (`.percent` 0–1 decimals, frame w 62 leading) · "`W × H px`" · "sRGB · Transparent" · Spacer · hint text.
- Without: "Ready when you are" · Spacer · hint.
- Busy: `ProgressView().controlSize(.mini)` + "Working…"; importing: "Importing images…".

Hint text per tool (exact strings):

| Tool/state | Text |
|---|---|
| marquee, Ellipse | Drag an ellipse · Shift add · Option subtract · Shift again mid-drag circle · Drag inside to move · Delete clears · ⌘D deselect |
| marquee, Rectangle | Drag a rectangle · Shift add · Option subtract · Shift again mid-drag square · Drag inside to move · ⌘-drag moves pixels · Delete clears · ⌘D deselect |
| wand, Object | Click an object to select its outline · Tab for Wand · Shift add · Option subtract · Drag inside to move · ⌘-drag moves pixels · Delete clears · ⌘D deselect |
| wand, Wand | Click to select similar colors · Tab for Object · Shift add · Option subtract · Drag inside to move · ⌘-drag moves pixels · Delete clears · ⌘D deselect |
| lasso, Freehand | Drag to select · Drag inside to move · Shift add · Option subtract · Delete clears · ⌥⌫/⌘⌫ fill · ⌘D deselect |
| lasso, Polygonal | Click corners · Click start, double-click or Enter to close · Delete removes corner · Escape cancel |
| brush | "Drag to paint" / "Drag to erase" + " · [ ] size · Shift-[ ] hardness · 1–0 opacity · Escape cancel · Space to pan" |
| blur | "Drag to soften" / "Drag to smudge" / "Drag to push pixels" + " · [ ] size · Shift-[ ] hardness · 1–0 strength · Space to pan" |
| cloneStamp | Option-click to set the source · Drag to clone · [ ] size · Shift-[ ] hardness · 1–0 opacity · Space to pan |
| spotHealing | Drag over blemishes to heal · [ ] size · Shift-[ ] hardness · Escape cancel · Space to pan |
| type | Drag a text box · Click text to edit · Drag box handles to resize · ⌘Return finish · Escape cancel |
| shape | Drag to draw a shape on a new layer · Shift {45° / square / circle} · Option from center · Shift-U or Tab for the next shape · Escape cancel · Space to pan |
| gradient | Drag to draw · Drag ends to adjust · Shift 45° · 1–0 opacity · Enter apply · Escape cancel |
| crop | Drag to crop · Enter apply · Escape cancel · Space to pan |
| move | Drag to move · Handles to resize · Circle to rotate · 1–0 layer opacity · Space to pan |
| hand | Drag to pan · Pinch to zoom |
| idle | No tool selected · Press a tool's key to pick one · Space to pan |
| zoom | Click to zoom in · Option-click to zoom out · Drag right or left to zoom smoothly · Space to pan |

### 1.9 Alerts (system NSAlert / SwiftUI .alert)

| Title | Message | Buttons | Source |
|---|---|---|---|
| Import couldn’t finish | error text | OK | ContentView |
| Couldn’t paint | error text | OK | ContentView |
| Couldn’t crop | error text | OK | ContentView |
| Save changes to {name}? | Your changes will be lost if you don’t save them. | Save, Cancel, Don’t Save | `IO/ProjectController.swift` |
| {title} (errors, e.g. "Couldn’t export PNG") | error text | OK | ProjectController |
| “{file}” was changed on disk. | Another app changed this project… | Revert, Keep Mine | `IO/ProjectController+ExternalChanges.swift` |
| This layer supplies a live mask / These layers supply live masks | Bake keeps the current masked appearance… | Bake and Delete, Cancel, Remove Links and Delete | `Document/LiveLayerMask.swift` |

Open/Save panels are system `NSOpenPanel`/`NSSavePanel` sheets.

---

## 2. Panels, dialogs and tool headers

Notation for controls: `Label` (scrub) means the label text is a horizontal drag target (`scrubbable`, cursor resizeLeftRight, value += dx × sensitivity, clamped). "↕" means Up/Down arrow keys step the field by 1 (×10 with Shift) (`arrowSteps`). Fields are `.roundedBorder` unless noted.

### 2.1 Layers panel — `LayersPanel` (`UI/LayersPanel.swift`)

VStack(alignment .leading, spacing 0), width = panel width (default 252).

1. Header HStack, padding 18: "Layers" `system 12 semibold` · Spacer · layer count `.caption.monospacedDigit()` `.tertiary`.
2. Divider.
3. `LayerAppearanceControls` (`UI/LayerAppearanceControls.swift`), VStack spacing 8, padding 12:
   - HStack: "Blend" `.caption` · `BlendModePicker` (NSPopUpButton, capsule, full width). Disabled unless `canEditAppearance`.
   - HStack spacing 6: "Opacity" `.caption` (scrub, sensitivity 1, 0…100) · `Slider` 0…1 (fills width) · HStack spacing 2 { TextField w 44 (↕) · "%" `.caption` }.
   - Blend menu items with separators between groups: Normal | Darken, Multiply, Color Burn, Linear Burn | Lighten, Screen, Color Dodge, Linear Dodge (Add) | Overlay, Soft Light, Hard Light, Vivid Light, Linear Light, Pin Light, Hard Mix | Difference, Exclusion, Subtract, Divide | Hue, Saturation, Color, Luminosity. Hovering an item previews it on the canvas.
4. Divider.
5. `NativeLayerList` (fills), or empty state: VStack spacing 10, padding 16, `.secondary`: `square.3.layers.3d` 25pt light · "No layers yet" `.callout.weight(.medium)` · "Create a canvas or import an image." / "Import an image or add a blank layer." `.caption` centered.
6. Divider.
7. Footer HStack(spacing 0), `.buttonStyle(.plain)`, `.secondary`, padding h8 v4; each icon has `footerHitArea` (padding h8 v12). Symbols at default body size (13pt):

| Order | Symbol | Action / help |
|---|---|---|
| 1 | `plus.square` | New blank layer (⇧⌘N) |
| 2 | `folder.badge.plus` | Group selected layers (⌘G) |
| 3 | `rectangle.inset.filled` (`LayerMaskMenu`, `.borderless`) | Add layer mask (Option-click for a black mask) / …revealing the selection (Option-click to hide it) |
| 4 | `sparkles` Menu (borderless, fixedSize) | Layer effects: stroke and drop shadow — items: Stroke…, Drop Shadow…, Color Overlay…, Inner Shadow…, Outer Glow…, Inner Glow… |
| 5 | `circle.lefthalf.filled` Menu | New adjustment layer — items (no ellipses): Hue/Saturation, Levels, Curves, Exposure, Gradient Map, Grain, Add Noise, Gaussian Blur, Motion Blur, Invert, Black & White, Color Balance |
| — | Spacer | |
| 6 | `trash` | Delete selected effect / layer mask / layers / layer |

Note: `Menu` with `.borderlessButton` style draws a small disclosure chevron next to the icon **(system)**.

### 2.2 `NativeLayerList` row layout (`UI/NativeLayerList.swift`)

AppKit `NSTableView` (`LayerTableView`) in an `NSScrollView` (vertical scroller, no background). `style = .plain`, no header, clear background, `rowHeight 52`, `intercellSpacing (0, 2)`, multi-select, one column (initial width 252, autoresizes). Selected rows use the **system table selection highlight** (accent when the table is first responder in the key window, gray otherwise).

Row height = `52 + 24 × effectCount`.

`LayerCell` (NSTableCellView) constraints, y measured from the row top (flipped); all vertical centers at y = 26:

| Element | Frame / style |
|---|---|
| Eye (`EyeSwipeButton`, borderless) | leading 8, 20 × 32; `eye` / `eye.slash`; drag across rows sets visibility |
| Indentation | disclosure.leading = eye.trailing + `min(depth, 8) × 24 + (clipped ? 24 : 0)` |
| Disclosure | 16 × 24, only for folders; `chevron.down` (expanded) / `chevron.right` (collapsed) |
| Thumbnail slot | leading = disclosure.trailing − 2, width 36, top 0…52 |
| Layer thumbnail (`LayerThumbnailButton`) | centered in slot; pixel layers: canvas aspect fitted in a 36 box (e.g. 36×20 for 16:9); adjustment/folder/text: 36×36 icon; `cornerRadius 3`; border 2pt `controlAccentColor` when this row is the single active target and the layer (not mask) is targeted |
| Mask slot | leading = thumbSlot.trailing + gap (5, or 13 when a link icon shows), width 30 if mask else 0 |
| Mask thumbnail | canvas aspect fitted in a 30 box, radius 3; 2pt accent border when mask targeted; white border when shown alone |
| Link button | 9 × 20, centerX = maskSlot.leading − 6.5; `link` symbol 10pt medium rotated 45° (template, `secondaryLabelColor`); empty when unlinked; hidden for folders/adjustments |
| Disabled-mask mark | "╱" `systemFont 32 medium`, `systemRed`, centered on mask thumbnail |
| Name | leading = maskSlot.trailing + 5, trailing −8, top 9; `systemFont 13`, single line, truncating tail; prefix "↳ " for clipped layers; becomes an editable rounded-bezel field while renaming |
| Detail line | below name +3, `systemFont 10`, `secondaryLabelColor`: `"W × H px"` (+ " · N%" when scaled), "Text · Double-click to edit", "Adjustment · Double-click to edit", "Folder", or "Clipped to {source}" |
| Row edge line | bottom, full width, 1 device pixel, white 6% |
| Hidden-by-parent row | whole cell alpha 0.35 |
| Effect sub-row (`LayerEffectRow`) | height 24 each, stacked from y=52; eye 20 × 22 at leading 38 + indent (same indent as the row), `secondaryLabelColor`; label `systemFont 11` at eye.trailing + 8 (labelColor if enabled, secondary if not); selected bg `controlAccentColor` 30% |

Thumbnail images (`UI/CanvasThumbnail.swift`), rendered at 2× backing scale: canvas-shaped checkerboard, base gray 0.22, squares gray 0.32, 6pt tiles, layer pixels placed by its transform. Mask thumbnails: fill = mask background (white or black) then mask pixels. Icons: adjustment `AdjustmentKind.symbol` drawn at 1.21× symbol size into 36×36 (Curves rotated 90° clockwise); folder = `folder` at 80% of 36; editable text = `textformat`.

Adjustment layer symbols: Hue/Saturation `circle.lefthalf.filled`, Levels `slider.horizontal.3`, Curves `point.topleft.down.to.point.bottomright.curvepath`, Exposure `plusminus.circle`, Gradient Map `paintpalette`, Grain `circle.grid.3x3`, Add Noise `circle.dotted`, Gaussian Blur `drop.fill`, Motion Blur `wind`, Invert `circle.righthalf.filled`, Black & White `circle.filled.pattern.diagonalline.rectangle`, Color Balance `scale.3d`.

Custom cursors in the list (not needed for PNG parity): clipping cursor (`arrow.turn.down.right` + `rectangle.badge.plus`/`rectangle.badge.minus`), show-mask cursor (duplicate cursor + `eye.fill`).

Tooltips: thumbnail "Select image pixels" / "Editable text layer"; mask "Select layer mask; Option-click to view it alone; Shift-click to enable/disable; Cmd-click to select its black areas (Cmd-Shift adds, Cmd-Option subtracts)"; link "Unlink layer and mask to move or transform them separately" / "Link layer and mask so they move together"; effect row "Click to select; double-click to edit; Option-drag to copy {kind}".

### 2.3 Tool headers (options bar)

All use `.padding(.horizontal, 18).toolHeaderBar()` (height 42, 12pt controls) unless noted, followed by a Divider. Segmented pickers are `.pickerStyle(.segmented).labelsHidden().fixedSize()`.

#### 2.3.1 Move — `TransformInspector` (`UI/TransformInspector.swift`)
HStack spacing 12, trailing pad 18:
1. Title "Transform" / "Transform Mask" (leading pad 18).
2. Toggle (checkbox) "Auto Select" — default off (persisted `tool.autoSelect`); shows inverted while ⌘ held.
3. Toggle "Show Controls" — default on (`tool.transformControls`).
4. Horizontal ScrollView (hidden indicators), HStack spacing 12, h-pad 18, disabled when nothing to transform or while distorting:
   - "X" (scrub, caption, secondary) + field, w 85; range −30000…30000
   - "Y" same
   - "W" + field w 85, 1…30000
   - "H" + field w 85, 1…30000
   - Toggle `.button` style with `link` symbol — lock aspect ratio, default on; Shift inverts while held
   - "Scale" + field + "%" suffix, w 110, 0.1…30000
   - "°" + field, w 75, −360…360
   - Picker "Sampling" (menu, labeled), w 170: Nearest, Smooth, High quality
   - Button "Flip H", Button "Flip V"
5. HStack spacing 12: Button "Cancel" (Esc), Button "Apply" (Return) — opacity 0 unless a persistent (⌘T/distort) edit is pending; fades 0.12s.

Field format: integer if whole, else 2 decimals.

#### 2.3.2 Brush / Eraser / Spot Healing / Clone Stamp / Smear — `BrushControls` (`UI/BrushControls.swift`)
HStack spacing 12. Title: "Spot Healing", "Clone Stamp", "Smear", "Eraser" or "Brush".

| # | Control | Shown for | Details / defaults |
|---|---|---|---|
| 1 | Segmented Mode | brush | Paint, Erase (default Paint) |
| 1 | Segmented Mode | blur (Smear) | Liquify, Blur, Smudge (default Liquify) |
| 1 | Segmented Type | spotHealing | Content-Aware, Create Texture, Proximity Match (default Content-Aware) |
| 1 | Toggle "Aligned" (checkbox) + Segmented Sample: This Layer / All Layers | cloneStamp | Aligned on; This Layer |
| 2 | "Size" (scrub 1, 1…2000) + field w 48 (0 dp) + "px" | all | 40 |
| 3 | "Hardness" (scrub 0.01) + Slider w 100 (0…1) + field w 42 + "%" | all | 100% (Clone Stamp and Smear keep separate tips starting 40 px / 0% / 100%) |
| 4 | "Opacity" or "Strength" (blur) + Slider w 100 (0.01…1) + field w 42 + "%" | all | 100% |
| 5 | "Radius" (scrub 0.1) + Slider w 100 (0.5…20) + field w 42 (0–1 dp, 0.5…50) + "px" | blur in Blur mode | 5 |
| 6 | "Smoothing" (scrub 1) + Slider w 100 (0…100) + field w 42 | brush | 0 |
| 7a | Picker "Paint" (menu, labeled) w 180: Black · Hide / White · Reveal | when a mask is targeted | Black · Hide |
| 7b | "Color" + swatch button 34×18, RoundedRectangle r=4 continuous, inner white 1pt inset 1, outer black 1pt | brush, spotHealing (not clone/blur) | foreground color |
| 8 | Spacer(minLength 0) | | |
| 9 | "Option-click to set the source" `.secondary` | cloneStamp without source | |
| 10 | "Mask" `.secondary` | mask targeted | |

#### 2.3.3 Marquee / Lasso / Magic — `LassoControls` (`UI/LassoControls.swift`)
HStack spacing 12. Title "Marquee" / "Lasso" / "Magic".

| # | Control | Shown for | Values / defaults |
|---|---|---|---|
| 1 | Segmented shape | marquee | Rectangle, Ellipse (default Rectangle) |
| 1 | Segmented mode | wand | Wand, Object (default Wand) |
| 1 | Segmented lasso | lasso | Freehand, Polygonal (default Freehand) |
| 2 | Segmented selection mode | all | New, Add, Subtract (default New; live-reflects held Shift/Option) |
| 3 | Wand controls: HStack spacing 12 { "Tolerance" (scrub) + field w 44 right-aligned (0…255, default 32) · Picker (menu, unlabeled): Point Sample, 3 by 3 Average, 5 by 5 Average · Segmented This Layer / All Layers (default This Layer) · Toggle "Contiguous" (on) } | wand/Wand | |
| 3 | Object controls: Segmented This Layer / All Layers (default **All Layers**) · "Edge" (scrub) + field w 40 (−10…10, default 0) + "px" | wand/Object | |
| 4 | Toggle "Anti-alias" (on) | lasso, wand, ellipse marquee | |
| 5 | `Divider().frame(height: 18)` | all | |
| 6 | Button "Expand" + field w 40 (1…500, default 1) + "px" (unit scrubbable) | all; disabled without selection | |
| 7 | Button "Contract" + field w 40 (1…500, default 1) + "px" | | |
| 8 | Button "Feather" + field w 48 (1…250, default 2) + "px"; HStack spacing 5 | | |
| 9 | Spacer | | |
| 10 | "Empty selection" (secondary) if empty; Button "Deselect" | when a selection exists | |

#### 2.3.4 Gradient — `GradientControls` (`UI/GradientControls.swift`)
HStack spacing 12: "Gradient" · Segmented Linear/Radial (Linear) · preview swatch 56×18 (4pt checkerboard gray 45% over white, linear gradient of current colors, r=3, black 50% border) · Picker (menu, unlabeled): Foreground to Background, Foreground to Transparent (default **Foreground to Transparent**) · Toggle "Reverse" (off) · "Opacity" (scrub) + Slider w 100 + field w 42 + "%" (100) · Spacer · "Mask" (if mask) · Buttons "Cancel", "Apply" (only while a gradient edit is pending).

#### 2.3.5 Type — `TypeControls` (`UI/TypeControls.swift`)
HStack spacing 12: "Type" · horizontal ScrollView (hidden indicators) HStack spacing 10:
1. Font pop-up (`TypeFontPicker`, AppKit NSPopUpButton, capsule, width 210, truncating tail; menu lists all installed fonts each set in its own face; "(Multiple)" item for mixed selections). Default "Helvetica".
2. Size field w 52 + "px" (scrub 1…2000), default 72.
3. Color swatch 36×18, r=3, black 50% border. Default black.
4. Alignment: 3 plain buttons 30×26, selected bg white 14% r=4; symbols `text.alignleft`, `text.aligncenter`, `text.alignright`. Default Left.
5. "Tracking" (scrub −100…1000) + field w 45, default 0.
6. "Leading" (scrub 0…5000) + field w 52 with placeholder "Auto" (0 = Auto = 120% of size).
Then outside the scroll view: Buttons "Cancel" + "Done" while editing text, else "Edit Text" (disabled unless the active layer is live text).

#### 2.3.6 Shape — `ShapeControls` (`UI/ShapeControls.swift`)
HStack spacing 12: "Shape" · Segmented Rectangle/Ellipse/Line (Rectangle) · for Line: HStack spacing 6 { "Width" (scrub 1…5000) + Slider w 100 (1…100) + field w 48 right-aligned + "px" } default 4 · for Rectangle: { "Radius" (scrub 0…5000) + Slider w 100 (0…200) + field w 48 + "px" } default 0 · HStack spacing 6 { "Fill" + swatch 36×18 r=3 black 50% border (foreground color) } · Spacer.

#### 2.3.7 Crop — `CropControls` (`UI/CropControls.swift`)
HStack spacing 14: "Crop" · Picker "Ratio" (menu, labeled) w 170: Free, Original, 1:1, 4:3, 3:4, 16:9, 9:16 (default Free) · "`W × H px`" monospaced digits (when a crop rect exists; defaults to the whole canvas or the selection bounds) · Spacer · Button "Cancel" · Button "Apply Crop".

#### 2.3.8 Hand / Zoom — `NavigationToolHeader` (`UI/NavigationToolHeader.swift`)
HStack spacing 12: "Pan" or "Zoom" · (zoom only) TextField w 72, right-aligned, shows current zoom % (≤2 decimals, trailing zeros trimmed) + "%" (scrubbable 0.1…3200) · Spacer.

#### 2.3.9 Eyedropper (inline in `ContentView`)
HStack spacing 16: "Eyedropper" · Toggle "Sample Ring" `.checkbox` (default on) · Spacer.

#### 2.3.10 No tool (inline)
HStack spacing 16: "Select a tool" · Spacer.

### 2.4 Floating panels (non-modal)

#### 2.4.1 Levels — `LevelsSheet` (`UI/LevelsSheet.swift`) — width 440, padding 24, spacing 16
1. Picker "Channel" (menu, labeled) w 180: RGB, Red, Green, Blue.
2. Histogram `Canvas` h 150, bg black 25%; bar color gray/red/green/blue per channel; "Loading histogram…" `.caption` overlay until ready. Below: input handles strip h 20 — three `triangle.fill` 12pt (black, gray, white) 22×20, gray 0.5 shadow.
3. HStack: field groups "Input black" (0 dp), "Gamma" (2 dp, 0.1…9.99), "Input white" — each VStack spacing 5 { caption secondary label (scrub) · field w 80 right-aligned }, Spacers between.
4. Output gradient bar black→white h 14 + handles strip h 20 (two triangles).
5. HStack: "Output black", Spacer, "Output white".
6. HStack: "Sample" caption secondary · 3 buttons `Label(“Black|Gray|White”, systemImage: "eyedropper")` tinted accent when armed.
7. (armed) "Click the original layer to set {mode}. Click the eyedropper again to stop." caption.
8. VStack spacing 6: "Auto" caption · Buttons "Contrast", "Color", "Color + neutral midtones" (disabled until histogram ready).
9. HStack: Toggle "Preview" (⌥P) · Spacer · Button "Reset".
10. Caption: "Original pixels · alpha-weighted histogram" (or "…selection and alpha-weighted histogram", or "Underlying pixels · …" for adjustment layers).
11. Divider; HStack: "Cancel" (Esc) · Spacer · (ProgressView small while committing) · "OK" `.borderedProminent` (Return).
Defaults: black 0, gamma 1.00, white 255, output 0/255.

#### 2.4.2 Hue/Saturation — `HueSaturationSheet` (`UI/HueSaturationSheet.swift`) — width 460, padding 24, spacing 16
1. HStack spacing 12: Picker range (menu, unlabeled) w 160: Master, Reds, Yellows, Greens, Cyans, Blues, Magentas (disabled when Colorize) · Spacer · sampling controls: (non-Master, non-Colorize) three plain buttons 24×20 `eyedropper` with badges `plus.circle.fill` / `minus.circle.fill` 8pt semibold (offset 3,1), selected bg accent 25% r=4, then `Divider` h16; (non-Colorize) targeted-adjustment button `hand.point.up.left` 24×20.
2. Slider rows (HStack spacing 10): title w 76 (scrub; double-click resets) · `CameraRawSlider` (colored track) · field w 48 right-aligned + unit:
   - Hue: −180…180 (Colorize 0…360), unit "°", track `.spectrum`
   - Saturation: −100…100 (Colorize 0…100), track `.chroma` (Master) / `.saturation(hue)`
   - Lightness: −100…100, track black→white
3. (range ≠ Master, not Colorize) `SpectrumEditor`: two spectrum bars h 16 r=3 (72 slices), handle strip h 12 between (outer marks 7×5, inner bars 2pt), readout of 4 handle degrees `.caption.monospacedDigit()` secondary; Toggle "Apply outside this range instead".
4. HStack spacing 18: Toggle "Colorize" (on → hue 0, sat 25) · Toggle "Preview" · Button "Reset" · Spacer.
5. "Limited to the selection" `.callout` secondary (when a selection exists).
6. Divider; Cancel / Spacer / OK `.borderedProminent`.

#### 2.4.3 Color Range — `ColorRangeSheet` (`UI/ColorRangeSheet.swift`) — width 340, padding 24, spacing 16
1. HStack spacing 6: three eyedropper buttons (Sample, Add, Remove; badges as above), bg accent 25% for the effective mode · Spacer.
2. Preview: black box with the black/white selection image, canvas aspect fitted in 292×200, white 20% border.
3. "Click the image to pick the color to select." / "Shift-click adds a color, Option-click takes one away." `.callout` secondary.
4. HStack spacing 10: "Fuzziness" (scrub, 0…200) · Slider · field w 48 right-aligned (default 40).
5. Toggle "Invert".
6. (error text orange).
7. Divider; Cancel / Spacer / OK `.borderedProminent`.

#### 2.4.4 Selection amount — `SelectionAmountSheet` (`UI/LassoControls.swift`) — width 380, padding 24, spacing 16
HStack spacing 10: "Amount" minWidth 60 (scrub) · Slider 1…500 (Feather 1…250) step 1 · TextField w 56 right-aligned + "px" (focused on appear). Validation text "Enter a whole number from 1 to {max} px." `.callout` secondary (opacity 0 when valid). Divider. HStack: "Cancel" · Spacer · "OK" `.borderedProminent`. Starting value = last used amount (1, 1, 2).

#### 2.4.5 Layer effects — `EffectsSheet` (`UI/EffectsSheet.swift`) — width 340, padding 20, spacing 16
Slider row = HStack spacing 10 { title w 64 (scrub) · Slider w 130 · field w 48 right-aligned (0 dp) + unit }. Swatch = 36×18 r=3 continuous, inner white 1pt, outer black 1pt. Footer HStack spacing 10: Spacer · "Cancel" · "OK" (plain bordered, not prominent).

| Kind (title, `.headline`) | Header right | Rows (slider range; typed range) | Defaults |
|---|---|---|---|
| Stroke | Segmented Outside/Inside | "Color" (w 64) + swatch; Size 0…20 (0…500) px; Opacity 0…100 % | size 4, color = background color, opacity 100, Outside |
| Drop Shadow | swatch | Opacity 0…100 %; Angle −180…180 °; Distance 0…100 (0…5000) px; Blur 0…100 (0…500) px | 50%, 90°, 20, 20, black |
| Color Overlay | swatch | Opacity 0…100 % | 100%, background color |
| Inner Shadow | swatch | Opacity; Angle; Distance 0…50 (0…5000); Blur 0…100 (0…500) | 50%, 90°, 10, 10, black |
| Outer Glow | swatch | Size 0…100 (0…500) px; Opacity % | 20, 75%, white |
| Inner Glow | swatch | Size; Opacity | 10, 75%, white |

#### 2.4.6 Filters and image adjustments — `FilterSheet` (`UI/FilterSheet.swift`) — width 380 (Camera Raw 440 × window height), padding 24, spacing 16
Generic row `control(title, …)`: HStack spacing 10 { title (width = widest title in the panel, min 60; scrub; double-click resets when colored) · `Slider` (log scale when marked) or `CameraRawSlider` when a colored track is given · field w 56 right-aligned + unit }.
Common footer: Toggle "Preview" · (orange preview error) · "Limited to the selection" `.callout` secondary · Divider · HStack { "Cancel" · Spacer · [ProgressView small + "Applying…"/"Working…" callout secondary] · "OK" `.borderedProminent` }.

| Filter (title) | Menu | Controls in order (range, unit, decimals, log?) | Defaults |
|---|---|---|---|
| Gaussian Blur | Filter | Radius 0.1…250 px 1dp log | 1 |
| Motion Blur | Filter | Angle −90…90 °; Distance 1…2000 px log | 0, 10 |
| Add Noise | Filter | Amount 0.1…400 % 1dp log; "Distribution" + Segmented Uniform/Gaussian; Toggle "Monochromatic" | 10, Uniform, off |
| Vignette | Filter | "Color" (w 95) + swatch 24×24 r=6 (white 1.5 inner, black 1 outer); Amount 0…100 %; Midpoint 0…100 %; Roundness −100…100; Feather 0…100 %; Highlights 0…100 % | black, 35, 50, 100, 60, 25 |
| Bloom / Glow | Filter | Amount 0…100 %; Radius 1…150 px log | 40, 24 |
| Dither | Filter | see below | |
| Tonal Contrast | Filter | Amount 0…100 %; Shadows −100…100 %; Midtones; Highlights; Radius 1…100 px log | 50, 40, 60, 30, 16 |
| Lens Correction | Filter | Remove Distortion −100…100; caption "Positive straightens lines that bow outward (barrel); negative, lines that bow inward (pincushion)." | 0 |
| Camera Raw Filter | Filter (docked right) | `CameraRawControls` §2.4.7 | |
| Remove Background | Filter | text "Hide the background behind a layer mask…"; Segmented Basic/Advanced; (Advanced) Refine 0…40 px, Contrast 0…100 %, Shift Edge −10…10 px | Basic, 12, 25, 0 |
| Content-Aware Fill | Edit menu | text "Fill the selection using surrounding pixels from this layer." | |
| Curves | Image | `CurvesControls` (below) | identity |
| Exposure | Image | Exposure −20…20 2dp; Offset −0.5…0.5 4dp; Gamma 0.01…9.99 2dp log | 0, 0, 1 |
| Gradient Map | Image | `GradientMapControls`: gradient bar h 20 r=4 (black 35% border); HStack spacing 20 { swatch 24×24 + "Shadows", swatch + "Highlights" }; Toggle "Reverse" | black, white, off |
| Grain | Image | Amount 0…100; Size 0.5…20 px 1dp log; Roughness 0…100 | 25, 1.5, 50 |
| Black & White | Image | Reds, Yellows, Greens, Cyans, Blues, Magentas (−200…300 %, luminance tracks at 0/60/120/180/240/300°); Toggle "Tint"; (Tint) Hue 0…360 ° plain track, Saturation 0…100 % | 40, 60, 40, 60, 20, 80; tint off, 40°, 20% |
| Color Balance | Image | "Shadows"/"Midtones"/"Highlights" `.headline` each followed by Cyan / Red, Magenta / Green, Yellow / Blue (−100…100, opposing tracks); Toggle "Preserve Luminosity" | 0s |

Opposing track colors (sRGB): Cyan/Red 0.10,0.72,0.80 → 0.86,0.18,0.20; Magenta/Green 0.80,0.22,0.70 → 0.24,0.70,0.30; Yellow/Blue 0.95,0.82,0.18 → 0.22,0.40,0.92.

**Dither controls** (in order, conditional): Picker "Style" (menu, labeled) grouped with dividers: [Atkinson (Classic Mac), Floyd–Steinberg] | [Bayer 2 × 2, Bayer 4 × 4, Bayer 8 × 8] | [Halftone Dots, Halftone Lines, Halftone Diamonds] | [Mac Patterns, ASCII, Scanlines (CRT)] (default Atkinson) · Pixel Size 1…32 px (not ASCII/Scanlines; default 2) · Text Size 6…64 px (ASCII; 14) · Line Spacing 2…32 px, Glow 0…100 % (35), Dots 0…100 % (0), Wobble 0…64 px (0) (Scanlines) · Cell Size 4…64 px (8) and Angle −90…90 ° (45) (halftones) · "Characters" + monospaced field (ASCII; default " .:-=+*#%@") · Tones 2…8 (diffusion/Bayer; 2) · Diffusion 0…100 % (diffusion; 100) · Density −100…100 (0) · Contrast −100…100 (0) · Picker "Colors" (menu, fixedSize): Black & White, Two Colors, Original · (Two Colors) "Dark" swatch 24×24, "Light" swatch · (pixelSize > 1) Picker "Pixel Shape": Square, Dot · (halftone/patterns/ASCII) Toggle "Light on Dark" (on).

**CurvesControls** (`UI/CurvesControls.swift`), VStack spacing 12: Picker "Channel" (RGB, Red, Green, Blue) · Canvas h 260, bg black 35%, 4×4 grid white 12%, curve white 2pt, points 8pt circles (selected accent) · "Click to add a point. Drag to adjust." caption secondary · HStack { "Input N · Output N" monospaced (when selected) · Spacer · Button "Remove point" } · Button "Reset curve". Max 32 points.

#### 2.4.7 Camera Raw Filter — `CameraRawControls` and friends (`UI/CameraRaw*.swift`)
Docked panel 440 wide, full window height. VStack spacing 10:
1. Scope box h 110, bg black 35%, r=4: RGB histogram (three filled ribbons at 55% opacity) or vectorscope (context menu: Histogram / Vectorscope). Corner buttons: `triangle.fill` `.caption2` (left = shadow clipping, blue when on; right = highlight clipping, red when on; white 55% off).
2. Readout "R —   G —   B —" / "R n   G n   B n" `.caption.monospacedDigit()` secondary.
3. ScrollView, VStack spacing 12 of sections. Section header: plain button { `chevron.down`/`chevron.right` caption semibold w 12 + title `.headline` } + Spacer + (when the section changes anything) eye button `eye`/`eye.slash` borderless. Section content indented 18. Default expanded: Light, Color, Color Grading.

Slider row (Light/Color/Effects/Detail/Optics/Geometry/Calibration): HStack spacing 10 { title minWidth 96 (scrub; double-click reset) · `CameraRawSlider` (NSSlider, height 22) · field w 56 right-aligned }.

| Section | Controls in order (range; default; reset if not 0) |
|---|---|
| Light | Exposure −5…5 (2dp); Contrast, Highlights, Shadows, Whites, Blacks −100…100 |
| Color | "White Balance" + Picker (Custom, Auto) + `eyedropper` button (borderless, accent when armed); caption when armed; Temperature (track blue→yellow), Tint (green→mauve), Vibrance (chroma), Saturation (chroma), all −100…100 |
| Color Grading | Picker (menu, fixedSize): Three-Way, Shadows, Midtones, Highlights, Global; Three-Way = HStack spacing 30 of 3 wheels (caption title, `GradeWheel` 86×86: angular hue gradient 85% opacity, white 80% ring, white 10pt dot; readout "H°  S" `.caption2` mono; luminance slider w 96); Blending 0…100 (reset 50), Balance −100…100 — title w 78 |
| Effects | Texture, Clarity, Dehaze (−100…100); subheadline "Glow"; Glow 0…100; Picker "Style": Diffusion, Bloom, Halation; indented 16: Range, Spread, Warmth; subheadline "Vignette"; Amount −100…100; Picker "Style": Highlight Priority, Color Priority, Paint Overlay; indented 16: Midpoint 0…100 (50), Roundness −100…100, Feather 0…100 (50), Highlights 0…100; subheadline "Grain": Amount 0…100, Size 0…100 (reset 25), Roughness 0…100 (reset 50) |
| Curve | Segmented Parametric/Point; (Point) Segmented RGB/Red/Green/Blue; graph h 150 (black 35%, diagonal white 25%, curve white 1.5, parametric split markers white 3pt, points 8pt); Parametric: Highlights, Lights, Darks, Shadows −100…100 (title w 88, field w 48); Point: "In n   Out n" caption, Picker "Preset": Custom, Linear, Medium Contrast, Strong Contrast, (RGB) Refine Saturation −100…100; bordered button `Label("Targeted Adjustment", systemImage: "scope")` |
| Color Mixer | Segmented HSL / Color / Point Color. HSL: Segmented Hue/Saturation/Luminance + 8 rows Reds, Oranges, Yellows, Greens, Aquas, Blues, Purples, Magentas (title w 78, −100…100, hue/sat/lum tracks at 0,30,60,120,180,240,270,300°). Color: 8 circle swatches 18pt (selected white 2pt ring) + Hue/Saturation/Luminance sliders (title w 88, no field). Point Color: `eyedropper` + up to 8 picked circles 16pt; Hue Shift, Saturation Shift, Luminance Shift (−100…100), Hue Range 5…180 (30), Saturation Range 0.05…1 (0.4), Luminance Range 0.05…1 (0.4) (title w 110, no field); Toggle "Visualize Range". Then "Targeted Adjustment" `scope` button |
| Detail | subheadline "Sharpening": Amount 0…150, Radius 0…100 (10), Detail (25), Masking (0); subheadline "Noise Reduction": Luminance; Luminance Detail (50), Luminance Contrast (0) (45% opacity/disabled until Luminance > 0); Color; Color Detail (50), Color Smoothness (50) (dimmed until Color > 0) |
| Optics | Toggle "Remove Chromatic Aberration"; Toggle "Enable Lens Profile Corrections"; (on) caption + Distortion 0…100 (100), Vignetting 0…100 (100); subheadline "Manual": Distortion −100…100; "Defringe" + `eyedropper`; Purple Amount 0…100; "Purple Hue" caption with "Low"/"High" caption2 + two sliders 0…360 (270, 310); Green Amount; "Green Hue" (60, 120); Vignetting −100…100; Midpoint 0…100 (50) |
| Geometry | subheadline "Upright"; Segmented Off/Guided; (Guided) button `Label("Draw Guides", systemImage: "line.diagonal")`, Button "Clear Guides"; Picker "Projection": Perspective, Rectilinear; Vertical, Horizontal (−100…100), Rotate −45…45, Aspect, Scale, Offset X, Offset Y; Toggle "Constrain Crop" |
| Calibration | Picker "Process": Version 1…Version 6 (default 6); caption summary; subheadline "Shadows": Tint; "Red Primary": Hue, Saturation; "Green Primary": Hue, Saturation; "Blue Primary": Hue, Saturation (all −100…100) |

`CameraRawSlider` gradient tracks (`GradientSliderCell`): 4pt rounded bar with the track colors replacing the system bar. Temperature 0.22,0.46,0.95 → 0.98,0.82,0.18; Tint 0.28,0.70,0.34 → 0.70,0.40,0.64; Chroma 0.62,0.62,0.64 → 0.86,0.18,0.20; `.hue(h)` HSB(h−50,0.85,0.9)→(h+50); `.saturation(h)` gray 0.55 → HSB(h,0.9,0.9); `.luminance(h)` HSB(h,0.55,0.18) → HSB(h,0.35,0.95); `.spectrum(h)` 13 stops h−180…h+180 step 30 at S 0.85 B 0.9.

#### 2.4.8 Color Picker — `ColorPickerSheet` (`UI/ColorPickerSheet.swift`) — fixedSize, padding 20
HStack(alignment .top, spacing 14):
1. Saturation/brightness field 256×256: white→pure hue horizontal gradient, clear→black vertical; marker ring 12pt white 1.5 + black 0.75; black 60% border.
2. Hue strip 20×256 (hue 360→0 top to bottom), black 60% border, h-pad 7; two 7×10 triangle arrows (`.primary`) either side at the current hue. Total width 34.
3. Column 180×256: HStack spacing 16 { preview 64×64 r=5 continuous, black 60% border · VStack spacing 8 of "OK" (Return) and "Cancel" (Esc), `.controlSize(.large)`, width 90 }; Spacer(min 12); Grid (h 8, v 6): rows "R", "G", "B" (label w 14 scrub + field w 52 ↕, 0…255) and "#" + hex field w 84 monospaced; "Click the canvas to sample" caption secondary (not for dialog targets).

Panel titles: "Color Picker (Foreground Color)", "(Background Color)", "(Text Color)", "({effect} Color)", "(Gradient Map Shadows|Highlights)", "(Vignette Color)", "(Dither Dark|Light Color)", "({dialog title})" e.g. "Color Picker (JPEG Background)".

#### 2.4.9 Keyboard Shortcuts — `KeyboardShortcutsSheet` (`UI/KeyboardShortcuts.swift`) — width 660, padding 24, spacing 10
Secondary intro text "Click a shortcut, then press its new key combination. Changes apply when you save." · TextField "Search shortcuts" · ScrollView h 465 { LazyVStack spacing 6: for groups "Menus", "Canvas & Layers", "Text Editing": `.headline` (top pad 8) then rows HStack { title · Spacer · `ShortcutRecorder` NSButton rounded 150×26 showing the chord label or "Press keys…" }; Divider; headline "Contextual keys & mouse gestures" + two paragraphs } · conflict text (orange `.callout`, h 22) · Divider · HStack { "Restore Defaults" · Spacer · "Cancel" · "Save" `.borderedProminent` }.

### 2.5 Window sheets and welcome form

Sheets from `IO/ProjectController.swift` are `NSWindow` with `styleMask [.titled, .fullSizeContentView]` and `NSHostingController` content, presented with `window.beginSheet`. Canvas Size, Image Size, Trim, Grid use `.textFieldStyle(.roundedBorder)` and `.roundedControls()`.

#### 2.5.1 Welcome / New canvas — `NewCanvasSheet` (`UI/NewCanvasSheet.swift`) — padding 28, maxWidth 500, spacing 24 (shown centered on the empty canvas)
1. HStack: "New canvas" `.title2.weight(.semibold)` · Spacer · presets Menu drawn as three 2.5pt dots stacked (spacing 2.5) in a 28×28 frame, menu indicator hidden. Menu = inline picker: Custom | 4K 3840×2160, 1440p 2560×1440, 1080p 1920×1080 | iPhone 18 Pro 1206×2622, iPhone 18 Pro Max 1320×2868, MacBook Pro 14" 3024×1964, MacBook Pro 16" 3456×2234, Studio Display 5120×2880 | Instagram Square 1080×1080, Instagram Portrait 1080×1350, Instagram Story 1080×1920, YouTube Thumb 1080×608.
2. HStack spacing 16: "Width" block · `multiply` symbol `.tertiary` (top pad 20) · "Height" block. Block = VStack spacing 8 { label `.callout.weight(.medium)` · HStack { plain TextField · "px" secondary } padding 12, bg `.quaternary` 50% r=7 }. Defaults 1920 × 1080 (or clipboard image size on first appear unless `skipsInitialClipboardCanvasSize`). Width field focused on appear.
3. "Transparent canvas · sRGB" `.callout` secondary (or orange "Enter whole numbers from 1 to 30,000 pixels.").
4. HStack spacing 10: "Open project" `.bordered` · "Import image" `.bordered` · Spacer · "Create canvas" `.borderedProminent` (Return).

#### 2.5.2 Canvas Size — `CanvasSizeSheet` (`UI/CanvasSizeSheet.swift`) — width 450, padding 24, spacing 16
"Canvas Size" `.title2.bold()` · "Current: W × H pixels" · "{bytes} uncompressed RGBA canvas" `.callout` secondary · Divider · Picker "Units" (Pixels, Percent, Inches, Centimeters) · "Width" w 60 (scrub) + field (0–3 dp) · "Height" · Toggle "Relative to current dimensions" (off) · Toggle "Lock original aspect ratio" (off) · "New: W × H pixels · {bytes} uncompressed" callout secondary / orange error · HStack(top, spacing 24) { VStack spacing 8 { "Anchor" · 3×3 Grid spacing 3 of buttons 25×25 `circle.fill` (selected, accent tint) / `circle` (secondary) } · VStack spacing 8 top pad 28 { anchor name `.callout.bold()` (default "Center") · "Keeps this point fixed. Artwork is not scaled; cropped content remains outside the canvas." callout secondary } } · Picker "Canvas extension": Transparent, Foreground, Background, Black, White, Custom (default Transparent) · (Custom) "Extension color" + `DialogColorSwatch` 34×18 · HStack { "Cancel" · Spacer · "OK" }.

#### 2.5.3 Image Size — `ImageSizeSheet` (`UI/ImageSizeSheet.swift`) — width 430, padding 24, spacing 18
"Image Size" title2 bold · "Current: W × H pixels" secondary · Picker "Units" (Pixels, Percent, Inches, Centimeters; Resample off hides Pixels/Percent) · "Width" w 75 + field · "Height" · Toggle "Lock aspect ratio" (on) · HStack { "Resolution" (scrub 1…9600) · field · "pixels/inch" secondary } (default document resolution, 72) · Toggle "Resample" (on) · (on) Picker "Sampling": Nearest, Smooth, High quality (default High quality) + "Resizes layer pixels and applies existing transforms. Undo restores the originals." / (off) "Only print dimensions and resolution change. Pixels stay unchanged." · "Result: W × H pixels" (or orange limits text) · HStack { "Cancel" · Spacer · "Resize" }.

#### 2.5.4 Trim — `TrimSheet` (`UI/TrimSheet.swift`) — width 320, padding 24, spacing 18
"Trim" title2 bold · VStack spacing 8 { "Based On" `.headline` · radio group: Transparent Pixels (default), Top Left Pixel Color, Bottom Right Pixel Color } · Divider · { "Trim Away" headline · Grid (h 24, v 8): Top/Bottom, Left/Right checkboxes, all on } · Divider · { "Cancel" · Spacer · "OK" `.borderedProminent` }.

#### 2.5.5 Grid Settings — `GridSettingsSheet` (`UI/GridSettingsSheet.swift`) — width 360, padding 24, spacing 18
"Grid" title2 bold · "Color" w 110 + Picker (Light Gray, Light Blue, Light Red, Green, Medium Blue, Yellow, Magenta, Cyan, Black, Custom; default Light Gray) + `DialogColorSwatch` · "Style" w 110 + Picker (Lines, Dashed Lines, Dots) · "Opacity" w 110 (scrub 0.5) + Slider 1…100 + field w 48 + "%" (45) · Divider · "Gridline every" w 110 (scrub) + field + "pixels" secondary (64; 2…4096) · "Subdivisions" w 110 (scrub 0.2) + field (8; 1…64) · "A subdivision every 8 pixels." callout secondary (or orange) · HStack { "Cancel" · "Restore Defaults" · Spacer · "OK" `.borderedProminent` }.

#### 2.5.6 Export JPEG — `JPEGExportSheet` (`UI/JPEGExportSheet.swift`) — padding 24, spacing 16 (width ≈ 608)
HStack spacing 8 { "Export JPEG" title2 bold · Spacer · "Fit" · `plus.magnifyingglass` · `minus.magnifyingglass` } (bottom pad −8) · preview 560×330 on `Color(white: 0.12)` (fit or zoom steps 25/50/100/200/400/800 %, nearest-neighbor ≥100%; spinner on `.regularMaterial` r=8 while encoding) · HStack { "Quality" · Slider 0…1 step 0.01 · "85%" monospaced w 45 trailing } (default 0.85 or last used) · HStack spacing 8 { "Background for transparency" · swatch (default white) } · HStack spacing 12 { "W × H px · sRGB" secondary · Spacer · file size (`ByteCountFormatter .file`) / "Updating…" / red error · "Cancel" · "Export…" }.

#### 2.5.7 Photoshop conversion report — `PSDConversionSheet` (`UI/PSDConversionSheet.swift`) — SwiftUI `.sheet`, padding 24, spacing 16, min 520×360
Title (`request.title`) title2 bold · "Compositor will convert these Photoshop features. Nothing is applied until you continue." secondary (or "Reading the file to see what needs converting.") · reading: ProgressView small + "Reading the Photoshop file…" (min h 180) / else `List` rows { layer name `.headline` · message } padding v4, min h 180 · HStack { Spacer · "Cancel" · confirm title (default action) }.

#### 2.5.8 RAW develop — `RawDevelopSheet` (`UI/RawDevelopSheet.swift`) — SwiftUI `.sheet`, padding 24, spacing 16, fixedSize
"Develop “{file}”" title2 bold · preview 560×340 on black 35% r=6 (+ spinner) · rows HStack spacing 10 { title w 90 · Slider w 300 · value monospaced secondary w 80 trailing }: Exposure −3…3 " EV" 2dp (0), Temperature 2000…12000 " K" 0dp (as shot, 5000 fallback), Tint −150…150 (as shot), Boost 0…1 2dp (1) · HStack { "Reset" · Spacer · "Cancel" · "Import" }.

### 2.6 Tools without their own header
Move/Hand/Zoom/Crop/Type/Shape/Eyedropper/Gradient/Brush-family/Selection headers are all covered above; there are no separate headers for Eraser (Brush Erase mode), Blur/Smudge/Liquify (Smear modes) or Object selection (Magic Object mode). Healing = Spot Healing only; there is no separate Healing Brush tool.

---

## 3. Menus and shortcuts

Shortcuts are the defaults from `ShortcutDefinition.all`; users can remap them (stored in UserDefaults `keyboardShortcuts.v1`). Items marked (system) come from AppKit/SwiftUI and are not declared in code; verify them on the Mac.

The Mac's menu bar, as the parity harness dumps it (`menus.json` in the references' `ui/`), runs Compositor, File, Edit, View, Select, Image, Filter, Layer, Window, Help: SwiftUI puts the View menu after Edit, although the sections below list it last. The dump also shows separators this table leaves out: after Import Images…, after Redo, and before Clear Menu in Open Recent, even when the list is empty.

### 3.1 Compositor (app menu)
| Item | Shortcut |
|---|---|
| About Compositor (system) | |
| Check for Updates… | |
| — | |
| Services ▸ (system) | |
| — | |
| Hide Compositor | none (⌘H is taken by Show Transform Controls) |
| Hide Others | ⌥⌘H |
| Show All | |
| — | |
| Quit Compositor (system) | ⌘Q |

No Settings item (the app has no Settings scene).

### 3.2 File
| Item | Shortcut | Notes |
|---|---|---|
| New Canvas… | ⌘N | |
| Open Project… | ⌘O | |
| Open Recent ▸ | | recent project names (no extension), —, Clear Menu |
| Import Images… | | |
| Save | ⌘S | |
| Save As… | ⇧⌘S | |
| — | | |
| Export PNG… | ⇧⌘E | |
| Export JPEG… | ⌥⇧⌘S | |
| — | | |
| Close Project | ⌘W | |
| Page Setup… / Print… (system `printItem` group, not replaced) | ⇧⌘P / ⌘P | verify |

### 3.3 Edit
| Item | Shortcut | Notes |
|---|---|---|
| Undo / Undo {action name} | ⌘Z | |
| Redo / Redo {action name} | ⇧⌘Z | |
| Cut | ⌘X | replaces the whole pasteboard group (no system Delete / Select All here) |
| Copy | ⌘C | |
| Copy Merged | ⇧⌘C | |
| Paste | ⌘V | |
| — | | |
| Keyboard Shortcuts… | | opens floating panel |
| Fill with Foreground Color | ⌥⌫ | |
| Fill with Background Color | ⌘⌫ | |
| Clear Selection Pixels | | |
| Content-Aware Fill… | ⇧⌫ | |
| (system) Writing Tools, AutoFill, Start Dictation…, Emoji & Symbols | ⌃⌘Space etc. | appended by macOS |

### 3.4 Select
| Item | Shortcut |
|---|---|
| All | ⌘A |
| Deselect | ⌘D |
| Inverse | ⇧⌘I |
| Layer's Pixels | |
| Subject | ⌥⌘A |
| Color Range… | |
| Mask's Black Areas | |
| — | |
| Expand… | |
| Contract… | |
| Feather… | |

### 3.5 Image
| Item | Shortcut |
|---|---|
| Curves… | ⌘M (conflicts with system Window ▸ Minimize ⌘M; check which one the Mac shows) |
| Levels… | ⌘L |
| Hue/Saturation… | ⌘U |
| Black & White… | |
| Color Balance… | |
| Exposure… | |
| Gradient Map… | |
| Grain… | |
| Invert / Invert Mask | ⌘I |
| — | |
| Canvas Size… | ⌥⌘C |
| Image Size… | ⌥⌘I |
| Trim… | |
| — | |
| Flip Canvas Horizontal | |
| Flip Canvas Vertical | |

### 3.6 Filter
Gaussian Blur…, Motion Blur…, Add Noise…, Vignette…, Bloom / Glow…, Dither…, Tonal Contrast…, Lens Correction…, Camera Raw Filter…, Remove Background… (no shortcuts; order = `FilterKind.allCases` minus Content-Aware Fill and image adjustments).

### 3.7 Layer
| Item | Shortcut |
|---|---|
| New Adjustment Layer ▸ Hue/Saturation…, Levels…, Curves…, Exposure…, Gradient Map…, Grain…, Add Noise…, Gaussian Blur…, Motion Blur…, Invert, Black & White…, Color Balance… | |
| Edit Adjustment… | |
| — | |
| Transform Layer / Transform Selection | ⌘T |
| Duplicate Layer / Layer via Copy (with a selection) | ⌘J |
| — | |
| Create Clipping Mask / Release Clipping Mask | ⌥⌘G |
| — | |
| Group Selected Layers | ⌘G |
| Ungroup Layers | ⇧⌘G |
| Move Out of Folder | |
| New Blank Layer | ⇧⌘N |
| Rename Layer… | |
| Hide Layer / Show Layer | |
| — | |
| Move Layer Up | ⌘] |
| Move Layer Down | ⌘[ |
| Merge Down / Merge Layers / Merge Group | ⌘E |
| — | |
| Flip Layer Horizontal | |
| Flip Layer Vertical | |
| — | |
| Delete Layer / Delete Layers / Delete Layer Mask / Delete {Effect name} | |

### 3.8 View
System items first (Show Toolbar / Customize Toolbar… — verify), then, after `.toolbar`:

| Item | Shortcut | Notes |
|---|---|---|
| Fit Canvas | ⌘0 | zooms the Export JPEG preview when that sheet is open |
| Actual Pixels | ⌘1 | |
| Zoom In | ⌘= | |
| Zoom Out | ⌘- | |
| Pixel Grid (800% and above) ✓ | | default on |
| Snap ✓ | | `snappingEnabled`, default on (first of two "Snap" items) |
| Show Transform Controls ✓ | ⌘H | enabled only with the Move tool |
| — | | |
| Show ▸ Grid ✓ (⌘'), Guides ✓ (⌘;) | | defaults: grid off, guides on |
| Grid Settings… | | |
| Rulers ✓ | ⌘R | default off |
| — | | |
| Snap ✓ | ⇧⌘; | `snapEnabled`, default on |
| Snap To ▸ Guides ✓, Grid ✓, Layers ✓, Document Bounds ✓ | | defaults on, off, on, on |
| — | | |
| Lock Guides ✓ | ⌥⌘; | default off |
| Clear Guides | | |
| Enter Full Screen (system) | ⌃⌘F | |

### 3.9 Window and Help (system)
Window: Minimize ⌘M, Zoom, Fill/Center/Move & Resize (macOS 15+), Bring All to Front, window list. Help: search field + "Compositor Help". Nothing declared in code.

### 3.10 Layer row context menu (`NativeLayerList.Coordinator.contextMenu(for:)`)
Right-clicking selects the row (or the thumbnail/mask target) first.
1. Duplicate Layer
2. Rename…
3. Delete Layer / Delete Selected Layers / Delete Mask
4. —
5. Create Clipping Mask / Release Clipping Mask
6. Group Selected Layers
7. Ungroup Layers (only when the right-clicked row is a folder)
8. Move Out of Folder
9. Merge Down / Merge Layers / Merge Group
10. —
11. Add Mask ▸ Reveal All (White), Hide All (Black)
12. Enable Mask / Disable Mask
13. Delete Mask
14. Link Mask / Unlink Mask
15. —
16. Hide Layer / Show Layer

No key equivalents shown. Enabled states are in `validateMenuItem`.

Other context menus: Camera Raw scope (Histogram, Vectorscope); tab overflow pill menu (hidden tab titles).

### 3.11 Non-menu keys (group "Canvas & Layers" and "Text Editing")
| Action | Default |
|---|---|
| Select tool (idle) / Move / Hand / Zoom / Brush / Eraser / Spot Healing / Clone Stamp / Type / Gradient / Shape / Eyedropper / Marquee (cycle) / Magic / Lasso (cycle) / Blur-Smudge-Liquify / Crop | A / V / H / Z / B / E / J / S / T / G / U / I / M / W / L / R / C (Shift+letter also works) |
| Swap colors / Reset colors | X / D |
| Cycle tool mode | Tab |
| Temporary Hand (hold) | Space |
| Delete selection / layer / effect / lasso point | Delete |
| Apply / Cancel current canvas operation | Return / Esc |
| Brush size − / + | [ / ] |
| Brush hardness − / + | ⇧[ / ⇧] |
| Previous / next blend mode | ⇧- / ⇧= |
| Cycle shape kind | ⇧U |
| Opacity digits (two digits = exact %) | 0–9 |
| Nudge 1 px / 10 px | arrows / ⇧arrows |
| Move selected pixels 1 px / 10 px | ⌘arrows / ⇧⌘arrows |
| Toggle Levels preview | ⌥P |
| Finish editing text | ⌘Return |
| Tracking −/+ (by 10) | ⌥← / ⌥→ (⌥⇧) |
| Leading −/+ (by 10) | ⌥↑ / ⌥↓ (⌥⇧) |

Reserved (cannot be assigned): ⌘Q, ⌘, and ⌥⌘M.

---

## 4. Headless rendering in ParityHarness

### 4.1 Harness prerequisites
- Compile the bridging header C files in `Rendering/*.c` and link **Sparkle** (`IO/CompositorApplicationDelegate.swift` imports it). If only `ContentView(session:)` is needed, the delegate can be left out and `applicationDelegate` passed as nil (no tab strip, no project window bridge).
- `NSApplication.shared` must exist before any window. Set `NSApp.setActivationPolicy(.accessory)`, `NSApp.appearance = NSAppearance(named: .darkAqua)` (the delegate normally does this), and call `SliderSnap.install()` for parity of slider behavior (not needed for static renders).
- Persisted state that changes rendering. Pin it:
  - `ToolDefaults` reads `UserDefaults` keys `tool.*` unless the env var `XCTestConfigurationFilePath` is set; set it (any value) so the compiled defaults are used.
  - `@AppStorage("layersPanelWidth")` (252), `jpegExportQuality` (0.85), `keyboardShortcuts.v1` (shortcut labels), `CompositorCPUCanvas`. Run the harness with an empty defaults domain (unique bundle id or `-layersPanelWidth 252` style argument overrides).
  - `NewCanvasSheet` reads the general pasteboard on first appear; set `session.skipsInitialClipboardCanvasSize = true` or clear the pasteboard.
- Deterministic session: `let s = EditorSession(); s.createDocument(width:height:emptyLayer:)`; add pixels with `s.insert(ImportedImage(image: cg, thumbnail: cg, name: "…"))` (tests use this). Layer ids are random UUIDs but never drawn.

### 4.2 Render recipe
ImageRenderer cannot draw AppKit-backed controls on macOS (TextField, Toggle, Slider, Picker, segmented, bordered Button, List, ScrollView scrollers, every `NSViewRepresentable`); they render blank or as placeholders. Almost every view here contains one, so use the pattern the app's own tests use (`CompositorTests/LevelsTests.swift` `panelPreview`, `MaskAloneTests.swift`):

```swift
let host = NSHostingView(rootView: AnyView(view.roundedControls()))
host.frame = CGRect(origin: .zero, size: fixedSize ?? host.fittingSize)
let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
window.appearance = NSAppearance(named: .darkAqua)
window.contentView = host
window.makeKeyAndOrderFront(nil)            // key window: accent-colored selection, prominent buttons
RunLoop.main.run(until: Date().addingTimeInterval(0.1))
host.layoutSubtreeIfNeeded()
window.makeFirstResponder(nil)              // no caret / focus ring, unless the state under test needs them
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w*2, pixelsHigh: h*2, bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
rep.size = host.bounds.size                  // fixes 2× regardless of the screen
host.cacheDisplay(in: host.bounds, to: rep)
let png = rep.representation(using: .png, properties: [:])
```

`CanvasView` overrides `cacheDisplay` so snapshots always use the Core Graphics path (the Metal `MetalCanvasView` is skipped). A window is required for: every NSViewRepresentable, `NativeLayerList` (NSTableView), `CanvasView` (viewport size and backing scale come from `layout`/`viewDidMoveToWindow`), and any view whose appearance depends on key/first-responder status. `NSApp.run()` is not required; pumping the run loop is enough.

### 4.3 Per-view instantiation

| View | Type / file | Instantiate | Render | Needs window | Live-state hazards |
|---|---|---|---|---|---|
| Whole editor | `ContentView` (`ContentView.swift`) | `ContentView(session: s)` (delegate nil); frame 1180 × 780, the scene's default size, as the harness hosts it | NSHostingView | yes | toolbar lives in the window title bar, not the content view (see §4.4); canvas as below |
| Workspace + tabs | `ProjectWorkspaceView` | needs `CompositorApplicationDelegate()` (Sparkle) | NSHostingView | yes | tab strip only appears inside the real toolbar |
| Tab strip | `ProjectTabStrip` | `ProjectTabStrip(workspace: ProjectWorkspace())`, add tabs with `addTab(reuseEmpty: false)`; frame (828, 34) | NSHostingView | no (but has `TitleBarDragView`) | 0.1 s Timer polling the drag pasteboard; reorder animation |
| Tool rail | private `toolRail` in ContentView | not accessible directly: crop from the whole-editor render (x 0…56 below the header) or copy into the harness | NSHostingView | yes (IndicatorlessScrollView) | none |
| Color palette | `ColorPaletteControls` | `ColorPaletteControls(session: s)` | NSHostingView (ImageRenderer would work too: plain buttons only) | no | popover only when mask targeted |
| Status bar | private in ContentView | crop from whole editor | | | busy spinner appears after 250 ms of work |
| Canvas | `EditorCanvas` / `CanvasView` (`Rendering/EditorCanvas.swift`) | `EditorCanvas(session: s)` or `CanvasView(session: s)` directly; after layout call `s.fit()` or `s.zoom(to:)` | `cacheDisplay` (CG path forced) | yes | marching ants phase advances every 0.12 s (capture right after layout or with no selection); brush cursor/sample ring follow the mouse; backing scale from the window |
| Rulers | `CanvasRulerView`, `CanvasRulerCorner` | need the viewport the canvas set | NSHostingView | yes | redrawn from viewport |
| Mask badge | `MaskAloneBadge` | needs `s.viewsMaskAlone = true` on a layer with a mask | NSHostingView | yes | |
| Tool headers | `TransformInspector`, `BrushControls`, `LassoControls`, `GradientControls`, `TypeControls`, `ShapeControls`, `CropControls`, `NavigationToolHeader` | `s.selectTool(.x)` then `XControls(session: s)`, frame width 1180 − 0 (full), height 42 | NSHostingView | yes (AppKit pop-ups in Type; all TextFields) | HeldModifiers (live ⌘/⇧ flips Auto Select / lock); Transform Apply buttons fade 0.12 s; `TypeFontPicker` builds its font list in a background task (closed control only shows the current name) |
| Eyedropper / idle headers | inline in ContentView | copy the 5-line HStacks or crop | | | |
| Layers panel | `LayersPanel` | `LayersPanel(session: s, width: 252)`, height e.g. 600 | NSHostingView | yes | NSTableView selection color depends on first responder + key window; overlay vs legacy scrollers follow the system "Show scroll bars" setting (set `scrollerStyle = .overlay` on the found NSScrollView); blend pop-up, opacity slider |
| Layer list only | `NativeLayerList` | `NativeLayerList(session: s)` | NSHostingView | yes | as above; thumbnails render synchronously |
| Layer context menu | `NativeLayerList.Coordinator(session:).contextMenu(for:)` | returns `NSMenu` | not renderable as pixels headlessly: dump item tree (title, key, enabled, submenu, separators) to JSON and compare structurally | — | |
| Blend pop-up | `BlendModePicker` | inside LayersPanel | NSHostingView | yes | open menu not capturable |
| Levels | `LevelsSheet` | `s.beginLevels()`; `await s.levels?.histogramTask?.value` | NSHostingView | yes | "Loading histogram…" until the task finishes |
| Hue/Saturation | `HueSaturationSheet` | `s.beginHueSaturation()` | NSHostingView | yes (`CameraRawSlider`) | async preview job (no visual effect on the panel) |
| Color Range | `ColorRangeSheet` | `s.beginColorRange()` (needs pixels) | NSHostingView | yes | preview image stays black until a color is sampled |
| Selection amount | `SelectionAmountSheet` | `s.selectAll(); s.promptSelectionAmount(.feather)`; `SelectionAmountSheet(session: s, operation: .feather)` | NSHostingView | yes | field is focused on appear (focus ring, caret blink) |
| Effects | `EffectsSheet` | `s.addEffect(.shadow)` (opens editing); `EffectsSheet(session: s, kind: .shadow)` | NSHostingView | yes | |
| Filters | `FilterSheet` | `s.beginFilter(.gaussianBlur)` etc. | NSHostingView | yes (Black & White, Color Balance use CameraRawSlider) | preview renders async; Remove Background / Content-Aware Fill show "Working…" spinner until the Vision/fill job finishes |
| Camera Raw | `FilterSheet` with `.cameraRaw` or `CameraRawControls(session: s)` | `s.beginFilter(.cameraRaw)`; give the host a fixed 440 × 780 frame | NSHostingView | yes | histogram/vectorscope (`cameraRawScope`) computed async; readout shows "—" until the pointer hovers the canvas; Option key monitor |
| Curves (filter) | `CurvesControls(settings: $binding)` | standalone with a `@State` binding | NSHostingView | no | |
| Color picker | `ColorPickerSheet(state:finish:)` | `ColorPickerState(background: false, original: .black)` or `s.openColorPicker(background: false)` | NSHostingView | yes (TextFields) | |
| Keyboard shortcuts | `KeyboardShortcutsSheet` (private) | only via `ShortcutSettings.shared.show()` (opens its own NSPanel); to render, make the struct internal in the harness copy or capture the panel's `contentView` | NSHostingView / panel contentView | yes | LazyVStack in a ScrollView renders only visible rows; recorder buttons are NSButtons |
| New canvas / welcome | `NewCanvasSheet(session: s)` | set `skipsInitialClipboardCanvasSize` | NSHostingView | yes | width field focused on appear |
| Canvas Size | `CanvasSizeSheet(document:session:finish:)` | `s.document!` | NSHostingView | yes | |
| Image Size | `ImageSizeSheet(document:finish:)` | | NSHostingView | yes | |
| Trim | `TrimSheet(finish:)` | | NSHostingView | yes | radio group is AppKit |
| Grid Settings | `GridSettingsSheet(session:grid:appearance:preview:finish:)` | `LayoutGrid()`, `GridAppearance()` | NSHostingView | yes | |
| Export JPEG | `JPEGExportSheet(raster:session:finish:)` | needs an `ExportRaster` from the session's export path | NSHostingView | yes | 200 ms debounce then async encode; spinner on `.regularMaterial` (materials do not capture reliably); file size text depends on the encoder |
| PSD report | `PSDConversionSheet(request:finish:)` | build `PSDConversionRequest(title:confirmTitle:conversions:)` by hand | NSHostingView | yes (`List`) | spinner when `isReading` |
| RAW develop | `RawDevelopSheet(session:url:settings:)` | needs a real RAW file; preview developed async via Core Image RAW | NSHostingView | yes | slow and camera-dependent; consider excluding from pixel parity |
| Tool icons | `GradientToolIcon`, `CloneStampToolIcon`, `PolygonalLassoToolIcon`, `ObjectSelectionToolIcon`, `SpectrumEditor`, `CanvasRulerCorner` | direct | **ImageRenderer** works (pure `Canvas`/shapes); use `.foregroundStyle(.white)` and `renderer.scale = 2` | no | |
| Floating panel chrome | `FloatingPanelController` | | title bar is window chrome; compare content only and draw the title bar separately | | panel frame positions are per-session memory |
| Main menu | `CompositorApp.commands` | not compiled in the harness | compare §3 as data (e.g. dump `NSApp.mainMenu` from the real app to JSON) | | |

### 4.4 Things that are hard to pin down
- **System accent color** (`controlAccentColor`, `Color.accentColor`): user setting. Record it on the Mac run (e.g. write its sRGB value into the PNG's sidecar) and use the same value on Windows.
- **Key window / first responder**: table selection, focus rings, prominent buttons and segmented controls look different in inactive windows. Keep the harness window key.
- **Liquid Glass / materials / vibrancy**: the unified toolbar, `.regularMaterial` and NSVisualEffectView backgrounds do not render through `cacheDisplay`. Capturing the toolbar needs a real on-screen window and `CGWindowListCreateImage`/ScreenCaptureKit (Screen Recording permission). Treat toolbar parity as a separate, looser check.
- **Timers and async work**: marching ants (0.12 s), tab strip pasteboard poll (0.1 s), busy indicator (250 ms), JPEG encode (200 ms debounce), Levels histogram, Camera Raw scope, Color Range preview, font menu preload, filter previews. Await the task or wait for the value before capturing.
- **Animations**: Transform Apply/Cancel fade (0.12 s), tab reorder (0.15 s), CameraRawSlider track click (0.18 s), NSTableView row changes (disabled in code). Capture after ≥0.3 s idle.
- **Hover / pointer**: scrub-label cursors, brush cursor overlay, sample ring, Camera Raw RGB readout. Keep the pointer outside the window.
- **Text caret blink** in focused fields (NewCanvasSheet, SelectionAmountSheet focus on appear).
- **Scrollers**: overlay vs always-visible follows the system setting and input device.
- **Fonts**: SF Pro with macOS text rendering. Windows will need a substitute (e.g. bundle Inter or use Segoe UI Variable) and a tolerance in the diff.
- **Backing scale**: pin 2× with a manually sized `NSBitmapImageRep`; `CanvasThumbnail` already renders at a fixed 2×.

---

## 5. SF Symbols used

| Symbol | Where |
|---|---|
| `arrow.up.left.and.arrow.down.right` | tool: Move |
| `rectangle.dashed` | tool: Marquee (rect); marquee cursor |
| `circle.dashed` | tool: Marquee (ellipse); cursor |
| `lasso` | tool: Lasso; cursor |
| `wand.and.stars` | tool: Magic (Wand) |
| `crop` | tool: Crop |
| `paintbrush.pointed` | tool: Brush |
| `eraser` | tool: Brush in Erase mode |
| `bandage` | tool: Spot Healing |
| `seal` | tool: Clone Stamp (symbol fallback; the rail draws a custom icon) |
| `drop` | tool: Smear |
| `square.bottomhalf.filled` | tool: Gradient (symbol fallback; custom icon drawn) |
| `square.on.circle` | tool: Shape |
| `textformat` | tool: Type; editable-text layer thumbnail |
| `eyedropper` | tool: Eyedropper; Levels/Hue-Sat/Color Range/Camera Raw samplers; eyedropper cursors |
| `hand.draw` | tool: Hand |
| `magnifyingglass` | tool: Zoom |
| `plus` | toolbar New canvas; tab "New" drop slot; eyedropper-cursor badge |
| `minus` | eyedropper-cursor badge |
| `plus.magnifyingglass`, `minus.magnifyingglass` | toolbar zoom; JPEG preview zoom; zoom cursors |
| `xmark` | tab close; mask-alone badge close |
| `chevron.down`, `chevron.right` | tab overflow pill; folder disclosure; Camera Raw sections |
| `arrow.left.and.right` | swap colors (rotated 45°) |
| `arrow.counterclockwise` | reset colors |
| `square.3.layers.3d` | Layers empty state |
| `plus.square` | Layers footer: new layer |
| `folder.badge.plus` | Layers footer: group |
| `rectangle.inset.filled` | Layers footer: add mask; mask-alone badge |
| `sparkles` | Layers footer: effects menu |
| `circle.lefthalf.filled` | Layers footer: adjustment menu; Hue/Saturation adjustment icon |
| `trash` | Layers footer: delete |
| `eye`, `eye.slash` | layer visibility; effect rows; Camera Raw section eyes |
| `eye.fill` | show-mask-alone cursor |
| `link` | layer–mask link (rotated 45°); Transform lock-ratio toggle |
| `folder` | folder layer thumbnail |
| `arrow.turn.down.right`, `rectangle.badge.plus`, `rectangle.badge.minus` | clipping-mask cursors |
| `scissors` | move-pixels cursor |
| `arrow.triangle.2.circlepath` | rotate cursor |
| `text.alignleft`, `text.aligncenter`, `text.alignright` | Type alignment |
| `triangle.fill` | Levels handles; Camera Raw clipping indicators |
| `plus.circle.fill`, `minus.circle.fill` | Add/Remove eyedropper badges |
| `hand.point.up.left` | Hue/Saturation targeted adjustment |
| `scope` | Camera Raw Targeted Adjustment |
| `line.diagonal` | Camera Raw Draw Guides |
| `multiply` | New canvas "×" |
| `circle`, `circle.fill` | Canvas Size anchor grid |
| `slider.horizontal.3` | Levels adjustment icon |
| `point.topleft.down.to.point.bottomright.curvepath` | Curves adjustment icon (rotated 90°) |
| `plusminus.circle` | Exposure adjustment icon |
| `paintpalette` | Gradient Map adjustment icon |
| `circle.grid.3x3` | Grain adjustment icon |
| `drop.fill` | Gaussian Blur adjustment icon |
| `wind` | Motion Blur adjustment icon |
| `circle.dotted` | Add Noise adjustment icon |
| `circle.righthalf.filled` | Invert adjustment icon |
| `circle.filled.pattern.diagonalline.rectangle` | Black & White adjustment icon |
| `scale.3d` | Color Balance adjustment icon |

Custom-drawn icons (no SF Symbol; port the paths): `GradientToolIcon`, `CloneStampToolIcon`, `PolygonalLassoToolIcon`, `ObjectSelectionToolIcon`, the three-dot presets button in New canvas, the hue-strip arrows in the color picker.
