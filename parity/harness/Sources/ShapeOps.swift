import CoreGraphics
import Foundation

/// The Shape and Gradient tools' ops and resizing a layer (see parity/README.md), each driven through the
/// `EditorSession` functions the canvas and the options bars call, with points already in document pixels.
enum ShapeOp {
    /// The Shape tool dragged from `from` to `to`: `beginShape`, `dragShape` (Shift is `square`, Option `fromCenter`),
    /// `finishShape`, with the options bar set to `kind`, `cornerRadius` and `lineWidth` and the Fill swatch to `color`.
    case shape(kind: ShapeKind, from: CGPoint, to: CGPoint, square: Bool, fromCenter: Bool, color: PaletteColor?,
               cornerRadius: Double?, lineWidth: Double?, layer: String?)
    /// The Gradient tool dragged from `from` to `to` on `layer` (or its mask), then Apply: `beginGradient`,
    /// `moveGradient`, `endGradientDrag`, `commitGradient`.
    case gradient(from: CGPoint, to: CGPoint, settings: GradientSettings, foreground: PaletteColor?,
                  background: PaletteColor?, layer: String?, mask: Bool)
    /// Move / Transform: the layer's box set to `rect` (rotation and flips kept), then applied: `beginTransform`,
    /// `previewTransform`, `commitTransform`, which draws a shape layer's shape again at its new size.
    case resizeLayer(layer: String?, rect: CGRect)

    static let names = ["shape", "gradient", "resizeLayer"]

    static func parse(_ op: String, _ fields: JSONFields, path: String) throws -> ShapeOp {
        func point(_ key: String) throws -> CGPoint {
            let p = try JSONValue.numbers(fields.required(key), count: 2, "\(path).\(key)")
            return CGPoint(x: p[0], y: p[1])
        }
        func color(_ key: String) throws -> PaletteColor? {
            try fields.optional(key) { value, at -> PaletteColor in
                let c = try JSONValue.numbers(value, count: 3, at)
                guard c.allSatisfy({ (0...1).contains($0) }) else { throw HarnessError("\(at) must hold numbers 0–1") }
                return PaletteColor(red: CGFloat(c[0]), green: CGFloat(c[1]), blue: CGFloat(c[2]))
            }
        }
        let layer = try fields.optional("layer", JSONValue.string)
        switch op {
        case "shape":
            let kind = try JSONValue.choice(ShapeKind.self, fields.required("kind"), "\(path).kind")
            let radius = try fields.optional("cornerRadius", JSONValue.number)
            if let radius, !(0...5000).contains(radius) { throw HarnessError("\(path).cornerRadius must be 0–5000, as the options bar allows") }
            let width = try fields.optional("lineWidth", JSONValue.number)
            if let width, !(1...5000).contains(width) { throw HarnessError("\(path).lineWidth must be 1–5000, as the options bar allows") }
            return .shape(kind: kind, from: try point("from"), to: try point("to"),
                          square: try fields.optional("square", JSONValue.bool) ?? false,
                          fromCenter: try fields.optional("fromCenter", JSONValue.bool) ?? false,
                          color: try color("color"), cornerRadius: radius, lineWidth: width, layer: layer)
        case "gradient":
            var settings = GradientSettings()
            if let shape = try fields.optional("type", { try JSONValue.choice(GradientShape.self, $0, $1) }) { settings.shape = shape }
            if let style = try fields.optional("style", { try JSONValue.choice(GradientStyle.self, $0, $1) }) { settings.style = style }
            settings.reversed = try fields.optional("reversed", JSONValue.bool) ?? false
            if let opacity = try fields.optional("opacity", JSONValue.number) {
                guard (0.01...1).contains(opacity) else { throw HarnessError("\(path).opacity must be 0.01–1, as the options bar allows") }
                settings.opacity = CGFloat(opacity)
            }
            return .gradient(from: try point("from"), to: try point("to"), settings: settings,
                             foreground: try color("foreground"), background: try color("background"), layer: layer,
                             mask: try fields.optional("mask", JSONValue.bool) ?? false)
        case "resizeLayer":
            let rect = try JSONValue.numbers(fields.required("rect"), count: 4, "\(path).rect")
            return .resizeLayer(layer: layer, rect: CGRect(x: rect[0], y: rect[1], width: rect[2], height: rect[3]))
        default:
            throw HarnessError("\(path).op “\(op)” isn't a shape op")
        }
    }

    func apply(to session: EditorSession) async throws {
        guard session.document != nil else { throw HarnessError("there's no document") }
        session.brushError = nil
        switch self {
        case let .shape(kind, from, to, square, fromCenter, color, cornerRadius, lineWidth, layer):
            try session.parityShapeTool(.shape)
            if let layer { try session.parityShapeActivate(layer, mask: false) }
            // The options bar's segmented control cancels any shape in progress before it switches.
            session.cancelShape()
            session.shapeKind = kind
            if let cornerRadius { session.shapeCornerRadius = cornerRadius }
            if let lineWidth { session.shapeLineWidth = lineWidth }
            if let color { session.setPaletteColor(color, background: false) }
            let count = session.document?.layers.count ?? 0
            session.beginShape(at: from)
            guard session.shapeDraft != nil else { throw HarnessError("the Shape tool didn't start a shape") }
            session.dragShape(to: to, square: square, fromCenter: fromCenter)
            session.finishShape()
            if let message = session.brushError { throw HarnessError(message) }
            guard session.document?.layers.count == count + 1 else { throw HarnessError("the drag made no shape layer") }
        case let .gradient(from, to, settings, foreground, background, layer, mask):
            try session.parityShapeTool(.gradient)
            if let layer {
                try session.parityShapeActivate(layer, mask: mask)
            } else if mask, let active = session.activeLayerID {
                session.selectLayerTarget(active, mask: true)
            }
            guard session.isMaskSelected == mask else { throw HarnessError(mask ? "the layer has no mask to target" : "the mask stayed targeted") }
            if let foreground { session.setPaletteColor(foreground, background: false) }
            if let background { session.setPaletteColor(background, background: true) }
            session.gradientSettings = settings
            session.beginGradient(at: from)
            guard let edit = session.gradientEdit else {
                throw HarnessError(session.brushError ?? "the Gradient tool didn't start on the active layer")
            }
            session.moveGradient(end: to)
            session.endGradientDrag()
            guard session.gradientEdit === edit else { throw HarnessError("the drag was too short to leave a gradient") }
            // Apply (or Return), as the options bar does while a gradient is pending.
            await session.commitGradient()
            if let message = session.brushError { throw HarnessError(message) }
            guard session.gradientEdit == nil else { throw HarnessError("Apply didn't apply the gradient") }
        case let .resizeLayer(layer, rect):
            try session.parityShapeTool(.move)
            if let layer { try session.parityShapeActivate(layer, mask: false) }
            guard let id = session.activeLayerID,
                  let original = session.document?.layers.first(where: { $0.id == id })?.transform else {
                throw HarnessError("there's no active layer to resize")
            }
            var draft = original
            draft.origin = rect.origin
            draft.size = rect.size
            guard draft.isValid else { throw HarnessError("the box \(rect) is outside the app's limits") }
            session.beginTransform()
            guard session.transformEdit != nil else { throw HarnessError("the active layer can't be transformed") }
            session.previewTransform(draft)
            session.commitTransform()
            guard session.transformEdit == nil, session.document?.layers.first(where: { $0.id == id })?.transform == draft else {
                throw HarnessError("the transform wasn't applied")
            }
        }
    }
}

extension EditorSession {
    /// Picks `tool` from the tool rail, which first applies or cancels whatever the current tool had in progress.
    fileprivate func parityShapeTool(_ value: NavigationTool) throws {
        selectTool(value)
        guard tool == value else { throw HarnessError("the \(value.rawValue) tool couldn't be picked") }
    }

    /// Clicks a layer's row (or its mask thumbnail) in the Layers panel. `reference` is the layer's UUID, or its name
    /// for a layer an earlier op made (its UUID is new each run).
    fileprivate func parityShapeActivate(_ reference: String, mask: Bool) throws {
        let layers = document?.layers ?? []
        let found = UUID(uuidString: reference).flatMap { id in layers.first { $0.id == id } }
            ?? layers.last { $0.name == reference }
        guard let layer = found else { throw HarnessError("there's no layer “\(reference)”") }
        selectLayerTarget(layer.id, mask: mask)
        guard activeLayerID == layer.id else { throw HarnessError("layer “\(layer.name)” couldn't be selected") }
    }
}
