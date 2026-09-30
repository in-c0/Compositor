import AppKit
import SwiftUI

/// A window that can sit off screen, so the pointer is never over it (no hover states), and still be key, so
/// controls, table selections and prominent buttons draw as they do in the app's active window.
final class CaptureWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

/// Where capture windows go: far outside every screen.
let offscreenOrigin = NSPoint(x: -20_000, y: -20_000)

/// Lets AppKit and SwiftUI catch up for `seconds`, doing what `NSApp.run()` would: waiting events are handled (the
/// window server's activation and key-window changes arrive as events), the run loop runs (layout, display, timers),
/// and the main actor is released between slices so tasks the views started can finish.
func settle(_ seconds: Double) async {
    let end = Date().addingTimeInterval(seconds)
    repeat {
        while let event = NSApp.nextEvent(matching: .any, until: Date(), inMode: .default, dequeue: true) {
            NSApp.sendEvent(event)
        }
        CFRunLoopRunInMode(.defaultMode, 0.01, true)
        try? await Task.sleep(for: .milliseconds(10))
    } while Date() < end
}

/// A pixel capture of a view: 1 pixel per point, sRGB, composited over its window's background.
struct Snapshot {
    let image: NSBitmapImageRep
    /// The window's backing scale when it was taken. Content the window rasterized at a higher scale is scaled down
    /// to 1x, so a value other than 1 is worth knowing about when comparing.
    let backingScale: CGFloat
    let keyWindow: Bool

    /// `crop` is in points from the view's top left.
    @MainActor static func take(_ view: NSView, in window: NSWindow, crop: CGRect? = nil) throws -> Snapshot {
        view.layoutSubtreeIfNeeded()
        let bounds = view.bounds
        let width = Int(bounds.width.rounded()), height = Int(bounds.height.rounded())
        guard width > 0, height > 0 else { throw HarnessError("the view has no size (\(bounds.width) × \(bounds.height))") }
        // 1x whatever the screen: the bitmap's pixel size is its point size. Tagged sRGB before drawing, so colors are
        // converted to sRGB rather than to the display's profile.
        func bitmap() throws -> NSBitmapImageRep {
            guard let plain = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8,
                                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .calibratedRGB,
                                               bytesPerRow: 0, bitsPerPixel: 0),
                  let tagged = plain.retagging(with: .sRGB) else { throw HarnessError("couldn't make a \(width) × \(height) bitmap") }
            tagged.size = bounds.size
            return tagged
        }
        let drawn = try bitmap()
        view.cacheDisplay(in: bounds, to: drawn)
        // What the window shows behind the view: a hosting view is transparent wherever SwiftUI draws nothing.
        let flattened = try bitmap()
        guard let context = NSGraphicsContext(bitmapImageRep: flattened) else { throw HarnessError("couldn't draw into the bitmap") }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        context.imageInterpolation = .none
        window.effectiveAppearance.performAsCurrentDrawingAppearance {
            (window.backgroundColor ?? NSColor.windowBackgroundColor).setFill()
            NSRect(origin: .zero, size: bounds.size).fill()
        }
        _ = drawn.draw(in: NSRect(origin: .zero, size: bounds.size), from: .zero, operation: .sourceOver, fraction: 1,
                   respectFlipped: false, hints: [.interpolation: NSImageInterpolation.none.rawValue])
        NSGraphicsContext.restoreGraphicsState()
        var image = flattened
        if let crop {
            let pixels = CGRect(x: crop.minX, y: crop.minY, width: crop.width, height: crop.height).integral
                .intersection(CGRect(x: 0, y: 0, width: width, height: height))
            guard !pixels.isEmpty, let whole = flattened.cgImage, let cropped = whole.cropping(to: pixels) else {
                throw HarnessError("the crop \(crop) is outside the \(width) × \(height) view")
            }
            image = NSBitmapImageRep(cgImage: cropped)
        }
        return Snapshot(image: image, backingScale: window.backingScaleFactor, keyWindow: window.isKeyWindow)
    }

    func writePNG(to url: URL) throws {
        guard let data = image.representation(using: .png, properties: [:]) else { throw HarnessError("couldn't encode the PNG") }
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: .atomic)
    }
}

/// One view in its own key window, off screen, hosted as the app hosts it.
@MainActor final class Stage {
    let window: CaptureWindow
    let host: NSView

    /// `size` nil sizes the window to the view's fitting size, as `FloatingPanelController.show` does; otherwise the
    /// view gets exactly `size` and can't resize the window.
    init(_ root: AnyView, size: CGSize?) {
        let host = NSHostingView(rootView: root)
        // Toolbars and titles stay out of the capture window: only the content is compared.
        host.sceneBridgingOptions = []
        if size != nil { host.sizingOptions = [] }
        let window = CaptureWindow(contentRect: CGRect(origin: .zero, size: size ?? CGSize(width: 100, height: 100)),
                                   styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .darkAqua)
        window.contentView = host
        if let size {
            host.frame = CGRect(origin: .zero, size: size)
        } else {
            let fitting = host.fittingSize
            if fitting.width > 0, fitting.height > 0 { window.setContentSize(fitting) }
        }
        window.setFrameOrigin(offscreenOrigin)
        self.window = window
        self.host = host
    }

    /// Orders the window in as the key window, lets the view settle, then takes focus away from any field that
    /// grabbed it on appear, so no caret or focus ring is captured.
    func show(settling seconds: Double = 0.4) async {
        window.makeKeyAndOrderFront(nil)
        await settle(seconds)
        _ = window.makeFirstResponder(nil)
        await settle(0.2)
    }

    func snapshot(crop: CGRect? = nil) throws -> Snapshot {
        try Snapshot.take(host, in: window, crop: crop)
    }

    func close() {
        window.orderOut(nil)
        window.contentView = nil
        window.close()
    }
}
