import AppKit
import CoreText
import Foundation

/// `LayerTextStyle` from JSON: only the fields present change. Field names are the Swift property names, as the
/// project file writes them; `null` clears an optional field (`boxSize`, `colorRuns`, `fontRuns`).
nonisolated enum TextStyleJSON {
    static let fields: Set<String> = [
        "content", "fontName", "fontSize", "red", "green", "blue", "alignment", "tracking", "leading", "boxSize",
        "colorRuns", "fontRuns",
    ]

    /// Checks `value` is an object of `LayerTextStyle` fields, leaving out `excluded`.
    static func check(_ value: Any, excluding excluded: Set<String> = [], _ path: String) throws -> [String: Any] {
        let object = try JSONValue.object(value, path)
        for key in object.keys.sorted() where !fields.contains(key) || excluded.contains(key) {
            let allowed = fields.subtracting(excluded).sorted().joined(separator: ", ")
            throw HarnessError("\(path).\(key) isn't a text style field here; the fields are \(allowed)")
        }
        return object
    }

    static func patched(_ base: LayerTextStyle, with patch: [String: Any], _ path: String) throws -> LayerTextStyle {
        let encoded = try JSONEncoder().encode(base)
        guard var object = try JSONSerialization.jsonObject(with: encoded) as? [String: Any] else {
            throw HarnessError("\(path) can't be set field by field")
        }
        for (key, value) in patch {
            object[key] = value is NSNull ? nil : value
        }
        let data = try JSONSerialization.data(withJSONObject: object)
        do {
            return try JSONDecoder().decode(LayerTextStyle.self, from: data)
        } catch {
            throw HarnessError("\(path): \(describe(error))")
        }
    }
}

/// Notes the text ops leave for harness-info.json: how the app laid the text out, so the port's layout can be
/// checked against it directly rather than only through pixels.
@MainActor var parityTextNotes: [String] = []

extension EditorSession {
    /// The Type tool: its controls set to `style` (face, size, alignment, tracking, leading, and the foreground color),
    /// then a click at `point` for point text or a box dragged out as `rect`, `style.content` typed, the runs' letters
    /// selected and recolored or set in their face, and the text committed.
    func parityText(point: CGPoint?, rect: CGRect?, style patch: [String: Any], path: String) throws {
        guard document != nil else { throw HarnessError("there's no document to type in") }
        let style = try TextStyleJSON.patched(LayerTextStyle(), with: patch, path)
        guard style.isValid else { throw HarnessError("\(path): the text style is outside the app's limits") }
        var defaults = style
        defaults.content = LayerTextStyle().content
        defaults.colorRuns = nil
        defaults.fontRuns = nil
        defaults.boxSize = nil
        textDefaults = defaults
        foregroundColor = PaletteColor(red: style.red, green: style.green, blue: style.blue)
        brushError = nil
        if let rect {
            beginText(in: rect)
        } else if let point {
            beginText(at: point, newLayer: true)
        }
        guard var draft = textDraft else { throw HarnessError(brushError ?? "the Type tool didn't start") }
        draft.style.content = style.content
        draft.style.colorRuns = style.colorRuns
        draft.style.fontRuns = style.fontRuns
        try parityCommit(draft)
    }

    /// A text layer opened for editing (Layer > Edit Text), `style`'s fields changed, then committed.
    func parityEditText(layer id: UUID, style patch: [String: Any], path: String) throws {
        guard let layer = document?.layers.first(where: { $0.id == id }) else { throw HarnessError("there's no layer \(id.uuidString)") }
        guard layer.liveText != nil else { throw HarnessError("layer “\(layer.name)” isn't editable text") }
        selectLayer(id)
        guard activeLayerID == id else { throw HarnessError("layer “\(layer.name)” couldn't be selected") }
        editActiveText()
        guard var draft = textDraft else { throw HarnessError("layer “\(layer.name)” couldn't be opened for editing") }
        draft.style = try TextStyleJSON.patched(draft.style, with: patch, path)
        try parityCommit(draft)
    }

    private func parityCommit(_ draft: TextDraft) throws {
        guard draft.style.isValid else {
            cancelText()
            throw HarnessError("the text style is outside the app's limits")
        }
        brushError = nil
        guard applyText(draft), textDraft == nil else {
            let message = brushError ?? "the text couldn't be committed"
            cancelText()
            throw HarnessError(message)
        }
        parityTextNotes.append(Self.parityLayoutNote(draft.style))
    }

    /// The layout `textImage` draws, as JSON: the fonts, the box, each line fragment and each glyph's position.
    static func parityLayoutNote(_ style: LayerTextStyle) -> String {
        let string = attributedText(style)
        let padding = LayerTextStyle.padding
        let size = textBoxSize(style)
        let width = ceil(size.width), height = ceil(size.height)
        let storage = NSTextStorage(attributedString: string)
        let layout = NSLayoutManager()
        let container = NSTextContainer(size: CGSize(width: max(1, width - 2 * padding), height: max(1, height - 2 * padding)))
        container.lineFragmentPadding = 0
        storage.addLayoutManager(layout)
        layout.addTextContainer(container)
        let glyphs = layout.glyphRange(for: container)
        let measured = string.boundingRect(with: CGSize(width: 100_000, height: 100_000), options: [.usesLineFragmentOrigin, .usesFontLeading])
        func rect(_ r: CGRect) -> [Double] { [r.origin.x, r.origin.y, r.size.width, r.size.height].map(Double.init) }
        var fonts: [[String: Any]] = []
        var seen: Set<String> = []
        for name in [style.fontName] + (style.fontRuns ?? []).map(\.fontName) where !seen.contains(name) {
            seen.insert(name)
            let requested = NSFont(name: name, size: style.fontSize)
            let font = requested ?? NSFont.systemFont(ofSize: style.fontSize)
            let url = CTFontCopyAttribute(font as CTFont, kCTFontURLAttribute) as? URL
            fonts.append([
                "requested": name, "found": requested != nil, "fontName": font.fontName, "pointSize": Double(font.pointSize),
                "ascender": Double(font.ascender), "descender": Double(font.descender), "leading": Double(font.leading),
                "capHeight": Double(font.capHeight), "xHeight": Double(font.xHeight),
                "unitsPerEm": Int(CTFontGetUnitsPerEm(font as CTFont)), "file": url?.path ?? "",
                "version": (CTFontCopyName(font as CTFont, kCTFontVersionNameKey) as String?) ?? "",
            ])
        }
        // Each glyph's outline in font units, so the port can check it has the same outlines.
        var outlines: [String: [String: String]] = [:]
        var index0 = glyphs.location
        while index0 < NSMaxRange(glyphs) {
            let character = layout.characterIndexForGlyph(at: index0)
            let glyph = layout.cgGlyph(at: index0)
            if character < string.length, let font = string.attribute(.font, at: character, effectiveRange: nil) as? NSFont {
                let unscaled = CTFontCreateCopyWithAttributes(font as CTFont, CGFloat(CTFontGetUnitsPerEm(font as CTFont)), nil, nil)
                var parts: [String] = []
                if let path = CTFontCreatePathForGlyph(unscaled, glyph, nil) {
                    path.applyWithBlock { element in
                        let e = element.pointee
                        func p(_ i: Int) -> String { "\(Double(e.points[i].x)) \(Double(e.points[i].y))" }
                        switch e.type {
                        case .moveToPoint: parts.append("M \(p(0))")
                        case .addLineToPoint: parts.append("L \(p(0))")
                        case .addQuadCurveToPoint: parts.append("Q \(p(0)) \(p(1))")
                        case .addCurveToPoint: parts.append("C \(p(0)) \(p(1)) \(p(2))")
                        case .closeSubpath: parts.append("Z")
                        @unknown default: parts.append("?")
                        }
                    }
                }
                outlines[font.fontName, default: [:]][String(glyph)] = parts.joined(separator: " ")
            }
            index0 += 1
        }
        var lines: [[String: Any]] = []
        var index = glyphs.location
        while index < NSMaxRange(glyphs) {
            var range = NSRange(location: 0, length: 0)
            let fragment = layout.lineFragmentRect(forGlyphAt: index, effectiveRange: &range)
            let used = layout.lineFragmentUsedRect(forGlyphAt: index, effectiveRange: nil)
            var items: [[Double]] = []
            for g in range.location..<NSMaxRange(range) {
                let location = layout.location(forGlyphAt: g)
                let glyph = layout.cgGlyph(at: g)
                let character = layout.characterIndexForGlyph(at: g)
                let shown = layout.propertyForGlyph(at: g) == .null ? 0.0 : 1.0
                items.append([Double(glyph), Double(character), Double(location.x), Double(location.y), shown])
            }
            lines.append(["fragment": rect(fragment), "used": rect(used), "glyphs": items])
            index = NSMaxRange(range)
        }
        let note: [String: Any] = [
            "textLayout": [
                "boxSize": [Double(size.width), Double(size.height)], "measured": rect(measured), "fonts": fonts, "lines": lines,
                "typesetterBehavior": layout.typesetterBehavior.rawValue, "outlines": outlines,
            ],
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: note, options: [.sortedKeys]) else { return "textLayout: unencodable" }
        return String(decoding: data, as: UTF8.self)
    }

    /// Every installed face, as `PostScript name<TAB>file<TAB>version`, for harness-info.json.
    nonisolated static func parityInstalledFonts() -> [String] {
        let collection = CTFontCollectionCreateFromAvailableFonts(nil)
        let descriptors = (CTFontCollectionCreateMatchingFontDescriptors(collection) as? [CTFontDescriptor]) ?? []
        return descriptors.map { descriptor in
            let name = CTFontDescriptorCopyAttribute(descriptor, kCTFontNameAttribute) as? String ?? ""
            let url = CTFontDescriptorCopyAttribute(descriptor, kCTFontURLAttribute) as? URL
            let font = CTFontCreateWithFontDescriptor(descriptor, 12, nil)
            let version = (CTFontCopyName(font, kCTFontVersionNameKey) as String?) ?? ""
            return "\(name)\t\(url?.path ?? "")\t\(version)"
        }.sorted()
    }
}
