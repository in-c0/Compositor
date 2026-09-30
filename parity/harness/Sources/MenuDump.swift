import AppKit
import SwiftUI

/// Menus as JSON. Both menus in menus.json go through `raw` (here, or the same fields from
/// parity/harness/menus/MenuDump.m inside the running app) and then `normalize`, so they share one form:
///
///     {"separator": true}
///     {"title": "Save As…", "enabled": true, "key": "s", "modifiers": ["shift", "command"], "shortcut": "⇧⌘S",
///      "checked": true, "hidden": true, "alternate": true, "items": [...]}
///
/// Keys that are false or empty are left out. `key` is the key equivalent as AppKit stores it, lowercased when an
/// uppercase letter implied Shift; `shortcut` is how the menu draws it.
enum MenuDump {
    /// An NSMenu's items as the fields MenuDump.m writes. `update()` runs first, as it does before a menu opens, so
    /// enabled states are the ones the menu would show.
    static func raw(_ menu: NSMenu) -> [[String: Any]] {
        menu.update()
        return menu.items.map { item -> [String: Any] in
            if item.isSeparatorItem { return ["separator": true] }
            var entry: [String: Any] = [
                "title": item.title, "key": item.keyEquivalent,
                "modifiers": Int(item.keyEquivalentModifierMask.intersection(.deviceIndependentFlagsMask).rawValue),
                "enabled": item.isEnabled, "hidden": item.isHidden, "state": item.state.rawValue, "alternate": item.isAlternate,
            ]
            if let submenu = item.submenu { entry["submenu"] = raw(submenu) }
            return entry
        }
    }

    static func normalize(_ items: [[String: Any]]) -> [[String: Any]] {
        items.map { item -> [String: Any] in
            if item["separator"] as? Bool == true { return ["separator": true] }
            var entry: [String: Any] = ["title": item["title"] as? String ?? "", "enabled": item["enabled"] as? Bool ?? false]
            if var key = item["key"] as? String, !key.isEmpty {
                var flags = NSEvent.ModifierFlags(rawValue: UInt((item["modifiers"] as? NSNumber)?.uintValue ?? 0))
                if key.count == 1, key != key.lowercased() {
                    // An uppercase key equivalent means Shift, whatever the mask says.
                    flags.insert(.shift)
                    key = key.lowercased()
                }
                let order: [(NSEvent.ModifierFlags, String, String)] = [
                    (.function, "function", "fn "), (.control, "control", "⌃"), (.option, "option", "⌥"),
                    (.shift, "shift", "⇧"), (.command, "command", "⌘"),
                ]
                let held = order.filter { flags.contains($0.0) }
                entry["key"] = key
                entry["modifiers"] = held.map { $0.1 }
                entry["shortcut"] = held.map { $0.2 }.joined() + label(key)
            }
            if let state = item["state"] as? Int, state != NSControl.StateValue.off.rawValue {
                entry["checked"] = state == NSControl.StateValue.on.rawValue ? true as Any : "mixed" as Any
            }
            if item["hidden"] as? Bool == true { entry["hidden"] = true }
            if item["alternate"] as? Bool == true { entry["alternate"] = true }
            if let system = item["system"] as? String { entry["system"] = system }
            if let submenu = item["submenu"] as? [[String: Any]] { entry["items"] = normalize(submenu) }
            return entry
        }
    }

    /// How a menu draws a key equivalent's key.
    static func label(_ key: String) -> String {
        guard let scalar = key.unicodeScalars.first, key.unicodeScalars.count == 1 else { return key.uppercased() }
        switch scalar.value {
        case 0x08, 0x7F: return "⌫"
        case 0xF728: return "⌦"
        case 0x0D, 0x03: return "↩"
        case 0x09: return "⇥"
        case 0x1B: return "⎋"
        case 0x20: return "Space"
        case 0xF700: return "↑"
        case 0xF701: return "↓"
        case 0xF702: return "←"
        case 0xF703: return "→"
        case 0xF729: return "↖"
        case 0xF72B: return "↘"
        case 0xF72C: return "⇞"
        case 0xF72D: return "⇟"
        case 0xF704...0xF726: return "F\(scalar.value - 0xF704 + 1)"
        default: return key.uppercased()
        }
    }

    /// The documents whose rows' menus are listed: plain layers, a folder with a mask, clipped layers, an adjustment
    /// layer and a layer with effects, so every variant title appears.
    static let contextMenuDocuments = ["blend/stack", "masks/folder-mask", "clipping/stack", "adjust/mod-mask", "effects/all"]

    /// Right-clicking each row of the Layers panel, as `LayerTableView.menu(for:)` handles it: the row is selected,
    /// then the coordinator builds the menu. Each row starts from a freshly opened document.
    static func layerContextMenus(corpus: URL) async -> [[String: Any]] {
        var results: [[String: Any]] = []
        for document in contextMenuDocuments {
            var row = 0
            while true {
                var entry: [String: Any] = ["document": document, "row": row]
                do {
                    guard let menu = try await contextMenu(document: document, row: row, corpus: corpus, entry: &entry) else { break }
                    entry["items"] = normalize(raw(menu))
                } catch {
                    entry["error"] = describe(error)
                    results.append(entry)
                    break
                }
                results.append(entry)
                row += 1
            }
        }
        return results
    }

    /// The menu for `row` (0 is the top row), or nil past the last row.
    private static func contextMenu(document: String, row: Int, corpus: URL, entry: inout [String: Any]) async throws -> NSMenu? {
        let session = EditorSession()
        _ = try await UIDocuments.open(document, corpus: corpus, into: session)
        let stage = Stage(AnyView(LayersPanel(session: session, width: layersPanelSize.width).roundedControls()), size: layersPanelSize)
        defer { stage.close() }
        await stage.show()
        guard let table = findTable(in: stage.host) else { throw HarnessError("the Layers panel has no layer table") }
        guard row < table.numberOfRows else { return nil }
        if let layer = (table.delegate as? NativeLayerList.Coordinator)?.layer(for: row) {
            entry["layer"] = layer.name
        }
        // On the row's name, clear of the eye, the thumbnails and any effect rows below it.
        let rect = table.rect(ofRow: row)
        let point = table.convert(NSPoint(x: rect.maxX - 24, y: rect.minY + 17), to: nil)
        guard let event = NSEvent.mouseEvent(with: .rightMouseDown, location: point, modifierFlags: [],
                                             timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: stage.window.windowNumber,
                                             context: nil, eventNumber: 0, clickCount: 1, pressure: 1) else {
            throw HarnessError("couldn't make a right-click event")
        }
        guard let menu = table.menu(for: event) else { throw HarnessError("row \(row) has no menu") }
        return menu
    }

    private static func findTable(in view: NSView) -> LayerTableView? {
        if let table = view as? LayerTableView { return table }
        for child in view.subviews { if let table = findTable(in: child) { return table } }
        return nil
    }
}
