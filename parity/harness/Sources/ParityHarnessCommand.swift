import AppKit
import CoreGraphics
import Foundation

/// `ParityHarness render --corpus <dir> --out <dir> [--case <id>]...`
///
/// Renders parity corpus cases through the Mac app's own code: the project is opened, the case's ops run through
/// the app's editing commands, and the result is flattened by the same path as File > Export PNG. See
/// parity/README.md for the case format.
@main
struct ParityHarnessCommand {
    static func main() async {
        let options: RenderOptions
        do {
            options = try RenderOptions.parse(Array(CommandLine.arguments.dropFirst()))
        } catch let error as UsageError {
            if error.isHelp {
                print(RenderOptions.usage)
                exit(0)
            }
            standardError("ParityHarness: \(error.message)\n\n\(RenderOptions.usage)")
            exit(2)
        } catch {
            standardError("ParityHarness: \(error.localizedDescription)")
            exit(2)
        }
        // Some AppKit code expects the shared application object to exist. It is created here but never run.
        _ = NSApplication.shared
        await CorpusRenderer(options: options).run()
        exit(0)
    }
}

nonisolated struct UsageError: Error {
    var message: String
    var isHelp = false
}

nonisolated func standardError(_ message: String) {
    FileHandle.standardError.write(Data((message + "\n").utf8))
}

struct RenderOptions {
    let corpus: URL
    let output: URL
    /// Empty renders every case in the corpus.
    let caseIDs: [String]

    static let usage = """
        usage: ParityHarness render --corpus <dir> --out <dir> [--case <id>]...

        Renders every case under the corpus, or only the named ones, with the Mac app's own code.
        Writes <out>/<id>.png, <out>/<id>.comp for cases with ops or an imported file, and
        <out>/harness-info.json. A case id is the case folder's path relative to the corpus,
        such as blend/multiply-50-opaque. A case that fails is recorded in harness-info.json and
        the run goes on; the exit status is non-zero only for usage errors.
        """

    static func parse(_ arguments: [String]) throws -> RenderOptions {
        if arguments.contains("--help") || arguments.contains("-h") || arguments.first == "help" {
            throw UsageError(message: "", isHelp: true)
        }
        guard let command = arguments.first else { throw UsageError(message: "missing command") }
        guard command == "render" else { throw UsageError(message: "unknown command “\(command)”") }
        var corpus: String?
        var output: String?
        var caseIDs: [String] = []
        var index = 1
        while index < arguments.count {
            let flag = arguments[index]
            guard ["--corpus", "--out", "--case"].contains(flag) else { throw UsageError(message: "unknown option “\(flag)”") }
            guard index + 1 < arguments.count else { throw UsageError(message: "\(flag) needs a value") }
            let value = arguments[index + 1]
            switch flag {
            case "--corpus": corpus = value
            case "--out": output = value
            default:
                let id = CorpusRenderer.normalizedID(value)
                guard !id.isEmpty else { throw UsageError(message: "--case needs a case id") }
                if !caseIDs.contains(id) { caseIDs.append(id) }
            }
            index += 2
        }
        guard let corpus else { throw UsageError(message: "--corpus is required") }
        guard let output else { throw UsageError(message: "--out is required") }
        let corpusURL = URL(fileURLWithPath: corpus).standardizedFileURL
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: corpusURL.path(percentEncoded: false), isDirectory: &isDirectory),
              isDirectory.boolValue else { throw UsageError(message: "--corpus \(corpus) is not a directory") }
        let outputURL = URL(fileURLWithPath: output).standardizedFileURL
        do {
            try FileManager.default.createDirectory(at: outputURL, withIntermediateDirectories: true)
        } catch {
            throw UsageError(message: "can't create --out \(output): \(error.localizedDescription)")
        }
        return RenderOptions(corpus: corpusURL, output: outputURL, caseIDs: caseIDs)
    }
}

final class CorpusRenderer {
    let options: RenderOptions

    init(options: RenderOptions) {
        self.options = options
    }

    /// `blend//multiply/` and `blend/multiply` name the same case.
    nonisolated static func normalizedID(_ id: String) -> String {
        id.split(separator: "/", omittingEmptySubsequences: true).joined(separator: "/")
    }

    func run() async {
        let available = discoverCases()
        let ids = options.caseIDs.isEmpty ? available.keys.sorted() : options.caseIDs
        if ids.isEmpty { standardError("ParityHarness: no case.json found under \(options.corpus.path(percentEncoded: false))") }
        var results: [CaseResult] = []
        for (number, id) in ids.enumerated() {
            let result: CaseResult
            if let folder = available[id] {
                result = await CaseRenderer(id: id, folder: folder, output: options.output).render()
            } else {
                result = CaseResult(id: id, status: .error, error: "There is no case.json for \(id) in the corpus.", outputs: [], notes: [])
            }
            standardError("[\(number + 1)/\(ids.count)] \(id): \(result.status.rawValue)\(result.error.map { " (\($0))" } ?? "")")
            results.append(result)
        }
        let infoURL = options.output.appending(path: "harness-info.json")
        do {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
            try encoder.encode(HarnessInfo.collect(cases: results)).write(to: infoURL, options: .atomic)
        } catch {
            standardError("ParityHarness: couldn't write \(infoURL.path(percentEncoded: false)): \(error.localizedDescription)")
        }
        let failed = results.filter { $0.status == .error }.count
        print("Rendered \(results.count - failed) of \(results.count) cases into \(options.output.path(percentEncoded: false)); \(failed) failed. See harness-info.json.")
    }

    /// Every folder under the corpus holding a case.json, by case id. Synchronous: directory enumeration isn't
    /// available from asynchronous contexts.
    private func discoverCases() -> [String: URL] {
        let root = options.corpus.resolvingSymlinksInPath()
        let rootComponents = root.pathComponents
        guard let enumerator = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil,
                                                              options: [.skipsHiddenFiles]) else { return [:] }
        var found: [String: URL] = [:]
        while let url = enumerator.nextObject() as? URL {
            // A case's input package holds no cases.
            if url.pathExtension.lowercased() == "comp" {
                enumerator.skipDescendants()
                continue
            }
            guard url.lastPathComponent == "case.json" else { continue }
            let folder = url.deletingLastPathComponent().resolvingSymlinksInPath()
            let id = folder.pathComponents.dropFirst(rootComponents.count).joined(separator: "/")
            if !id.isEmpty { found[id] = folder }
        }
        return found
    }
}
