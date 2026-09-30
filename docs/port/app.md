# The Windows app

`port/crates/app` builds `compositor`, the editor window around the engine. It follows the Mac app's layout and metrics from [ui-inventory.md](ui-inventory.md): the tool header, the 56-point tool rail, the canvas, the Layers panel and the status bar, with the same labels, defaults and hint text. It uses eframe/egui on wgpu, and the engine composites on the same wgpu device that draws the window (DX12 on Windows, Metal on the Mac).

## Building and running

The engine compiles its shaders for DX12 with Microsoft's DXC, which the app loads at run time from `dxcompiler.dll` and `dxil.dll` next to `compositor.exe`. Fetch them once before the first run:

```
pwsh port/tools/fetch-dxc.ps1
```

That puts both DLLs in `port/target/release` and `port/target/debug`, where Cargo writes the executable, so `cargo run` works from then on:

```
cd port
cargo run -p app -- ../parity/corpus/blend/stack/input.comp
```

The argument is optional. A `.comp` project is a folder, so File > Open Project asks for a folder.

If the app fails at startup with a DX12 shader error, the DLLs are missing from the executable's folder. Run `fetch-dxc.ps1` again, or pass `-Into <folder>` to put them somewhere else.

To make a folder you can copy to another machine, run:

```
pwsh port/tools/package-app.ps1
```

It builds the release app and puts `compositor.exe`, `dxcompiler.dll` and `dxil.dll` in `port/target/dist/compositor` (`-Out` picks another folder). All three files have to stay together.

On the Mac no DLLs are needed: wgpu compiles to Metal.

## What works

Every edit goes through the engine's own code, the same operations the parity cases run (`Renderer::apply_op`, `paint::apply`, `filters::apply`, the `select` ops, `export_jpeg`, `import_image`, `psd::import_file`), so what the canvas shows is what parity checks. While a drag or a sheet is in progress the canvas shows a preview of the result, and the document itself only changes when the edit is committed.

- Opening a `.comp` from File > Open Project or the command line, in a new project tab. The canvas is the engine's composite (`engine::composite::Compositor`), fitted to the window when opened. Several projects can be open at once, one per tab.
- Zoom and pan: Fit, 100%, Zoom In and Zoom Out (menu, toolbar and Ctrl+0, Ctrl+1, Ctrl+=, Ctrl+-), Ctrl-scroll or pinch to zoom at the pointer, scrolling to pan, the Hand tool, Space-drag and middle-drag, and the Zoom tool (click, Alt-click, or drag left and right). From 200% the canvas shows hard-edged pixels, and from 800% the pixel grid, as on the Mac.
- Rulers (View > Rulers, Ctrl+R), and the project's guides (View > Show > Guides, Clear Guides).
- Undo and Redo for every edit, with the Mac's action names. An undo step keeps the manifest, the active layer, the selection and only the pixels the edit changed. The title and the tab show when a project has unsaved changes.
- Save and Save As write the project through `comp_format` after `engine::session::normalize`, at the current format version, as the Mac's `projectSnapshot` does. Export PNG writes the flattened image, unpremultiplied as the Mac exports it. Export JPEG opens its sheet with a preview of the encoded file, the quality and the background color for transparency.
- File > Import Images: an image becomes a new layer centered on the canvas, or a new project when none is open. A Photoshop file opens the conversion report first when it has something to convert, then comes in as a new project, or as a folder named after the file inside the open one.
- Move tool: dragging moves the layer (Shift keeps it on one axis, Alt drags a copy), the handles resize it (Shift and the lock toggle keep or free the aspect ratio, Alt resizes from the center, dragging past the opposite side flips it) and the circle above rotates it (Shift in 15° steps). Moving and resizing snap to the canvas's edges and center, the other layers and the guides, within 10 points on screen; Ctrl drags freely. Auto Select picks the layer under the pointer. The Transform header's X, Y, W, H, Scale and angle fields, Sampling and Flip H and Flip V all edit the active layer. A linked mask moves with its layer.
- Crop tool: drag a frame (Alt from the center), move it or drag its handles, with the Ratio choices and snapping, then Apply Crop or Return. Image > Canvas Size, Image Size and Trim, and Image > Flip Canvas.
- Layers: New Blank Layer, New Folder, Duplicate, Delete, Rename, Group and Ungroup, Move Out of Folder, Merge Down and Merge Group, Create and Release Clipping Mask (also Alt-clicking a row), Flip Layer, and adjustment layers from the Layer menu or the footer. Masks: Add Mask (the footer's button, Alt for a black one), Delete, Enable and Disable (Shift-clicking the mask), Link and Unlink (the link icon), Invert. Clicking a mask's thumbnail targets the mask for painting, Invert and Delete. Double-clicking an adjustment layer opens its sheet, and double-clicking any other layer renames it. The footer's effects menu adds and edits layer effects.
- Sheets for the Filter menu and the Image menu's adjustments, each with the Mac's controls and a live preview: Levels (with the histogram and the Black, Gray and White samplers), Curves, Hue/Saturation, Exposure, Gradient Map, Grain, Black & White, Color Balance, Add Noise, Vignette, Dither, Lens Correction and Remove Background. The same sheets edit adjustment layers. Gaussian Blur, Motion Blur, Bloom / Glow and Tonal Contrast have sheets too, with the engine's exact Core Image blurs.
- Selections: the Marquee (rectangle and ellipse, Shift for a square mid-drag), the Lasso and the Polygonal Lasso (Return or a double-click closes it, Backspace removes a corner, Escape cancels), and the Magic Wand, with New, Add (Shift) and Subtract (Alt). Dragging inside a selection moves its outline. Select > All, Deselect, Inverse, Layer's Pixels, Mask's Black Areas, Color Range, and Expand, Contract and Feather (the menu's sheet or the header's buttons). The selection shows as marching ants.
- Painting: Brush and Eraser, Spot Healing, Clone Stamp (Alt-click sets the source) and the Smear tool's Blur, Smudge and Liquify, with every header setting, the brush circle, `[` and `]` for the size, Shift-`[` and Shift-`]` for the hardness, right-drag to size the brush, the digit keys for the opacity, and Shift for a straight line on from the last stroke. With a mask targeted the brush paints Black · Hide or White · Reveal. A blank layer gets clear pixels its own size before the first stroke, as the Mac does.
- Edit > Fill with Foreground and Background Color, and Delete with a selection clears the selected pixels.
- The Eyedropper (or Alt with a painting tool) samples the canvas into the foreground color. Clicking the rail's swatches or the brush's Color opens a color picker, which also samples the canvas when it's clicked.
- Keyboard: the menu shortcuts (Ctrl for ⌘, Alt for ⌥), the tool letters, X and D for the colors, Tab to step the tool's mode, Shift-U for the shape, Space to pan, Delete, Return and Escape, the arrows to nudge with the Move tool, and Shift-minus and Shift-equals to step the blend mode. Edit > Keyboard Shortcuts lists them.

## What waits on the engine

Menu items whose feature doesn't exist yet are in place but disabled, so the menus match the Mac's order and shortcuts. That covers Cut, Copy and Paste, the grid, printing and the Camera Raw Filter.

A few things work in the app but aren't what the Mac does exactly, because the engine has no operation for them yet:

- The engine's filters and strokes don't take a selection, so the app limits them to the selection itself, blending the result over the layer through the selection's coverage. This isn't checked against the Mac, and it only works on layers placed 1:1 and upright.
- Image > Invert and Edit > Fill run the Mac's arithmetic in the app; Levels and Hue/Saturation on a layer's pixels run the adjustment layers' kernels over them. No parity case covers these yet.
- Select > Subject and the Magic tool's Object mode, and Content-Aware Fill, report that the engine can't do them yet. So does Image Size on a layer whose mask it would redraw at a rotation.
- The Gradient, Shape and Type tools show their headers but say they aren't available when used: their engine code isn't on this branch yet.

When a project uses something the compositor can't draw yet, the canvas stays empty and a badge at the bottom says what isn't supported. When an operation can't be done exactly, the app says so instead of doing it approximately.

## Rendering the UI for parity

```
compositor --render-ui parity/ui/states.toml --corpus parity/corpus --out <dir>
```

This renders each state in `states.toml` offscreen at 1x and writes `<dir>/<id>.png`, `menus.json` and `ui-info.json`. The conventions are in [parity/README.md](../../parity/README.md#ui-states). It also writes `<dir>/port/window.png`, the whole Windows window with its menu bar and toolbar, which has no Mac counterpart.
