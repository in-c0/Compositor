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

- Opening a `.comp` from File > Open Project or the command line, in a new project tab. The canvas is the engine's composite (`engine::composite::Compositor`), fitted to the window when opened.
- Zoom and pan: Fit, 100%, Zoom In and Zoom Out (menu, toolbar and Ctrl+0, Ctrl+1, Ctrl+=, Ctrl+-), Ctrl-scroll or pinch to zoom at the pointer, scrolling to pan, the Hand tool, Space-drag and middle-drag. From 200% the canvas shows hard-edged pixels, and from 800% the pixel grid, as on the Mac.
- Rulers (View > Rulers, Ctrl+R).
- Layers panel edits, each re-rendered through the compositor and undoable: visibility (eye), blend mode, opacity (slider, field or scrubbing the label), reordering by dragging a row within its folder, Move Layer Up and Down, and Enable/Disable Mask from the row's context menu. With the Move tool, the digit keys set opacity and the arrow keys nudge the layer; the X and Y fields in the Transform header move it too.
- Undo and Redo for those edits.
- Save and Save As write the project through `comp_format` after `engine::session::normalize`, as the Mac's `projectSnapshot` does. Export PNG writes the flattened image, unpremultiplied as the Mac exports it.
- New Canvas: the welcome form shows when no project is selected, and Create canvas makes an empty project with one blank "Layer 1".
- Tool selection from the rail and the tool keys, with each tool's header and status bar hint.

## What waits on the engine

Menu items whose feature doesn't exist yet are in place but disabled, so the menus match the Mac's order and shortcuts. That covers painting, selections, filters and adjustments, transforms other than moving, layer creation and deletion, masks other than enabling and disabling them, import, JPEG export, printing, the grid and guides.

The tool headers show every control with the Mac's labels, ranges and defaults. Their settings are kept but don't do anything until the tools exist. Transform fields other than X and Y are disabled for the same reason.

When a project uses something the compositor can't draw yet, such as layer effects or transformed layers, the canvas stays empty and a badge at the bottom says what isn't supported. Nothing is drawn approximately.

## Rendering the UI for parity

```
compositor --render-ui parity/ui/states.toml --corpus parity/corpus --out <dir>
```

This renders each state in `states.toml` offscreen at 1x and writes `<dir>/<id>.png`, `menus.json` and `ui-info.json`. The conventions are in [parity/README.md](../../parity/README.md#ui-states). It also writes `<dir>/port/window.png`, the whole Windows window with its menu bar and toolbar, which has no Mac counterpart.
