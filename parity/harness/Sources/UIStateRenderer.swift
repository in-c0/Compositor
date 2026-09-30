import AppKit
import SwiftUI

nonisolated enum UIStatus: String, Codable, Sendable {
    case ok, error
}

/// How one state went, as ui-info.json lists it.
nonisolated struct UIStateResult: Codable, Sendable {
    let id: String
    var status: UIStatus
    var error: String?
    /// Relative to the output folder.
    var output: String?
    /// Pixel size of the PNG; at 1x, also its size in points.
    var width: Int?
    var height: Int?
    var backingScale: Double?
    var keyWindow: Bool?
    var appActive: Bool?
    var notes: [String] = []

    static func failure(_ id: String, _ error: String, notes: [String] = []) -> UIStateResult {
        UIStateResult(id: id, status: .error, error: error, notes: notes)
    }
}

/// The editor window's content size: the Window scene's default size. The toolbar isn't part of it; it lives in the
/// window's title bar, which isn't captured.
let editorContentSize = CGSize(width: 1180, height: 780)
/// ContentView's fixed bars: the tool header and its divider on top, the divider and status bar at the bottom, and
/// the tool rail's width.
let headerHeight = ToolHeaderStyle.height
let dividerThickness: CGFloat = 1
let statusBarHeight: CGFloat = 30
let toolRailWidth: CGFloat = 56
/// The Layers panel as the editor shows it by default (`layersPanelWidth`), at a fixed height.
let layersPanelSize = CGSize(width: 252, height: 600)
/// Camera Raw docks to the window's right edge at this width and the window's height.
let cameraRawSize = CGSize(width: FloatingPanelController.dockedWidth, height: editorContentSize.height)

/// Renders one state in this process: opens its document, picks its tool, builds the view the way the app hosts it,
/// and captures it.
struct UIStateRenderer {
    let state: UIState
    let options: UIOptions

    func render() async -> UIStateResult {
        var notes: [String] = []
        let output = state.id + ".png"
        let url = options.output.appending(path: output)
        try? FileManager.default.removeItem(at: url)
        do {
            let snapshot = try await capture(notes: &notes)
            try snapshot.writePNG(to: url)
            var result = UIStateResult(id: state.id, status: .ok, output: output, notes: notes)
            result.width = snapshot.image.pixelsWide
            result.height = snapshot.image.pixelsHigh
            result.backingScale = Double(snapshot.backingScale)
            result.keyWindow = snapshot.keyWindow
            result.appActive = NSApp.isActive
            if !snapshot.keyWindow { result.notes.append("The window wasn't key when captured, so controls may draw as inactive.") }
            return result
        } catch {
            try? FileManager.default.removeItem(at: url)
            return .failure(state.id, describe(error), notes: notes)
        }
    }

    private func capture(notes: inout [String]) async throws -> Snapshot {
        let session = EditorSession()
        // The welcome form would otherwise take its size from whatever image is on the pasteboard.
        session.skipsInitialClipboardCanvasSize = true
        if let document = state.document {
            notes += try await UIDocuments.open(document, corpus: options.corpus, into: session)
        }
        if let index = state.layer {
            notes.append(try UIDocuments.selectLayer(index, in: session))
        }
        if let tool = state.tool { try UIDocuments.selectTool(tool, in: session) }

        switch state.view {
        case "window":
            return try await editor(session)
        case "tool-header":
            return try await editor(session, crop: CGRect(x: 0, y: 0, width: editorContentSize.width, height: headerHeight))
        case "tool-rail":
            let top = headerHeight + dividerThickness
            return try await editor(session, crop: CGRect(x: 0, y: top, width: toolRailWidth,
                                                          height: editorContentSize.height - top - dividerThickness - statusBarHeight))
        case "status-bar":
            return try await editor(session, crop: CGRect(x: 0, y: editorContentSize.height - statusBarHeight,
                                                          width: editorContentSize.width, height: statusBarHeight))
        case "layers-panel":
            let stage = Stage(AnyView(LayersPanel(session: session, width: layersPanelSize.width).roundedControls()), size: layersPanelSize)
            defer { stage.close() }
            await stage.show()
            return try stage.snapshot()
        case "sheet":
            return try await sheet(state.sheet ?? "", session: session, notes: &notes)
        default:
            throw HarnessError("view “\(state.view)” isn't supported")
        }
    }

    /// The editor window's content, `ContentView` as the scene hosts it (without the app delegate, so there is no tab
    /// strip; it lives in the toolbar anyway). The tool rail, headers and status bar are private to ContentView, so
    /// those states are cropped from this.
    private func editor(_ session: EditorSession, crop: CGRect? = nil) async throws -> Snapshot {
        let stage = Stage(AnyView(ContentView(session: session).roundedControls()), size: editorContentSize)
        defer { stage.close() }
        await stage.show()
        // The canvas learns its size from layout; fit the document to it as opening a project does.
        session.fit()
        await settle(0.3)
        return try stage.snapshot(crop: crop)
    }

    // MARK: Sheets and panels

    private func sheet(_ name: String, session: EditorSession, notes: inout [String]) async throws -> Snapshot {
        func document() throws -> CanvasDocument {
            guard let document = session.document else { throw HarnessError("sheet “\(name)” needs a document") }
            return document
        }
        func opened(_ what: String, _ isOpen: Bool) throws {
            guard isOpen else { throw HarnessError(session.brushError ?? "the app wouldn't open \(what) in this state") }
        }
        if name.hasPrefix("filter:") {
            let raw = String(name.dropFirst("filter:".count))
            guard let kind = FilterKind(rawValue: raw) else {
                throw HarnessError("“\(raw)” isn't a filter; the filters are \(FilterKind.allCases.map(\.rawValue).joined(separator: ", "))")
            }
            session.beginFilter(kind)
            try opened(kind.rawValue, session.filterEdit != nil)
            defer { session.cancelFilter() }
            if kind == .cameraRaw {
                // Docked: the panel is as tall as the document window, and its content fills it.
                return try await capture(FilterSheet(session: session), panel: true, size: cameraRawSize) {
                    try await waitForFilterPreview(session, notes: &notes)
                    await settle(1)
                }
            }
            return try await capture(FilterSheet(session: session), panel: true) {
                try await waitForFilterPreview(session, notes: &notes)
            }
        }
        switch name {
        case "new-canvas":
            // The welcome form, shown in the empty canvas; the scene's `.roundedControls()` reaches it.
            return try await capture(NewCanvasSheet(session: session), panel: true)
        case "canvas-size":
            // Window sheets host the view in an NSHostingController with no modifiers of their own.
            return try await capture(CanvasSizeSheet(document: try document(), session: session) { _ in }, panel: false)
        case "image-size":
            return try await capture(ImageSizeSheet(document: try document()) { _ in }, panel: false)
        case "trim":
            _ = try document()
            return try await capture(TrimSheet { _ in }, panel: false)
        case "grid-settings":
            _ = try document()
            return try await capture(GridSettingsSheet(session: session, grid: session.layoutGrid, appearance: session.gridAppearance,
                                                       preview: { _, _ in }) { _ in }, panel: false)
        case "export-jpeg":
            guard let snapshot = session.projectSnapshot() else { throw HarnessError("sheet “\(name)” needs a document") }
            // ProjectController.exportJPEG: the flattened image, then the sheet, which encodes a preview after a pause.
            let raster = try await ImageExporter.shared.render(snapshot)
            return try await capture(JPEGExportSheet(raster: raster, session: session) { _ in }, panel: false) {
                await settle(2)
            }
        case "levels":
            session.beginLevels()
            try opened("Levels", session.levels != nil)
            defer { session.cancelLevels() }
            await session.levels?.histogramTask?.value
            return try await capture(LevelsSheet(session: session), panel: true)
        case "hue-saturation":
            session.beginHueSaturation()
            try opened("Hue/Saturation", session.hueSaturation != nil)
            defer { session.cancelHueSaturation() }
            return try await capture(HueSaturationSheet(session: session), panel: true) { await settle(0.5) }
        case "color-range":
            session.beginColorRange()
            try opened("Color Range", session.colorRange != nil)
            defer { session.cancelColorRange() }
            return try await capture(ColorRangeSheet(session: session), panel: true)
        case "layer-effects":
            guard let layer = session.activeLayer else { throw HarnessError("sheet “\(name)” needs a selected layer") }
            guard let kind = layer.effects?.kinds.first else {
                throw HarnessError("layer “\(layer.name)” has no effects to edit; pick a layer that has some")
            }
            // Double-clicking the effect's row: the first of the layer's effects, in the panel's order.
            session.selectEffect(kind, on: layer.id, editing: true)
            try opened(kind.rawValue, session.effectsEditing != nil)
            notes.append("Editing \(kind.rawValue) on “\(layer.name)”.")
            defer { session.finishEffectsEditing(commit: false) }
            return try await capture(EffectsSheet(session: session, kind: kind), panel: true)
        case "color-picker":
            // Clicking the foreground swatch.
            session.openColorPicker(background: false)
            guard let picker = session.colorPicker else { throw HarnessError("the app wouldn't open the color picker in this state") }
            defer { session.closeColorPicker(commit: false) }
            return try await capture(ColorPickerSheet(state: picker) { _ in }, panel: true)
        case "keyboard-shortcuts":
            return try await keyboardShortcuts()
        default:
            throw HarnessError("sheet “\(name)” isn't known; see parity/README.md for the sheet names")
        }
    }

    /// `panel` true hosts the view as `FloatingPanelController` does (`AnyView(content.roundedControls())`); false as
    /// ProjectController's window sheets do (the view alone). `prepare` runs once the window is showing.
    private func capture(_ content: some View, panel: Bool, size: CGSize? = nil,
                         prepare: () async throws -> Void = {}) async throws -> Snapshot {
        let root = panel ? AnyView(content.roundedControls()) : AnyView(content)
        let stage = Stage(root, size: size)
        defer { stage.close() }
        await stage.show()
        try await prepare()
        await settle(0.2)
        return try stage.snapshot()
    }

    /// Filters preview asynchronously; Remove Background and Content-Aware Fill show a spinner until theirs is done.
    private func waitForFilterPreview(_ session: EditorSession, notes: inout [String]) async throws {
        let deadline = Date().addingTimeInterval(10)
        while session.filterEdit?.preparing == true, Date() < deadline { await settle(0.05) }
        if session.filterEdit?.preparing == true { notes.append("The filter's preview was still being prepared after 10 s.") }
        if let error = session.filterEdit?.previewError { notes.append("Preview error: \(error)") }
    }

    /// Edit > Keyboard Shortcuts…: the sheet is private to the app, so it's opened as the menu opens it, in the
    /// app's own floating panel, and the panel's content view is captured.
    private func keyboardShortcuts() async throws -> Snapshot {
        ShortcutSettings.shared.show()
        defer { ShortcutSettings.shared.close() }
        await settle(0.4)
        guard let panel = NSApp.windows.first(where: { $0.identifier?.rawValue == "keyboardShortcuts" }),
              let content = panel.contentView else { throw HarnessError("the Keyboard Shortcuts panel didn't open") }
        panel.appearance = NSAppearance(named: .darkAqua)
        panel.makeKeyAndOrderFront(nil)
        await settle(0.4)
        _ = panel.makeFirstResponder(nil)
        await settle(0.2)
        return try Snapshot.take(content, in: panel)
    }
}

/// Opening documents and choosing tools and layers, as the app's menus and keys do.
enum UIDocuments {
    /// File > Open on the corpus case's input.comp. Returns notes for the state.
    static func open(_ caseID: String, corpus: URL, into session: EditorSession) async throws -> [String] {
        let folder = corpus.appending(path: caseID)
        let spec: CaseSpec
        do {
            spec = try CaseSpec.load(from: folder.appending(path: "case.json"))
        } catch {
            throw HarnessError("document \(caseID): \(describe(error))")
        }
        guard spec.input.lowercased().hasSuffix(".comp") else {
            throw HarnessError("document \(caseID): its input is \(spec.input); only .comp inputs can be opened as a state's document")
        }
        let input = folder.appending(path: spec.input)
        let snapshot = try await ProjectStore.shared.load(from: input)
        session.installProject(snapshot, from: input)
        return spec.ops.isEmpty ? [] : ["The document is \(caseID)'s input as saved; its ops aren't applied."]
    }

    /// Clicking a layer's row. `index` counts from the bottom of the stack, 0 first.
    static func selectLayer(_ index: Int, in session: EditorSession) throws -> String {
        guard let layers = session.document?.layers else { throw HarnessError("layer \(index): the state has no document") }
        guard layers.indices.contains(index) else { throw HarnessError("layer \(index): the document has \(layers.count) layers") }
        let layer = layers[index]
        session.selectLayer(layer.id)
        guard session.activeLayerID == layer.id else { throw HarnessError("layer \(index) (“\(layer.name)”) couldn't be selected") }
        return "Layer \(index) is “\(layer.name)”."
    }

    /// A tool's key. Names are the tool rail's tools in kebab case (`spot-healing`), plus `eraser` (the Brush in Erase
    /// mode, E), `magic` (the Wand tool, W), `smear` (R) and `none` (A).
    static func selectTool(_ name: String, in session: EditorSession) throws {
        let aliases: [String: NavigationTool] = ["eraser": .brush, "magic": .wand, "smear": .blur, "none": .idle]
        let camel = name.split(separator: "-").enumerated()
            .map { $0.offset == 0 ? String($0.element) : $0.element.prefix(1).uppercased() + $0.element.dropFirst() }.joined()
        guard let tool = aliases[name] ?? NavigationTool(rawValue: camel) else {
            throw HarnessError("tool “\(name)” isn't known; the tools are \(NavigationTool.allCases.map(\.rawValue).joined(separator: ", ")), eraser, magic, smear")
        }
        session.selectTool(tool)
        guard session.tool == tool else { throw HarnessError("the app didn't switch to the \(name) tool") }
        if tool == .brush { session.brushMode = name == "eraser" ? .erase : .paint }
    }
}
