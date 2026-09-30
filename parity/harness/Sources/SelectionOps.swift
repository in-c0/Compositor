import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// The selection ops (see parity/README.md), each driven through the `EditorSession` functions the canvas, the
/// options bar, the Select menu and the Layers panel call.
enum SelectionOp {
    /// The Marquee dragged from `from` to `to`, `square` as Shift held during the drag: `beginLasso`, `dragMarquee`
    /// (never from the center: the canvas uses Option to subtract), `finishLasso`.
    case marquee(kind: LassoKind, from: CGPoint, to: CGPoint, square: Bool, mode: SelectionMode, antialias: Bool)
    /// The Lasso through `points`: `beginLasso` at the first, `extendLasso` to each of the rest, `finishLasso`.
    case lasso(kind: LassoKind, points: [CGPoint], mode: SelectionMode, antialias: Bool)
    /// A Magic Wand click at `point` with `layer` active.
    case wand(layer: UUID?, point: CGPoint, mode: SelectionMode, settings: WandSettings, antialias: Bool)
    /// An Object Selection click at `point` with `layer` active.
    case object(layer: UUID?, point: CGPoint, mode: SelectionMode, settings: ObjectSelectionSettings, antialias: Bool)
    /// Select > Subject. The menu always replaces the selection; `mode` reaches the function's own parameter.
    case subject(mode: SelectionMode, antialias: Bool)
    /// Select > Color Range…: the settings, then a click per sample with that eyedropper, then OK.
    case colorRange(samples: [(point: CGPoint, mode: HueSampleMode)], fuzziness: Double, invert: Bool, antialias: Bool)
    case selectAll, deselect, invertSelection
    /// Select > Modify > Expand, Contract or Feather with this amount.
    case modify(kind: String, amount: Int)
    /// Cmd-click on a layer's thumbnail (its pixels) or its mask's thumbnail (the mask's black areas).
    case load(layer: UUID, mask: Bool, mode: SelectionMode)

    static let names = ["marquee", "lasso", "wand", "objectSelection", "selectSubject", "colorRange", "selectAll",
                        "deselect", "invertSelection", "modifySelection", "loadSelection"]

    static func parse(_ op: String, _ fields: JSONFields, path: String) throws -> SelectionOp {
        func point(_ key: String) throws -> CGPoint {
            let p = try JSONValue.numbers(fields.required(key), count: 2, "\(path).\(key)")
            return CGPoint(x: p[0], y: p[1])
        }
        func mode() throws -> SelectionMode {
            try fields.optional("mode") { try JSONValue.choice(SelectionMode.self, $0, $1) } ?? .replace
        }
        func antialias() throws -> Bool { try fields.optional("antialias", JSONValue.bool) ?? true }
        func layer(_ key: String) throws -> UUID? {
            try fields.optional(key) { (value: Any, at: String) throws -> UUID in
                let text = try JSONValue.string(value, at)
                guard let id = UUID(uuidString: text) else { throw HarnessError("\(at) “\(text)” isn't a UUID") }
                return id
            }
        }
        switch op {
        case "marquee":
            let kind = try fields.optional("shape") { try JSONValue.choice(LassoKind.self, $0, $1) } ?? .rectangle
            guard LassoKind.marqueeChoices.contains(kind) else { throw HarnessError("\(path).shape must be Rectangle or Ellipse") }
            return .marquee(kind: kind, from: try point("from"), to: try point("to"),
                            square: try fields.optional("square", JSONValue.bool) ?? false,
                            mode: try mode(), antialias: try antialias())
        case "lasso":
            let kind = try fields.optional("kind") { try JSONValue.choice(LassoKind.self, $0, $1) } ?? .freehand
            guard LassoKind.lassoChoices.contains(kind) else { throw HarnessError("\(path).kind must be Freehand or Polygonal") }
            let items = try JSONValue.array(fields.required("points"), "\(path).points")
            guard !items.isEmpty else { throw HarnessError("\(path).points is empty") }
            let points = try items.enumerated().map { index, item -> CGPoint in
                let p = try JSONValue.numbers(item, count: 2, "\(path).points[\(index)]")
                return CGPoint(x: p[0], y: p[1])
            }
            return .lasso(kind: kind, points: points, mode: try mode(), antialias: try antialias())
        case "wand":
            var settings = WandSettings()
            if let tolerance = try fields.optional("tolerance", JSONValue.integer) {
                guard (0...255).contains(tolerance) else { throw HarnessError("\(path).tolerance must be 0–255") }
                settings.tolerance = tolerance
            }
            if let size = try fields.optional("sampleSize", JSONValue.string) {
                guard let match = WandSampleSize.allCases.first(where: { $0.title == size }) else {
                    throw HarnessError("\(path).sampleSize “\(size)” isn't one of \(WandSampleSize.allCases.map { "“\($0.title)”" }.joined(separator: ", "))")
                }
                settings.sampleSize = match
            }
            settings.contiguous = try fields.optional("contiguous", JSONValue.bool) ?? true
            settings.sampleAllLayers = try fields.optional("sampleAllLayers", JSONValue.bool) ?? false
            return .wand(layer: try layer("layer"), point: try point("point"), mode: try mode(), settings: settings,
                         antialias: try antialias())
        case "objectSelection":
            var settings = ObjectSelectionSettings()
            settings.sampleAllLayers = try fields.optional("sampleAllLayers", JSONValue.bool) ?? true
            if let edge = try fields.optional("edge", JSONValue.integer) {
                guard (-10...10).contains(edge) else { throw HarnessError("\(path).edge must be -10–10") }
                settings.edgeOffset = edge
            }
            return .object(layer: try layer("layer"), point: try point("point"), mode: try mode(), settings: settings,
                           antialias: try antialias())
        case "selectSubject":
            return .subject(mode: try mode(), antialias: try antialias())
        case "colorRange":
            let items = try JSONValue.array(fields.required("samples"), "\(path).samples")
            let samples = try items.enumerated().map { index, item -> (point: CGPoint, mode: HueSampleMode) in
                let at = "\(path).samples[\(index)]"
                let sample = try JSONFields(item, path: at)
                let p = try JSONValue.numbers(sample.required("point"), count: 2, "\(at).point")
                let mode = try sample.optional("mode") { try JSONValue.choice(HueSampleMode.self, $0, $1) } ?? .replace
                try sample.rejectUnknown()
                return (CGPoint(x: p[0], y: p[1]), mode)
            }
            let fuzziness = try fields.optional("fuzziness", JSONValue.number) ?? 40
            guard ColorRangeEdit.fuzzinessRange.contains(fuzziness) else { throw HarnessError("\(path).fuzziness must be 0–200") }
            return .colorRange(samples: samples, fuzziness: fuzziness.rounded(),
                               invert: try fields.optional("invert", JSONValue.bool) ?? false, antialias: try antialias())
        case "selectAll": return .selectAll
        case "deselect": return .deselect
        case "invertSelection": return .invertSelection
        case "modifySelection":
            let kinds = ["expand", "contract", "feather"].filter { (try? fields.optional($0, JSONValue.integer)) != nil }
            guard kinds.count == 1, let kind = kinds.first else { throw HarnessError("\(path) needs exactly one of expand, contract, feather") }
            let amount = try JSONValue.integer(fields.required(kind), "\(path).\(kind)")
            guard (1...(kind == "feather" ? 250 : 500)).contains(amount) else {
                throw HarnessError("\(path).\(kind) must be 1–\(kind == "feather" ? 250 : 500), as the amount sheet allows")
            }
            return .modify(kind: kind, amount: amount)
        case "loadSelection":
            guard let id = try layer("layer") else { throw HarnessError("\(path).layer is missing") }
            return .load(layer: id, mask: try fields.optional("mask", JSONValue.bool) ?? false, mode: try mode())
        default:
            throw HarnessError("\(path).op “\(op)” isn't a selection op")
        }
    }

    func apply(to session: EditorSession) async throws {
        guard session.document != nil else { throw HarnessError("there's no document") }
        session.brushError = nil
        switch self {
        case let .marquee(kind, from, to, square, mode, antialias):
            try session.parityChooseTool(.marquee)
            session.cancelLasso()
            session.marqueeKind = kind
            session.selectionAntialiased = antialias
            session.beginLasso(at: from, mode: mode)
            guard session.lassoDraft != nil else { throw HarnessError("the Marquee didn't start") }
            session.dragMarquee(to: to, square: square, fromCenter: false)
            session.finishLasso()
        case let .lasso(kind, points, mode, antialias):
            try session.parityChooseTool(.lasso)
            session.cancelLasso()
            session.lassoKind = kind
            session.selectionAntialiased = antialias
            session.beginLasso(at: points[0], mode: mode)
            guard session.lassoDraft != nil else { throw HarnessError("the Lasso didn't start") }
            for point in points.dropFirst() { session.extendLasso(to: point) }
            session.finishLasso()
        case let .wand(layer, point, mode, settings, antialias):
            try session.parityChooseTool(.wand)
            try session.parityActivate(layer)
            session.wandMode = .wand
            session.wandSettings = settings
            session.selectionAntialiased = antialias
            await session.magicWand(at: point, mode: mode)
        case let .object(layer, point, mode, settings, antialias):
            try session.parityChooseTool(.wand)
            try session.parityActivate(layer)
            session.wandMode = .object
            session.objectSelectionSettings = settings
            session.selectionAntialiased = antialias
            await session.selectObject(at: point, mode: mode)
        case let .subject(mode, antialias):
            session.selectionAntialiased = antialias
            guard session.canSelectSubject else { throw HarnessError("Select > Subject isn't available") }
            await session.selectSubject(mode: mode)
        case let .colorRange(samples, fuzziness, invert, antialias):
            session.selectionAntialiased = antialias
            try await session.parityColorRange(samples: samples, fuzziness: fuzziness, invert: invert)
        case .selectAll: session.selectAll()
        case .deselect: session.deselect()
        case .invertSelection: session.invertSelection()
        case let .modify(kind, amount):
            guard session.canModifySelection else { throw HarnessError("Modify needs a selection that isn't empty") }
            switch kind {
            case "expand": session.expandSelection(by: amount)
            case "contract": session.contractSelection(by: amount)
            default: session.featherSelection(by: amount)
            }
        case let .load(layer, mask, mode):
            guard let target = session.document?.layers.first(where: { $0.id == layer }) else {
                throw HarnessError("there's no layer \(layer.uuidString)")
            }
            if mask {
                guard target.mask != nil else { throw HarnessError("layer “\(target.name)” has no mask") }
                session.loadMaskSelection(layerID: layer, mode: mode)
            } else {
                session.loadLayerSelection(layerID: layer, mode: mode)
            }
        }
        if let message = session.brushError { throw HarnessError(message) }
    }
}

extension EditorSession {
    /// Picking a tool in the tool rail.
    func parityChooseTool(_ value: NavigationTool) throws {
        selectTool(value)
        guard tool == value else { throw HarnessError("the \(value.rawValue) tool couldn't be chosen") }
    }

    /// Clicking a layer in the Layers panel, when the case names one.
    func parityActivate(_ layer: UUID?) throws {
        guard let layer else { return }
        guard document?.layers.contains(where: { $0.id == layer }) == true else { throw HarnessError("there's no layer \(layer.uuidString)") }
        selectLayer(layer)
        guard activeLayerID == layer else { throw HarnessError("layer \(layer.uuidString) couldn't be selected") }
    }

    /// Select > Color Range…: Fuzziness and Invert set, a click on the image per sample with that eyedropper chosen,
    /// then OK once the panel shows the last result. The panel works out each result off the main thread and drops
    /// all but the newest, so the harness waits for that one as a person would before pressing OK.
    func parityColorRange(samples: [(point: CGPoint, mode: HueSampleMode)], fuzziness: Double, invert: Bool) async throws {
        beginColorRange()
        guard let edit = colorRange else { throw HarnessError("Color Range couldn't open") }
        edit.fuzziness = fuzziness
        edit.invert = invert
        for sample in samples {
            edit.sampleMode = sample.mode
            sampleColorRange(at: sample.point, shift: false, option: false)
        }
        if edit.hasColors {
            let deadline = Date().addingTimeInterval(60)
            while edit.preview == nil && edit.error == nil {
                guard Date() < deadline else { cancelColorRange(); throw HarnessError("Color Range didn't finish") }
                try await Task.sleep(for: .milliseconds(5))
            }
        }
        if let message = edit.error { cancelColorRange(); throw HarnessError(message) }
        commitColorRange()
    }
}

/// `<case>.selection.png`: the session's selection as `DocumentSelection.coverage` draws it at document size,
/// 8-bit gray, white where selected.
nonisolated enum SelectionCoverageFile {
    static func write(_ selection: DocumentSelection, width: Int, height: Int, to url: URL) throws {
        let image = try selection.coverage(width: width, height: height)
        // Copy the bytes into a known layout (one byte per pixel, rows packed), so the file holds the coverage
        // exactly as computed.
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width,
                                      space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue),
              let target = context.data else { throw HarnessError("couldn't make the coverage bitmap") }
        if image.bitsPerPixel == 8, image.bitsPerComponent == 8, image.colorSpace?.model == .monochrome,
           image.alphaInfo == .none, let data = image.dataProvider?.data, let bytes = CFDataGetBytePtr(data),
           CFDataGetLength(data) >= image.bytesPerRow * (height - 1) + width {
            for y in 0..<height { memcpy(target + y * width, bytes + y * image.bytesPerRow, width) }
        } else {
            context.interpolationQuality = .none
            context.setBlendMode(.copy)
            context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        }
        guard let copy = context.makeImage(),
              let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
        else { throw HarnessError("couldn't write \(url.lastPathComponent)") }
        CGImageDestinationAddImage(destination, copy, nil)
        guard CGImageDestinationFinalize(destination) else { throw HarnessError("couldn't write \(url.lastPathComponent)") }
    }
}
