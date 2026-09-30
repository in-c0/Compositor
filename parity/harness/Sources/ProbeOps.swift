import CoreGraphics
import Foundation

/// A probe of Core Graphics itself rather than of an app command: a layer's image drawn once with `CGContext.draw`
/// into a fresh bitmap, which then replaces the layer's pixels. The port measures Core Graphics' resampling from
/// these, with every parameter of the draw under the case's control.
struct ProbeDraw {
    let layer: UUID
    /// The bitmap: 8-bit premultiplied RGBA in sRGB, cleared to transparent, as `BrushRaster.context` makes them.
    let width: Int
    let height: Int
    /// Concatenated onto the context's own (y-up) space before drawing, `[a, b, c, d, tx, ty]`.
    let transform: CGAffineTransform
    /// Where the image is drawn, in that space.
    let rect: CGRect
    let quality: CGInterpolationQuality
    let antialias: Bool

    static func parse(_ fields: JSONFields, path: String) throws -> ProbeDraw {
        let layerText = try JSONValue.string(fields.required("layer"), "\(path).layer")
        guard let layer = UUID(uuidString: layerText) else { throw HarnessError("\(path).layer “\(layerText)” isn't a UUID") }
        let width = try JSONValue.integer(fields.required("width"), "\(path).width")
        let height = try JSONValue.integer(fields.required("height"), "\(path).height")
        guard (1...4096).contains(width), (1...4096).contains(height) else { throw HarnessError("\(path): the bitmap must be 1–4096 pixels a side") }
        let r = try JSONValue.numbers(fields.required("rect"), count: 4, "\(path).rect")
        let t = try fields.optional("transform") { try JSONValue.numbers($0, count: 6, $1) } ?? [1, 0, 0, 1, 0, 0]
        let qualities: [String: CGInterpolationQuality] = ["none": .none, "low": .low, "medium": .medium, "high": .high, "default": .default]
        let name = try fields.optional("quality", JSONValue.string) ?? "high"
        guard let quality = qualities[name] else { throw HarnessError("\(path).quality “\(name)” isn't one of \(qualities.keys.sorted().joined(separator: ", "))") }
        return ProbeDraw(layer: layer, width: width, height: height,
                         transform: CGAffineTransform(a: t[0], b: t[1], c: t[2], d: t[3], tx: t[4], ty: t[5]),
                         rect: CGRect(x: r[0], y: r[1], width: r[2], height: r[3]), quality: quality,
                         antialias: try fields.optional("antialias", JSONValue.bool) ?? true)
    }

    func draw(_ image: CGImage) throws -> CGImage {
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else {
            throw HarnessError("couldn't make a \(width)x\(height) bitmap")
        }
        context.interpolationQuality = quality
        context.setShouldAntialias(antialias)
        context.concatenate(transform)
        context.draw(image, in: rect)
        guard let result = context.makeImage() else { throw HarnessError("couldn't read the bitmap back") }
        return result
    }
}

extension EditorSession {
    /// Replaces `probe.layer`'s pixels with its image drawn as `probe` says, the layer placed 1:1 at the top left.
    func parityProbeDraw(_ probe: ProbeDraw) async throws {
        guard let index = document?.layers.firstIndex(where: { $0.id == probe.layer }), let current = document?.layers[index],
              let asset = current.asset else { throw HarnessError("there's no pixel layer \(probe.layer.uuidString)") }
        let image = try probe.draw(asset.image)
        let drawn = ImportedImage(image: image, thumbnail: try PixelAdjust.thumbnail(of: image), name: current.name)
        var transform = current.transform
        transform.origin = .zero
        transform.size = CGSize(width: probe.width, height: probe.height)
        transform.rotation = 0
        transform.flipX = false
        transform.flipY = false
        var layer = current
        layer.asset = drawn
        layer.transform = transform
        beginEdit("Probe")
        document?.layers[index] = layer
        endEdit()
    }
}
