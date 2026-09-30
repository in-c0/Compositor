import Foundation

/// One `[[state]]` from parity/ui/states.toml: a view of the Mac app to render, with the document, tool and sheet it
/// shows. See the comments at the top of that file.
nonisolated struct UIState: Sendable {
    let id: String
    let view: String
    /// A corpus case id whose input.comp is open.
    let document: String?
    let tool: String?
    let sheet: String?
    /// The layer to select, bottom = 0.
    let layer: Int?
}

/// A state as read from the file: the state, or why it can't be rendered. A malformed state is recorded as an error
/// for its id and the rest still render.
nonisolated struct UIStateEntry: Sendable {
    let id: String
    let state: UIState?
    let error: String?
}

/// Reads states.toml. Only the part of TOML the file uses is understood: comments, `[[state]]` tables and
/// `key = value` lines whose value is a string, an integer or a boolean. Anything else is a syntax error, which
/// stops the run before anything renders.
nonisolated enum UIStateFile {
    static let fields: Set<String> = ["id", "view", "document", "tool", "sheet", "layer"]
    static let views: Set<String> = ["window", "tool-rail", "tool-header", "status-bar", "layers-panel", "sheet"]

    static func load(_ url: URL) throws -> [UIStateEntry] {
        let text = try String(contentsOf: url, encoding: .utf8)
        var tables: [[String: Any]] = []
        for (index, rawLine) in text.components(separatedBy: .newlines).enumerated() {
            let line = try stripComment(rawLine, line: index + 1).trimmingCharacters(in: .whitespaces)
            if line.isEmpty { continue }
            if line == "[[state]]" {
                tables.append([:])
                continue
            }
            guard let equals = line.firstIndex(of: "=") else {
                throw HarnessError("states.toml line \(index + 1): expected [[state]] or key = value")
            }
            let key = line[..<equals].trimmingCharacters(in: .whitespaces)
            guard !key.isEmpty, key.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "_" || $0 == "-" }) else {
                throw HarnessError("states.toml line \(index + 1): “\(key)” isn't a bare key")
            }
            guard !tables.isEmpty else { throw HarnessError("states.toml line \(index + 1): a key outside [[state]]") }
            guard tables[tables.count - 1][key] == nil else { throw HarnessError("states.toml line \(index + 1): \(key) is set twice") }
            tables[tables.count - 1][key] = try value(line[line.index(after: equals)...].trimmingCharacters(in: .whitespaces),
                                                      line: index + 1)
        }
        var seen: Set<String> = []
        return tables.enumerated().map { index, table -> UIStateEntry in
            let id = (table["id"] as? String).map(CorpusRenderer.normalizedID) ?? ""
            let name = id.isEmpty ? "state \(index + 1)" : id
            do {
                guard !id.isEmpty else { throw HarnessError("the state has no id") }
                guard seen.insert(id).inserted else { throw HarnessError("another state has the id \(id)") }
                return UIStateEntry(id: name, state: try state(id: id, table), error: nil)
            } catch {
                return UIStateEntry(id: name, state: nil, error: describe(error))
            }
        }
    }

    private static func state(id: String, _ table: [String: Any]) throws -> UIState {
        let unknown = Set(table.keys).subtracting(fields)
        guard unknown.isEmpty else { throw HarnessError("unknown field\(unknown.count == 1 ? "" : "s") \(unknown.sorted().joined(separator: ", "))") }
        func string(_ key: String) throws -> String? {
            guard let value = table[key] else { return nil }
            guard let text = value as? String else { throw HarnessError("\(key) must be a string") }
            return text
        }
        guard let view = try string("view") else { throw HarnessError("view is missing") }
        guard views.contains(view) else { throw HarnessError("view “\(view)” isn't one of \(views.sorted().joined(separator: ", "))") }
        var layer: Int?
        if let value = table["layer"] {
            guard let number = value as? Int, number >= 0 else { throw HarnessError("layer must be a whole number from 0") }
            layer = number
        }
        let sheet = try string("sheet")
        if view == "sheet", sheet == nil { throw HarnessError("a sheet state needs sheet") }
        if view != "sheet", sheet != nil { throw HarnessError("sheet is only for view = \"sheet\"") }
        return UIState(id: id, view: view, document: try string("document").map(CorpusRenderer.normalizedID),
                       tool: try string("tool"), sheet: sheet, layer: layer)
    }

    /// The line without a trailing `# comment`, leaving a # inside a string alone.
    private static func stripComment(_ line: String, line number: Int) throws -> String {
        var inString = false
        var escaped = false
        for (offset, character) in line.enumerated() {
            if inString {
                if escaped { escaped = false } else if character == "\\" { escaped = true } else if character == "\"" { inString = false }
            } else if character == "\"" {
                inString = true
            } else if character == "#" {
                return String(line.prefix(offset))
            }
        }
        if inString { throw HarnessError("states.toml line \(number): a string isn't closed") }
        return line
    }

    private static func value(_ text: String, line: Int) throws -> Any {
        if text == "true" { return true }
        if text == "false" { return false }
        if let number = Int(text) { return number }
        guard text.count >= 2, text.first == "\"", text.last == "\"" else {
            throw HarnessError("states.toml line \(line): only strings, integers and booleans are supported")
        }
        var result = ""
        var escaped = false
        for character in text.dropFirst().dropLast() {
            if escaped {
                switch character {
                case "n": result.append("\n")
                case "t": result.append("\t")
                case "\"", "\\": result.append(character)
                default: throw HarnessError("states.toml line \(line): unsupported escape \\\(character)")
                }
                escaped = false
            } else if character == "\\" {
                escaped = true
            } else if character == "\"" {
                throw HarnessError("states.toml line \(line): a quote inside a string must be escaped")
            } else {
                result.append(character)
            }
        }
        return result
    }
}

/// `ParityHarness ui --states <file> --corpus <dir> --out <dir> [--main-menu <file>] [--state <id>]...`, and the
/// per-state child processes it starts (`ui-state`, `ui-menus`).
nonisolated struct UIOptions: Sendable {
    let states: URL
    let corpus: URL
    let output: URL
    /// The app's own main menu, as parity/harness/menus/dump-main-menu.sh wrote it.
    let mainMenu: URL?
    /// Empty renders every state.
    let stateIDs: [String]
    /// Set in a child: where it writes its result.
    let result: URL?

    static let usage = """
        usage: ParityHarness ui --states <file> --corpus <dir> --out <dir> [--main-menu <file>] [--state <id>]...

        Renders the Mac app's own views for every state in the states file (parity/ui/states.toml), or
        only the named ones, into <out>/<id>.png at 1x in the dark appearance, and writes
        <out>/ui-info.json (per-state status, pixel size and environment) and <out>/menus.json (the
        main menu from --main-menu, and the Layers panel's row menu). Each state renders in its own
        process, so one that fails or crashes is recorded and the run goes on.
        """

    static func parse(_ arguments: [String]) throws -> UIOptions {
        var values: [String: String] = [:]
        var stateIDs: [String] = []
        var index = 1
        let flags = ["--states", "--corpus", "--out", "--main-menu", "--state", "--result"]
        while index < arguments.count {
            let flag = arguments[index]
            // `-Name value` pairs are defaults for this process's argument domain (see UIRunner.childDefaults).
            if flag.hasPrefix("-"), !flag.hasPrefix("--"), index + 1 < arguments.count {
                index += 2
                continue
            }
            guard flags.contains(flag) else { throw UsageError(message: "unknown option “\(flag)”") }
            guard index + 1 < arguments.count else { throw UsageError(message: "\(flag) needs a value") }
            let value = arguments[index + 1]
            if flag == "--state" {
                let id = CorpusRenderer.normalizedID(value)
                guard !id.isEmpty else { throw UsageError(message: "--state needs a state id") }
                if !stateIDs.contains(id) { stateIDs.append(id) }
            } else {
                values[flag] = value
            }
            index += 2
        }
        guard let states = values["--states"] else { throw UsageError(message: "--states is required") }
        guard let corpus = values["--corpus"] else { throw UsageError(message: "--corpus is required") }
        guard let output = values["--out"] else { throw UsageError(message: "--out is required") }
        let statesURL = URL(fileURLWithPath: states).standardizedFileURL
        guard FileManager.default.fileExists(atPath: statesURL.path(percentEncoded: false)) else {
            throw UsageError(message: "--states \(states) doesn't exist")
        }
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
        return UIOptions(states: statesURL, corpus: corpusURL, output: outputURL,
                         mainMenu: values["--main-menu"].map { URL(fileURLWithPath: $0).standardizedFileURL },
                         stateIDs: stateIDs, result: values["--result"].map { URL(fileURLWithPath: $0).standardizedFileURL })
    }
}
