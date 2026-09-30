# Compositor for Windows (the port)

This folder holds the Windows version of Compositor, written in Rust. It draws everything with wgpu, so the same WGSL shaders run on DX12 on Windows and on Metal on a Mac. Every pixel it draws is checked against the Mac app's own rendering: see `../parity/README.md`.

## Crates

| Crate | What it does |
| --- | --- |
| `comp-format` | Reads and writes `.comp` projects, with the Mac app's validation rules and byte-identical manifests. |
| `engine` | The renderer: compositing, blend modes, masks, clipping, adjustments, effects, filters, document operations, export. All pixel work is WGSL compute shaders on packed 8-bit buffers. |
| `psd` | Reads and writes Photoshop PSD and PSB files. |
| `image-import` | Opens JPEG, PNG, HEIC, TIFF, SVG and DNG files as the Mac app imports them, including Core Image's conversion to 8-bit sRGB. Pure Rust: heic-rs decodes HEVC, resvg draws SVG, moxcms reads ICC profiles. |
| `parity` | Generates the test corpus and compares the port's renders with the Mac's references. |
| `lab` | A scratchpad for fitting the Mac's arithmetic to references before it goes into WGSL. |

## Building on Windows

You need Rust (stable) and the Visual Studio C++ build tools. DX12 compiles the engine's shaders with Microsoft's DirectX Shader Compiler, loaded from `dxcompiler.dll` and `dxil.dll` next to the executable. The older compiler Windows includes can't compile them. Fetch the two DLLs once, after your first build:

```
cargo build --release
powershell -File tools/fetch-dxc.ps1 -Into target/release
```

The script downloads one pinned DXC release and checks its SHA-256 before copying anything. If the port reports "no GPU adapter", the DLLs are usually missing from the folder the executable runs from.

On a Mac, `cargo build --release` is all you need; Metal needs nothing extra.

## Checking parity locally

Download the newest references from a CI run (the `references` artifact of the Parity workflow), then:

```
cargo run --release -p parity -- run --corpus ../parity/corpus --refs <references folder> --out <output folder> --platform windows
```

Add `--case 'blend/*'` to check one feature. The run prints a report and writes `results.json`, the port's renders, and a heatmap for every case that fails.
