import CoreFoundation
import CoreGraphics
import Foundation

/// A corpus case.json. See parity/README.md.
struct CaseSpec {
    let feature: String?
    let label: String?
    /// Relative to the case folder.
    let input: String
    let ops: [ParityOp]

    static func load(from url: URL) throws -> CaseSpec {
        let data = try Data(contentsOf: url)
        let fields = try JSONFields(try JSONSerialization.jsonObject(with: data), path: "case.json")
        let input = try JSONValue.string(fields.required("input"), "case.json.input")
        guard !input.isEmpty, !input.hasPrefix("/") else { throw HarnessError("case.json.input must be a path inside the case folder") }
        let ops = try fields.optional("ops") { value, path in
            try JSONValue.array(value, path).enumerated().map { index, item in try ParityOp.parse(item, path: "\(path)[\(index)]") }
        } ?? []
        return CaseSpec(feature: try fields.optional("feature", JSONValue.string),
                        label: try fields.optional("label", JSONValue.string),
                        input: input, ops: ops)
    }
}

/// One editing operation from a case's `ops`, with its fields already checked and converted to the app's types.
enum ParityOp {
    /// Filter > (kind)… on `layer`, then OK. `seed` is set for Add Noise and Grain only (see `SeededFilter`).
    case filter(layer: UUID, kind: FilterKind, settings: FilterSettings, seed: UInt32?)
    /// Image > Canvas Size… with these options.
    case canvasSize(CanvasSizeOptions)
    /// The Crop tool's frame, then Apply Crop.
    case crop(CGRect)
    /// Image > Image Size… `resolution` nil keeps the document's, as the sheet starts with it.
    case imageSize(width: Int, height: Int, resolution: Double?, sampling: LayerSampling)

    var name: String {
        switch self {
        case .filter: "filter"
        case .canvasSize: "canvasSize"
        case .crop: "crop"
        case .imageSize: "imageSize"
        }
    }

    func apply(to session: EditorSession) async throws {
        switch self {
        case let .filter(layer, kind, settings, seed):
            try await session.parityApplyFilter(layer: layer, kind: kind, settings: settings, seed: seed)
        case let .canvasSize(options):
            try await session.parityCanvasSize(options)
        case let .crop(rect):
            try await session.parityCrop(rect)
        case let .imageSize(width, height, resolution, sampling):
            try await session.parityImageSize(width: width, height: height, resolution: resolution, sampling: sampling)
        }
    }

    static func parse(_ value: Any, path: String) throws -> ParityOp {
        let fields = try JSONFields(value, path: path)
        let op = try JSONValue.string(fields.required("op"), "\(path).op")
        let result: ParityOp
        switch op {
        case "filter":
            let layerText = try JSONValue.string(fields.required("layer"), "\(path).layer")
            guard let layer = UUID(uuidString: layerText) else { throw HarnessError("\(path).layer “\(layerText)” isn't a UUID") }
            let kind = try JSONValue.choice(FilterKind.self, fields.required("kind"), "\(path).kind")
            if kind == .cameraRaw {
                throw HarnessError("\(path).kind: Camera Raw Filter isn't supported by the harness; its settings have no JSON form")
            }
            let settings = try fields.optional("settings", FilterSettingsJSON.parse) ?? FilterSettings()
            var seed = try fields.optional("seed") { value, at -> UInt32 in
                let number = try JSONValue.integer(value, at)
                guard let seed = UInt32(exactly: number) else { throw HarnessError("\(at) must be 0–\(UInt32.max)") }
                return seed
            }
            if SeededFilter.kinds.contains(kind) {
                seed = seed ?? 0
            } else if seed != nil {
                throw HarnessError("\(path).seed is only for \(SeededFilter.kinds.map(\.rawValue).joined(separator: " and "))")
            }
            result = .filter(layer: layer, kind: kind, settings: settings, seed: seed)
        case "canvasSize":
            var options = CanvasSizeOptions(width: try JSONValue.integer(fields.required("width"), "\(path).width"),
                                            height: try JSONValue.integer(fields.required("height"), "\(path).height"))
            if let anchor = try fields.optional("anchor", JSONValue.integer) {
                guard (0...8).contains(anchor) else { throw HarnessError("\(path).anchor must be 0–8") }
                options.anchor = anchor
            }
            if let fill = try fields.optional("fill", { try JSONValue.numbers($0, count: 3, $1) }) {
                options.fill = CanvasExtensionColor(red: CGFloat(fill[0]), green: CGFloat(fill[1]), blue: CGFloat(fill[2]))
            }
            if let offset = try fields.optional("contentOffset", { try JSONValue.numbers($0, count: 2, $1) }) {
                options.contentOffset = CGPoint(x: offset[0], y: offset[1])
            }
            result = .canvasSize(options)
        case "crop":
            let rect = try JSONValue.numbers(fields.required("rect"), count: 4, "\(path).rect")
            guard rect.allSatisfy({ $0.rounded() == $0 }) else { throw HarnessError("\(path).rect must be whole document pixels") }
            result = .crop(CGRect(x: rect[0], y: rect[1], width: rect[2], height: rect[3]))
        case "imageSize":
            result = .imageSize(width: try JSONValue.integer(fields.required("width"), "\(path).width"),
                                height: try JSONValue.integer(fields.required("height"), "\(path).height"),
                                resolution: try fields.optional("resolution", JSONValue.number),
                                sampling: try fields.optional("sampling") { try JSONValue.choice(LayerSampling.self, $0, $1) } ?? .high)
        default:
            throw HarnessError("\(path).op “\(op)” isn't one of filter, canvasSize, crop, imageSize")
        }
        try fields.rejectUnknown()
        return result
    }
}

/// `FilterSettings` from JSON: only the fields present change, the rest keep `FilterSettings()`'s defaults. Field
/// names are the Swift property names; nested settings can be partial too.
nonisolated enum FilterSettingsJSON {
    static func parse(_ value: Any, path: String) throws -> FilterSettings {
        let object = try JSONValue.object(value, path)
        var settings = FilterSettings()
        let numbers: [String: WritableKeyPath<FilterSettings, Double>] = [
            "radius": \.radius, "angle": \.angle, "distance": \.distance, "amount": \.amount,
            "vignetteAmount": \.vignetteAmount, "vignetteMidpoint": \.vignetteMidpoint,
            "vignetteRoundness": \.vignetteRoundness, "vignetteFeather": \.vignetteFeather,
            "vignetteHighlights": \.vignetteHighlights,
            "bloomAmount": \.bloomAmount, "bloomRadius": \.bloomRadius,
            "tonalAmount": \.tonalAmount, "tonalRadius": \.tonalRadius, "tonalShadows": \.tonalShadows,
            "tonalMidtones": \.tonalMidtones, "tonalHighlights": \.tonalHighlights,
            "distortion": \.distortion,
            "refineEdges": \.refineEdges, "matteContrast": \.matteContrast, "shiftEdge": \.shiftEdge,
        ]
        let flags: [String: WritableKeyPath<FilterSettings, Bool>] = ["gaussian": \.gaussian, "monochromatic": \.monochromatic]
        for key in object.keys.sorted() {
            guard let item = object[key] else { continue }
            let at = "\(path).\(key)"
            if let keyPath = numbers[key] {
                settings[keyPath: keyPath] = try JSONValue.number(item, at)
                continue
            }
            if let keyPath = flags[key] {
                settings[keyPath: keyPath] = try JSONValue.bool(item, at)
                continue
            }
            switch key {
            case "vignetteColor": settings.vignetteColor = try JSONValue.patched(settings.vignetteColor, with: item, at)
            case "curves": settings.curves = try JSONValue.patched(settings.curves, with: item, at)
            case "exposure": settings.exposure = try JSONValue.patched(settings.exposure, with: item, at)
            case "gradientMap": settings.gradientMap = try JSONValue.patched(settings.gradientMap, with: item, at)
            case "grain": settings.grain = try JSONValue.patched(settings.grain, with: item, at)
            case "blackWhite": settings.blackWhite = try JSONValue.patched(settings.blackWhite, with: item, at)
            case "colorBalance": settings.colorBalance = try JSONValue.patched(settings.colorBalance, with: item, at)
            case "dither": settings.dither = try dither(item, at)
            case "backgroundQuality": settings.backgroundQuality = try JSONValue.choice(BackgroundQuality.self, item, at)
            case "cameraRaw": throw HarnessError("\(at): Camera Raw settings aren't supported by the harness")
            default: throw HarnessError("\(at) isn't a FilterSettings field")
            }
        }
        return settings
    }

    /// `DitherSettings` isn't Codable, so its fields are read one by one.
    private static func dither(_ value: Any, _ path: String) throws -> DitherSettings {
        let object = try JSONValue.object(value, path)
        var settings = DitherSettings()
        let numbers: [String: WritableKeyPath<DitherSettings, Double>] = [
            "pixelSize": \.pixelSize, "cellSize": \.cellSize, "textSize": \.textSize, "lineSpacing": \.lineSpacing,
            "glow": \.glow, "dots": \.dots, "wobble": \.wobble, "angle": \.angle, "levels": \.levels,
            "diffusion": \.diffusion, "density": \.density, "contrast": \.contrast,
        ]
        for key in object.keys.sorted() {
            guard let item = object[key] else { continue }
            let at = "\(path).\(key)"
            if let keyPath = numbers[key] {
                settings[keyPath: keyPath] = try JSONValue.number(item, at)
                continue
            }
            switch key {
            case "style": settings.style = try JSONValue.choice(DitherStyle.self, item, at)
            case "pixelShape": settings.pixelShape = try JSONValue.choice(DitherPixelShape.self, item, at)
            case "colors": settings.colors = try JSONValue.choice(DitherColors.self, item, at)
            case "dark": settings.dark = try JSONValue.patched(settings.dark, with: item, at)
            case "light": settings.light = try JSONValue.patched(settings.light, with: item, at)
            case "lightOnDark": settings.lightOnDark = try JSONValue.bool(item, at)
            case "characters": settings.characters = try JSONValue.string(item, at)
            default: throw HarnessError("\(at) isn't a DitherSettings field")
            }
        }
        return settings
    }
}

/// A JSON object whose fields are read by name; `rejectUnknown` then refuses any it didn't read, so a misspelt
/// field fails the case instead of being ignored.
final class JSONFields {
    let path: String
    private let values: [String: Any]
    private var read: Set<String> = []

    init(_ value: Any, path: String) throws {
        self.values = try JSONValue.object(value, path)
        self.path = path
    }

    func required(_ key: String) throws -> Any {
        read.insert(key)
        guard let value = values[key], !(value is NSNull) else { throw HarnessError("\(path).\(key) is missing") }
        return value
    }

    func optional<T>(_ key: String, _ convert: (Any, String) throws -> T) throws -> T? {
        read.insert(key)
        guard let value = values[key], !(value is NSNull) else { return nil }
        return try convert(value, "\(path).\(key)")
    }

    func rejectUnknown() throws {
        let unknown = Set(values.keys).subtracting(read)
        guard unknown.isEmpty else {
            throw HarnessError("\(path) has unknown field\(unknown.count == 1 ? "" : "s") \(unknown.sorted().joined(separator: ", "))")
        }
    }
}

nonisolated enum JSONValue {
    private static func isBoolean(_ number: NSNumber) -> Bool {
        CFGetTypeID(number) == CFBooleanGetTypeID()
    }

    static func object(_ value: Any, _ path: String) throws -> [String: Any] {
        guard let object = value as? [String: Any] else { throw HarnessError("\(path) must be a JSON object") }
        return object
    }

    static func array(_ value: Any, _ path: String) throws -> [Any] {
        guard let array = value as? [Any] else { throw HarnessError("\(path) must be a JSON array") }
        return array
    }

    static func string(_ value: Any, _ path: String) throws -> String {
        guard let string = value as? String else { throw HarnessError("\(path) must be a string") }
        return string
    }

    static func number(_ value: Any, _ path: String) throws -> Double {
        guard let number = value as? NSNumber, !isBoolean(number), number.doubleValue.isFinite else {
            throw HarnessError("\(path) must be a number")
        }
        return number.doubleValue
    }

    static func integer(_ value: Any, _ path: String) throws -> Int {
        let whole = try number(value, path)
        guard whole.rounded() == whole, abs(whole) <= 1e15 else { throw HarnessError("\(path) must be a whole number") }
        return Int(whole)
    }

    static func bool(_ value: Any, _ path: String) throws -> Bool {
        guard let number = value as? NSNumber, isBoolean(number) else { throw HarnessError("\(path) must be true or false") }
        return number.boolValue
    }

    static func numbers(_ value: Any, count: Int, _ path: String) throws -> [Double] {
        let items = try array(value, path)
        guard items.count == count else { throw HarnessError("\(path) must hold \(count) numbers") }
        return try items.enumerated().map { index, item in try number(item, "\(path)[\(index)]") }
    }

    static func choice<E: RawRepresentable & CaseIterable>(_ type: E.Type, _ value: Any, _ path: String) throws -> E
        where E.RawValue == String {
        let raw = try string(value, path)
        guard let result = E(rawValue: raw) else {
            let names = E.allCases.map { "“\($0.rawValue)”" }.joined(separator: ", ")
            throw HarnessError("\(path) “\(raw)” isn't one of \(names)")
        }
        return result
    }

    /// `base` with only the fields in `value` changed: `base` is encoded, `value` laid over it field by field
    /// (nested objects likewise, arrays replaced whole), and the result decoded with the type's own Codable.
    static func patched<T: Codable>(_ base: T, with value: Any, _ path: String) throws -> T {
        let patch = try object(value, path)
        let encoded = try JSONEncoder().encode(base)
        guard let original = try JSONSerialization.jsonObject(with: encoded) as? [String: Any] else {
            throw HarnessError("\(path) can't be set field by field")
        }
        let data = try JSONSerialization.data(withJSONObject: try merge(original, patch, path))
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw HarnessError("\(path): \(describe(error))")
        }
    }

    private static func merge(_ base: [String: Any], _ patch: [String: Any], _ path: String) throws -> [String: Any] {
        var result = base
        for (key, value) in patch {
            guard let current = base[key] else {
                throw HarnessError("\(path).\(key) isn't a field here; the fields are \(base.keys.sorted().joined(separator: ", "))")
            }
            if let inner = current as? [String: Any], let innerPatch = value as? [String: Any] {
                result[key] = try merge(inner, innerPatch, "\(path).\(key)")
            } else {
                result[key] = value
            }
        }
        return result
    }
}
