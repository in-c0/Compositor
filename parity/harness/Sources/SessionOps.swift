import CoreGraphics
import Foundation

/// The filters whose result depends on a random seed. The app draws one each time the filter panel opens
/// (`FilterEdit.seed`, a `let` set from `UInt32.random`), so a reference made through `commitFilter()` could never be
/// reproduced. For these the harness takes the seed from the case and runs `parityCommitSeededFilter` instead.
nonisolated enum SeededFilter {
    static let kinds: [FilterKind] = [.addNoise, .grain]
}

/// The corpus ops, driven through the same `EditorSession` commands the app's menus and sheets call.
extension EditorSession {
    /// Filter > (kind)… with `layer` selected, the panel's settings set to `settings`, then OK.
    func parityApplyFilter(layer: UUID, kind: FilterKind, settings: FilterSettings, seed: UInt32?) async throws {
        guard let target = document?.layers.first(where: { $0.id == layer }) else {
            throw HarnessError("there's no layer \(layer.uuidString)")
        }
        selectLayer(layer)
        guard activeLayerID == layer else { throw HarnessError("layer “\(target.name)” couldn't be selected") }
        brushError = nil
        beginFilter(kind)
        guard let edit = filterEdit else {
            let why = kind == .contentAwareFill
                ? "Content-Aware Fill needs a selection, and no op makes one"
                : "filters need one visible pixel layer that isn't a folder, an adjustment layer or a mask"
            throw HarnessError(brushError ?? "the app won't open \(kind.rawValue) on layer “\(target.name)”: \(why)")
        }
        // The panel's controls call this on every change; preview stays on, as the panel opens.
        updateFilter(settings, preview: true)
        if let message = brushError {
            cancelFilter()
            throw HarnessError(message)
        }
        if let seed {
            await parityCommitSeededFilter(seed: seed)
        } else {
            await commitFilter()
        }
        if let message = brushError { throw HarnessError(message) }
        // commitFilter returns without applying when an automatic filter's preview failed.
        if filterEdit === edit {
            let reason = edit.previewError ?? "OK didn't apply it"
            cancelFilter()
            throw HarnessError("\(kind.rawValue): \(reason)")
        }
    }

    /// `commitFilter()` (Document/Filters.swift) for Add Noise and Grain, identical except that the job's seed comes
    /// from the case rather than `FilterEdit.seed`. Keep in step with commitFilter: neither filter spreads or is
    /// automatic, so its Remove Background, cached-preview and trimming branches don't apply here.
    func parityCommitSeededFilter(seed: UInt32) async {
        guard let edit = filterEdit, !edit.committing, SeededFilter.kinds.contains(edit.kind) else { return }
        if edit.kind == .grain && edit.settings.grain.amount == 0 { cancelFilter(); return }
        edit.committing = true
        edit.previewTask?.cancel()
        filterSettings = edit.settings
        isProjectBusy = true
        defer { filterEdit = nil; isProjectBusy = false; brushRevision += 1 }
        var job = FilterJob(kind: edit.kind, image: edit.grownImage ?? edit.original.image, settings: edit.renderSettings(), scale: 1,
                            selection: edit.selection, mapping: edit.mapping, seed: seed)
        job.canvas = edit.canvas
        let filterJob = job
        do {
            let grown = edit.grownTransform
            let made = try await Task.detached(priority: .userInitiated) { () -> (asset: ImportedImage, transform: LayerTransform?) in
                let image = try PixelFilter.run(filterJob)
                return (ImportedImage(image: image, thumbnail: try PixelAdjust.thumbnail(of: image), name: filterJob.kind.rawValue), grown)
            }.value
            let asset = made.asset
            guard let index = document?.layers.firstIndex(where: { $0.id == edit.layerID }),
                  let current = document?.layers[index],
                  current.asset?.image === edit.original.image || (edit.startedEmpty && current.asset == nil),
                  current.transform == edit.transform else { return }
            var mask = current.mask
            if let grown = edit.grownTransform, let owned = current.mask, owned.placement == nil,
               owned.asset.image.width > 1 || owned.asset.image.height > 1 {
                var enabled = owned
                enabled.isEnabled = true
                guard let carried = enabled.clipImage(placement: current.transform, over: grown,
                                                      width: asset.image.width, height: asset.image.height) else { throw ExportError.render }
                mask = owned.replacing(try LayerMask.asset(from: carried))
            }
            beginEdit(edit.kind.rawValue)
            document?.layers[index] = ImageLayer(id: current.id, asset: asset, name: current.name, isVisible: current.isVisible,
                transform: made.transform ?? current.transform, parentID: current.parentID, isGroup: false,
                opacity: current.opacity, blendMode: current.blendMode, mask: mask, maskSourceID: current.maskSourceID,
                effects: current.effects)
            endEdit()
        } catch { brushError = error.localizedDescription }
    }

    /// The Crop tool's frame set to `rect`, then Apply Crop.
    func parityCrop(_ rect: CGRect) async throws {
        guard document != nil else { throw HarnessError("there's no document to crop") }
        guard CropGeometry.valid(rect) else { throw HarnessError("the crop frame \(rect) is outside the app's limits") }
        guard canStartProjectOperation else { throw HarnessError("the session is busy") }
        cropError = nil
        cropRect = rect
        await commitCrop()
        if let message = cropError {
            cancelCrop()
            throw HarnessError(message)
        }
        guard cropRect == nil else {
            cancelCrop()
            throw HarnessError("Apply Crop didn't run")
        }
    }

    /// Image > Canvas Size…: `ProjectController.canvasSize()` from the sheet's OK onwards.
    func parityCanvasSize(_ options: CanvasSizeOptions) async throws {
        guard document != nil, canStartProjectOperation else { throw HarnessError("the session is busy or has no document") }
        cancelCrop()
        commitTransform()
        isProjectBusy = true
        defer { isProjectBusy = false }
        guard let snapshot = projectSnapshot() else { throw HarnessError("the session has no document") }
        let resized = try await CanvasResizer.shared.resize(snapshot, to: options)
        applyDocumentSize(resized, actionName: "Canvas Size")
    }

    /// Image > Image Size…: `ProjectController.imageSize()` from the sheet's OK onwards.
    func parityImageSize(width: Int, height: Int, resolution: Double?, sampling: LayerSampling) async throws {
        guard let document, canStartProjectOperation else { throw HarnessError("the session is busy or has no document") }
        cancelCrop()
        commitTransform()
        isProjectBusy = true
        defer { isProjectBusy = false }
        let options = ImageSizeOptions(width: width, height: height, resolution: resolution ?? document.resolution, sampling: sampling)
        guard let snapshot = projectSnapshot() else { throw HarnessError("the session has no document") }
        let resized = try await ImageResizer.shared.resize(snapshot, to: options)
        applyImageSize(resized)
    }
}
