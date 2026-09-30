import AppKit
import Foundation

/// `ParityHarness ui`: renders every state in its own child process (`ui-state`), so a state that hangs, crashes or
/// leaves the app in a strange state can't take the others with it, then writes ui-info.json and menus.json.
final class UIRunner {
    let options: UIOptions
    /// A state that takes longer than this is stopped and recorded as an error.
    static let stateTimeout: Double = 120

    init(options: UIOptions) {
        self.options = options
    }

    /// The defaults domain the harness and its children read: each child starts from an empty one, so nothing a
    /// state (or an earlier run, or a person) left behind changes how the views draw.
    static var defaultsDomain: String { Bundle.main.bundleIdentifier ?? ProcessInfo.processInfo.processName }

    func run() async {
        let entries: [UIStateEntry]
        do {
            entries = try UIStateFile.load(options.states)
        } catch {
            standardError("ParityHarness: \(describe(error))")
            exit(2)
        }
        let chosen = options.stateIDs.isEmpty ? entries : options.stateIDs.map { id in
            entries.first { $0.id == id } ?? UIStateEntry(id: id, state: nil, error: "There is no state \(id) in \(options.states.lastPathComponent).")
        }
        if chosen.isEmpty { standardError("ParityHarness: no states in \(options.states.path(percentEncoded: false))") }
        var results: [UIStateResult] = []
        for (number, entry) in chosen.enumerated() {
            let result: UIStateResult
            if entry.state != nil {
                let child = await runChild("ui-state", extra: ["--state", entry.id], label: entry.id)
                if let data = child.data, let decoded = try? JSONDecoder().decode(UIStateResult.self, from: data) {
                    result = decoded
                } else {
                    result = .failure(entry.id, child.failure ?? "the state's process wrote an unreadable result")
                    try? FileManager.default.removeItem(at: options.output.appending(path: entry.id + ".png"))
                }
            } else {
                result = .failure(entry.id, entry.error ?? "the state can't be read")
            }
            let size = result.width.flatMap { width in result.height.map { " \(width)×\($0)" } } ?? ""
            standardError("[\(number + 1)/\(chosen.count)] \(result.id): \(result.status.rawValue)\(size)\(result.error.map { " (\($0))" } ?? "")")
            results.append(result)
        }

        var menus: [String: Any] = [:]
        readMainMenu(into: &menus)
        let child = await runChild("ui-menus", extra: [], label: "menus")
        if let data = child.data, let rows = (try? JSONSerialization.jsonObject(with: data)) as? [[String: Any]] {
            menus["layerContextMenus"] = rows
        } else {
            menus["layerContextMenusError"] = child.failure ?? "the menus process wrote an unreadable result"
        }
        standardError("menus: main menu \(menus["mainMenu"] == nil ? "missing" : "ok"), layer row menus \(menus["layerContextMenus"] == nil ? "missing" : "ok")")

        write(UIInfo.collect(states: results), to: "ui-info.json")
        do {
            let data = try JSONSerialization.data(withJSONObject: menus, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
            try data.write(to: options.output.appending(path: "menus.json"), options: .atomic)
        } catch {
            standardError("ParityHarness: couldn't write menus.json: \(error.localizedDescription)")
        }
        let failed = results.filter { $0.status == .error }.count
        print("Rendered \(results.count - failed) of \(results.count) UI states into \(options.output.path(percentEncoded: false)); \(failed) failed. See ui-info.json and menus.json.")
    }

    /// The main menu comes from the real app (parity/harness/menus/dump-main-menu.sh), because the harness can't
    /// compile CompositorApp.swift, where the menu commands are declared.
    private func readMainMenu(into menus: inout [String: Any]) {
        guard let url = options.mainMenu else {
            menus["mainMenuError"] = "Not captured: pass --main-menu with the file parity/harness/menus/dump-main-menu.sh writes (parity/harness/ui.sh does)."
            return
        }
        guard let data = try? Data(contentsOf: url), !data.isEmpty else {
            menus["mainMenuError"] = "The app didn't report its main menu (\(url.lastPathComponent) is missing or empty); see the log of dump-main-menu.sh."
            return
        }
        guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let items = object["items"] as? [[String: Any]] else {
            menus["mainMenuError"] = "\(url.lastPathComponent) isn't the JSON MenuDump.m writes."
            return
        }
        menus["mainMenu"] = MenuDump.normalize(items)
        if let capture = object["capture"] { menus["mainMenuCapture"] = capture }
    }

    private func write(_ value: some Encodable, to name: String) {
        let url = options.output.appending(path: name)
        do {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
            try encoder.encode(value).write(to: url, options: .atomic)
        } catch {
            standardError("ParityHarness: couldn't write \(url.path(percentEncoded: false)): \(error.localizedDescription)")
        }
    }

    /// Runs this executable as `command`, waits for it (up to `stateTimeout`), and returns what it wrote to its
    /// --result file, or why there is nothing. The child's own log is passed on, indented, to standard error.
    private func runChild(_ command: String, extra: [String], label: String) async -> (data: Data?, failure: String?) {
        let folder = FileManager.default.temporaryDirectory.appending(path: "parity-ui-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let resultURL = folder.appending(path: "result.json")
        let logURL = folder.appending(path: "log.txt")
        FileManager.default.createFile(atPath: logURL.path(percentEncoded: false), contents: nil)
        let log = try? FileHandle(forWritingTo: logURL)

        let process = Process()
        process.executableURL = Bundle.main.executableURL ?? URL(fileURLWithPath: CommandLine.arguments[0])
        process.arguments = [command, "--states", options.states.path(percentEncoded: false),
                             "--corpus", options.corpus.path(percentEncoded: false),
                             "--out", options.output.path(percentEncoded: false),
                             "--result", resultURL.path(percentEncoded: false)] + extra
        var environment = ProcessInfo.processInfo.environment
        // ToolDefaults uses its compiled defaults, not the tool.* preferences, when this is set.
        environment["XCTestConfigurationFilePath"] = environment["XCTestConfigurationFilePath"] ?? "/dev/null"
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = log ?? FileHandle.nullDevice
        process.standardError = log ?? FileHandle.nullDevice

        UserDefaults.standard.removePersistentDomain(forName: Self.defaultsDomain)
        do {
            try process.run()
        } catch {
            return (nil, "couldn't start the \(label) process: \(error.localizedDescription)")
        }
        let deadline = Date().addingTimeInterval(Self.stateTimeout)
        while process.isRunning, Date() < deadline { await settle(0.05) }
        var timedOut = false
        if process.isRunning {
            timedOut = true
            process.terminate()
            let grace = Date().addingTimeInterval(3)
            while process.isRunning, Date() < grace { await settle(0.05) }
            if process.isRunning { kill(process.processIdentifier, SIGKILL) }
            process.waitUntilExit()
        }
        try? log?.close()

        let logText = (try? String(contentsOf: logURL, encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !logText.isEmpty {
            standardError(logText.split(separator: "\n", omittingEmptySubsequences: false).map { "  | " + $0 }.joined(separator: "\n"))
        }
        if !timedOut, let data = try? Data(contentsOf: resultURL), !data.isEmpty { return (data, nil) }
        let how = timedOut ? "was stopped after \(Int(Self.stateTimeout)) s"
            : process.terminationReason == .uncaughtSignal ? "crashed (signal \(process.terminationStatus))"
            : "exited with status \(process.terminationStatus)"
        let tail = logText.split(separator: "\n").suffix(3).joined(separator: " / ")
        return (nil, "the \(label) process \(how) without a result" + (tail.isEmpty ? "" : ": \(tail)"))
    }
}

/// The child processes: one state, or the Layers panel's row menus.
enum UIChild {
    /// What the app delegate does at launch, plus an active app so the capture windows can be key.
    static func prepareApplication() async {
        _ = NSApp.setActivationPolicy(.regular)
        // Always dark, as applicationWillFinishLaunching sets it.
        NSApp.appearance = NSAppearance(named: .darkAqua)
        SliderSnap.install()
        // Snapshots draw the canvas with Core Graphics anyway; this keeps the Metal view from ever being made.
        UserDefaults.standard.register(defaults: ["CompositorCPUCanvas": true])
        NSApp.finishLaunching()
        NSApp.activate()
        await settle(0.3)
    }

    static func renderState(_ options: UIOptions) async {
        await prepareApplication()
        let id = options.stateIDs.first ?? ""
        let result: UIStateResult
        do {
            guard let entry = try UIStateFile.load(options.states).first(where: { $0.id == id }) else {
                throw HarnessError("there is no state \(id)")
            }
            guard let state = entry.state else { throw HarnessError(entry.error ?? "the state can't be read") }
            result = await UIStateRenderer(state: state, options: options).render()
        } catch {
            result = .failure(id, describe(error))
        }
        do {
            try JSONEncoder().encode(result).write(to: try resultURL(options))
        } catch {
            standardError("ParityHarness: couldn't write the state's result: \(describe(error))")
        }
    }

    static func dumpMenus(_ options: UIOptions) async {
        await prepareApplication()
        let rows = await MenuDump.layerContextMenus(corpus: options.corpus)
        do {
            try JSONSerialization.data(withJSONObject: rows).write(to: try resultURL(options))
        } catch {
            standardError("ParityHarness: couldn't write the menus: \(describe(error))")
        }
    }

    private static func resultURL(_ options: UIOptions) throws -> URL {
        guard let url = options.result else { throw HarnessError("--result is required") }
        return url
    }
}

/// ui-info.json: the machine, the settings every state renders with, and how each state went.
nonisolated struct UIInfo: Encodable {
    let macOS: String
    let macOSVersion: String
    let architecture: String
    let buildConfiguration: String
    let xcode: String?
    let swiftCompiler: String?
    /// Every PNG is one pixel per point; `backingScale` per state says what the window's own scale was.
    let scale: Int
    let appearance: String
    /// `NSColor.controlAccentColor` in the dark appearance, as sRGB hex. The app has no accent color of its own, so
    /// this is the Mac's setting.
    let accentColor: String?
    /// The global AppleAccentColor preference, or nil when unset (the default blue).
    let appleAccentColor: String?
    /// Whether ToolDefaults used its compiled defaults (XCTestConfigurationFilePath set for the children).
    let toolDefaultsPinned: Bool
    /// The defaults domain emptied before each state.
    let defaultsDomain: String
    let editorSize: [Double]
    let states: [UIStateResult]
}

@MainActor extension UIInfo {
    static func collect(states: [UIStateResult]) -> UIInfo {
        let machine = HarnessInfo.collect(cases: [])
        var accent: String?
        NSAppearance(named: .darkAqua)?.performAsCurrentDrawingAppearance {
            if let color = NSColor.controlAccentColor.usingColorSpace(.sRGB) {
                accent = String(format: "#%02X%02X%02X", Int((color.redComponent * 255).rounded()),
                                Int((color.greenComponent * 255).rounded()), Int((color.blueComponent * 255).rounded()))
            }
        }
        return UIInfo(macOS: machine.macOS, macOSVersion: machine.macOSVersion, architecture: machine.architecture,
                      buildConfiguration: machine.buildConfiguration, xcode: machine.xcode, swiftCompiler: machine.swiftCompiler,
                      scale: 1, appearance: "darkAqua", accentColor: accent,
                      appleAccentColor: UserDefaults.standard.object(forKey: "AppleAccentColor").map { "\($0)" },
                      toolDefaultsPinned: true, defaultsDomain: UIRunner.defaultsDomain,
                      editorSize: [editorSize.width, editorSize.height], states: states)
    }
}
