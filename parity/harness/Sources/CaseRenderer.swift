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
        for url in [pngURL, projectURL] where FileManager.default.fileExists(atPath: url.path(percentEncoded: false)) {
            try? FileManager.default.removeItem(at: url)
        }
    }

    private func renderCase(notes: inout [String]) async throws -> [String] {
        let spec = try CaseSpec.load(from: folder.appending(path: "case.json"))
        let input = folder.appending(path: spec.input)
        // A fresh session per case, as a new project tab has.
        let session = EditorSession()
        let isPhotoshop: Bool
        switch input.pathExtension.lowercased() {
        case "comp":
            // File > Open (ProjectController.open): load and validate the package, then install it.
            let snapshot = try await ProjectStore.shared.load(from: input)
            session.installProject(snapshot, from: input)
            isPhotoshop = false
        case "psd", "psb":
            notes += try await importPhotoshop(input, into: session)
            isPhotoshop = true
        default:
            throw HarnessError("input “\(spec.input)” must end in .comp, .psd or .psb")
        }
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
        if !spec.ops.isEmpty || isPhotoshop {
            // The QuickLook preview the app adds on save is left out; loading ignores it.
            try await ProjectStore.shared.save(snapshot, to: projectURL)
            outputs.append(id + ".comp")
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
}

private final class ConversionLog {
    var entries: [String] = []
}
