import CoreGraphics
import Foundation

/// A `stroke` op: one press, drag and release of a painting tool, fed to the same `EditorSession` calls the canvas's
/// mouse events make (`EditorCanvas.mouseDown`, `mouseDragged`, `mouseUp`). See parity/README.md.
struct StrokeOp {
    enum Tool: String, CaseIterable {
        case brush, eraser, heal, clone, blur, smudge, liquify
    }

    let tool: Tool
    let layer: UUID
    /// Paint the layer's mask rather than its pixels.
    let mask: Bool
    /// Document pixels, top-left origin, as `CanvasViewport.documentPoint` gives them.
    let points: [CGPoint]
    /// Shift held on the press: a straight line on from where the last stroke ended.
    let shift: Bool
    // Options-bar settings; nil keeps what the tool has.
    var size: CGFloat?
    var hardness: CGFloat?
    var opacity: CGFloat?
    var smoothing: CGFloat?
    var blurRadius: CGFloat?
    /// The foreground color, 0–1 per channel, for Brush on pixels.
    var color: PaletteColor?
    /// Painting a mask: White · Reveal rather than Black · Hide.
    var white: Bool?
    var healingMode: SpotHealingMode?
    /// Clone Stamp: an Option-click here before the stroke.
    var source: CGPoint?
    var aligned: Bool?
    var sampleAll: Bool?

    static func parse(_ fields: JSONFields, path: String) throws -> StrokeOp {
        let tool = try JSONValue.choice(Tool.self, fields.required("tool"), "\(path).tool")
        let layerText = try JSONValue.string(fields.required("layer"), "\(path).layer")
        guard let layer = UUID(uuidString: layerText) else { throw HarnessError("\(path).layer “\(layerText)” isn't a UUID") }
        let target = try fields.optional("target", JSONValue.string) ?? "pixels"
        guard target == "pixels" || target == "mask" else { throw HarnessError("\(path).target must be “pixels” or “mask”") }
        let points = try JSONValue.array(fields.required("points"), "\(path).points").enumerated().map { index, item -> CGPoint in
            let xy = try JSONValue.numbers(item, count: 2, "\(path).points[\(index)]")
            return CGPoint(x: xy[0], y: xy[1])
        }
        guard !points.isEmpty else { throw HarnessError("\(path).points needs at least one point") }
        var op = StrokeOp(tool: tool, layer: layer, mask: target == "mask", points: points,
                          shift: try fields.optional("shift", JSONValue.bool) ?? false)
        if let settings = try fields.optional("settings", { value, at in try JSONFields(value, path: at) }) {
            let number = { (key: String) throws -> CGFloat? in try settings.optional(key, JSONValue.number).map { CGFloat($0) } }
            op.size = try number("size")
            op.hardness = try number("hardness")
            op.opacity = try number("opacity")
            op.smoothing = try number("smoothing")
            op.blurRadius = try number("blurRadius")
            if let rgb = try settings.optional("color", { try JSONValue.numbers($0, count: 3, $1) }) {
                op.color = PaletteColor(red: CGFloat(rgb[0]), green: CGFloat(rgb[1]), blue: CGFloat(rgb[2]))
            }
            op.white = try settings.optional("white", JSONValue.bool)
            op.healingMode = try settings.optional("healingMode") { try JSONValue.choice(SpotHealingMode.self, $0, $1) }
            op.aligned = try settings.optional("aligned", JSONValue.bool)
            op.sampleAll = try settings.optional("sampleAll", JSONValue.bool)
            try settings.rejectUnknown()
        }
        op.source = try fields.optional("source") { value, at -> CGPoint in
            let xy = try JSONValue.numbers(value, count: 2, at)
            return CGPoint(x: xy[0], y: xy[1])
        }
        if op.healingMode == .createTexture {
            // BrushStroke.heal() seeds Create Texture's grain with UInt32.random, which the harness can't reach.
            throw HarnessError("\(path): Create Texture draws a random seed in BrushStroke.heal(), so its result can't be reproduced")
        }
        return op
    }
}

extension EditorSession {
    /// Selects the target and tool, sets the options bar, then presses, drags and releases as the canvas does.
    func parityStroke(_ op: StrokeOp) async throws {
        guard let target = document?.layers.first(where: { $0.id == op.layer }) else {
            throw HarnessError("there's no layer \(op.layer.uuidString)")
        }
        guard !isProjectBusy, brushStroke == nil, warpStroke == nil else { throw HarnessError("the session is busy") }
        // Clicking the layer's thumbnail, or its mask's.
        selectLayerTarget(op.layer, mask: op.mask)
        guard activeLayerID == op.layer, isMaskSelected == op.mask else {
            throw HarnessError("layer “\(target.name)”\(op.mask ? "'s mask" : "") couldn't be selected")
        }
        switch op.tool {
        case .brush, .eraser:
            selectTool(.brush)
            brushMode = op.tool == .eraser ? .erase : .paint
        case .heal: selectTool(.spotHealing)
        case .clone: selectTool(.cloneStamp)
        case .blur, .smudge, .liquify:
            selectTool(.blur)
            blurMode = op.tool == .blur ? .blur : op.tool == .smudge ? .smudge : .liquify
        }
        var settings = brushSettings
        if let size = op.size { settings.diameter = size }
        if let hardness = op.hardness { settings.hardness = hardness }
        if let opacity = op.opacity { settings.opacity = opacity }
        if let smoothing = op.smoothing { settings.smoothing = smoothing }
        if let radius = op.blurRadius { settings.blurRadius = radius }
        brushSettings = settings
        if let color = op.color {
            guard !op.mask else { throw HarnessError("settings.color is for painting pixels; a mask takes settings.white") }
            foregroundColor = color
        }
        if let white = op.white {
            guard op.mask else { throw HarnessError("settings.white is for painting a mask") }
            maskPaintWhite = white
        }
        if let mode = op.healingMode { spotHealingMode = mode }
        if let aligned = op.aligned { cloneSettings.aligned = aligned }
        if let all = op.sampleAll { cloneSettings.sampleAllLayers = all }
        // Option-click with Clone Stamp.
        if let source = op.source { setCloneSource(source) }
        brushError = nil
        // mouseDown: a Shift-click paints a line on from the last stroke's end, when there is one for this target.
        if op.shift, let from = shiftLineStart() {
            beginBrush(at: from)
            continueBrush(at: op.points[0])
        } else {
            beginBrush(at: op.points[0])
        }
        if let message = brushError { cancelBrush(); throw HarnessError(message) }
        guard brushStroke != nil || warpStroke != nil else { throw HarnessError("the stroke didn't start") }
        // mouseDragged, once per point.
        for point in op.points.dropFirst() { continueBrush(at: point) }
        // mouseUp: the release point, then the commit.
        continueBrush(at: op.points[op.points.count - 1])
        guard finishBrushImmediately() else { cancelBrush(); throw HarnessError("the session was busy at mouse-up") }
        if let message = brushError { throw HarnessError(message) }
    }
}
