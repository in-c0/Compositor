import CoreGraphics
import Foundation

nonisolated enum CaseStatus: String, Encodable, Sendable {
    case ok, error
}

nonisolated struct CaseResult: Encodable, Sendable {
    let id: String
    let status: CaseStatus
    var error: String?
    /// Files written for the case, relative to the output folder.
    var outputs: [String]
    /// Things worth knowing that didn't stop the case, such as the conversions a PSD import accepted.
    var notes: [String]
}

nonisolated struct HarnessError: LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}

nonisolated func describe(_ error: Error) -> String {
    if error is DecodingError { return String(describing: error) }
    return error.localizedDescription
}

/// One case: open its input as the app does, run its ops through the app's editing commands, and export.
struct CaseRenderer {
    let id: String
    let folder: URL
    let output: URL

    private var pngURL: URL { output.appending(path: id + ".png") }
    private var projectURL: URL { output.appending(path: id + ".comp") }
    private var jpegURL: URL { output.appending(path: id + ".jpg") }
    private var selectionURL: URL { output.appending(path: id + ".selection.png") }

    func render() async -> CaseResult {
        removeOutputs()
        var notes: [String] = []
        do {
            let outputs = try await renderCase(notes: &notes)
            return CaseResult(id: id, status: .ok, error: nil, outputs: outputs, notes: notes)
        } catch {
            // No half-written results: an error case has no files, so nothing stale is compared.
            removeOutputs()
            return CaseResult(id: id, status: .error, error: describe(error), outputs: [], notes: notes)
        }
    }

    private func removeOutputs() {
        for url in [pngURL, projectURL, jpegURL, selectionURL] where FileManager.default.fileExists(atPath: url.path(percentEncoded: false)) {
            try? FileManager.default.removeItem(at: url)
        }
    }

    private func renderCase(notes: inout [String]) async throws -> [String] {
        let spec = try CaseSpec.load(from: folder.appending(path: "case.json"))
        let input = folder.appending(path: spec.input)
        // A fresh session per case, as a new project tab has.
        let session = EditorSession()
        let imported: Bool
        switch input.pathExtension.lowercased() {
        case "comp":
            guard spec.raw == nil else { throw HarnessError("case.json.raw is only for camera RAW inputs") }
            // File > Open (ProjectController.open): load and validate the package, then install it.
            let snapshot = try await ProjectStore.shared.load(from: input)
            session.installProject(snapshot, from: input)
            imported = false
        case "psd", "psb":
            guard spec.raw == nil else { throw HarnessError("case.json.raw is only for camera RAW inputs") }
            notes += try await importPhotoshop(input, into: session)
            imported = true
        default:
            notes += try await importImage(input, raw: spec.raw, into: session)
            imported = true
        }
        parityTextNotes = []
        defer { notes += parityTextNotes }
        for (index, op) in spec.ops.enumerated() {
            do {
                try await op.apply(to: session)
            } catch {
                throw HarnessError("op \(index + 1) (\(op.name)): \(describe(error))")
            }
        }
        // File > Export PNG (ProjectController.exportPNG): the session's snapshot through ImageExporter.exportPNG.
        guard let snapshot = session.projectSnapshot() else { throw HarnessError("the session has no document to export") }
        try FileManager.default.createDirectory(at: pngURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try await ImageExporter.shared.exportPNG(snapshot, to: pngURL)
        var outputs = [id + ".png"]
        if let options = spec.jpeg {
            // File > Export JPEG: the same render, flattened onto the matte and encoded by ImageIO.
            let raster = try await ImageExporter.shared.render(snapshot)
            let result = try await ImageExporter.shared.jpeg(raster, options: options)
            try result.data.write(to: jpegURL)
            outputs.append(id + ".jpg")
        }
        if !spec.ops.isEmpty || imported {
            // The QuickLook preview the app adds on save is left out; loading ignores it.
            try await ProjectStore.shared.save(snapshot, to: projectURL)
            outputs.append(id + ".comp")
        }
        // The selection isn't part of the project, so it gets a file of its own: its coverage at document size.
        if let selection = session.selection, let document = session.document {
            try SelectionCoverageFile.write(selection, width: document.width, height: document.height, to: selectionURL)
            outputs.append(id + ".selection.png")
        }
        return outputs
    }

    /// Opening a .psd or .psb: a new project tab imports it into its empty session (ProjectWorkspace.receive →
    /// EditorSession.importImages). The conversions the app would list for the person to accept are accepted, as
    /// pressing Import does, and returned as notes.
    private func importPhotoshop(_ url: URL, into session: EditorSession) async throws -> [String] {
        let log = ConversionLog()
        session.confirmConversions = { conversions in
            log.entries += conversions.map { "PSD conversion accepted, layer “\($0.layerName)”: \($0.message)" }
            return true
        }
        defer { session.confirmConversions = nil }
        await session.importImages([url])
        if let message = session.importError { throw HarnessError(message) }
        guard session.document != nil else { throw HarnessError("the Photoshop import made no document") }
        return log.entries
    }

    /// Opening any other file: a new project tab imports it into its empty session (EditorSession.importImages), which
    /// decodes it with ImageIO, draws an SVG, or develops a camera RAW. For a RAW file the develop sheet opens with the
    /// camera's own settings; the harness changes the ones the case's `raw` sets and presses Import. The settings used
    /// are returned as a note. A file the app refuses fails the case with the app's own message.
    private func importImage(_ url: URL, raw: RawDevelopPatch?, into session: EditorSession) async throws -> [String] {
        let isRaw = RawImporter.matches(url)
        if raw != nil, !isRaw { throw HarnessError("case.json.raw is only for camera RAW inputs") }
        let log = ConversionLog()
        session.confirmRawDevelop = { _, asShot in
            var settings = asShot
            raw?.apply(to: &settings)
            log.entries.append("RAW develop: as shot \(asShot.asShotTemperature) K, tint \(asShot.asShotTint); imported with "
                + "exposure \(settings.exposure), temperature \(settings.temperature) K, tint \(settings.tint), boost \(settings.boost)")
            return settings
        }
        defer { session.confirmRawDevelop = nil }
        await session.importImages([url])
        if let message = session.importError { throw HarnessError(message) }
        guard session.document != nil else { throw HarnessError("the import made no document") }
        if isRaw, log.entries.isEmpty { throw HarnessError("the RAW develop step didn't run") }
        return log.entries
    }
}

/// The develop sheet's controls a case changes before pressing Import; the rest keep the camera's settings.
struct RawDevelopPatch {
    var exposure: Float?
    var temperature: Float?
    var tint: Float?
    var boost: Float?

    func apply(to settings: inout RawDevelopSettings) {
        if let exposure { settings.exposure = exposure }
        if let temperature { settings.temperature = temperature }
        if let tint { settings.tint = tint }
        if let boost { settings.boost = boost }
    }

    static func parse(_ value: Any, path: String) throws -> RawDevelopPatch {
        let fields = try JSONFields(value, path: path)
        // The sheet's slider ranges (RawDevelopSheet).
        func slider(_ key: String, _ range: ClosedRange<Double>) throws -> Float? {
            guard let number = try fields.optional(key, JSONValue.number) else { return nil }
            guard range.contains(number) else { throw HarnessError("\(path).\(key) must be \(range.lowerBound)–\(range.upperBound)") }
            return Float(number)
        }
        let patch = RawDevelopPatch(exposure: try slider("exposure", -3...3), temperature: try slider("temperature", 2000...12000),
                                    tint: try slider("tint", -150...150), boost: try slider("boost", 0...1))
        try fields.rejectUnknown()
        return patch
    }
}

private final class ConversionLog {
    var entries: [String] = []
}
