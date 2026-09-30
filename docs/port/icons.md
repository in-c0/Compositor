# Icons in the Windows app

The Mac app draws its icons with SF Symbols. Apple's license limits SF Symbols to Apple platforms, so the port can't ship them. It uses [Phosphor](https://phosphoricons.com) instead (MIT license), through the `egui-phosphor` crate, which embeds the regular and fill fonts. The mapping lives in `port/crates/app/src/icons.rs` (`phosphor()`); this page lists it so the substitutions can be reviewed in one place.

Phosphor glyphs fill more of their em square than SF Symbols do, so `icons::paint` draws them at 1.12 times the symbol's point size to match the Mac's optical size. The eyes are the exception: SF Symbols' `eye` is much wider than Phosphor's (18 against 14 points at 13 points, measured in the Layers panel), so `eye`, `eye.slash` and `eye.fill` are drawn 1.3 times larger again. They will never match SF Symbols pixel for pixel. `parity ui` (parity/README.md, "Comparing the UI") therefore doesn't gate on pixels: it reports each state's SSIM and share of far pixels, and the icon substitutions are part of what keeps those below a perfect score.

## Drawn by hand

The Mac draws four tool icons itself, with SwiftUI `Canvas` paths. The port copies those paths coordinate for coordinate, so these match the Mac's shapes:

| Mac view | Where | Port |
|---|---|---|
| `GradientToolIcon` | Gradient tool | `icons::gradient_tool`: the same 16 × 16 Floyd–Steinberg dither of a ramp in a rounded frame |
| `CloneStampToolIcon` | Clone Stamp tool | `icons::clone_stamp_tool` |
| `PolygonalLassoToolIcon` | Lasso tool in Polygonal mode | `icons::polygonal_lasso_tool` |
| `ObjectSelectionToolIcon` | Magic tool in Object mode | `icons::object_selection_tool` |

The three-dot preset button in the New canvas form and the color picker's hue arrows are also drawn directly, as on the Mac.

Two SF Symbols have no close Phosphor glyph, so the port draws them itself, sized from the Mac's tool rail at 17 points:

| SF Symbol | Where | Port |
|---|---|---|
| `rectangle.dashed` | Marquee tool (Rectangle) | `icons::rectangle_dashed`: a wide dashed rounded rectangle, 20 × 15 with its stroke. Phosphor's `selection` is square |
| `square.on.circle` | Shape tool | `icons::square_on_circle`: a circle behind a rounded square at its lower right. Phosphor's `shapes` is a triangle, circle and square |

## SF Symbol to Phosphor

"Fill" means the glyph comes from Phosphor's fill font.

| SF Symbol | Phosphor | Where |
|---|---|---|
| `arrow.up.left.and.arrow.down.right` | `arrows-out-simple` | Move tool |
| `circle.dashed` | `circle-dashed` | Marquee tool (Ellipse) |
| `lasso` | `lasso` | Lasso tool |
| `wand.and.stars` | `magic-wand` | Magic tool (Wand) |
| `crop` | `crop` | Crop tool |
| `paintbrush.pointed` | `paint-brush` | Brush tool |
| `eraser` | `eraser` | Brush tool in Erase mode |
| `bandage` | `bandaids` | Spot Healing tool |
| `seal` | `stamp` | Clone Stamp fallback (the rail draws the custom icon) |
| `drop` | `drop` | Smear tool |
| `square.bottomhalf.filled` | `gradient` | Gradient fallback (the rail draws the custom icon) |
| `textformat` | `text-aa` | Type tool; text layer thumbnails |
| `eyedropper` | `eyedropper` | Eyedropper tool; sampling buttons |
| `hand.draw` | `hand` | Hand tool |
| `magnifyingglass` | `magnifying-glass` | Zoom tool |
| `plus` / `minus` | `plus` / `minus` | New canvas button |
| `plus.magnifyingglass` / `minus.magnifyingglass` | `magnifying-glass-plus` / `magnifying-glass-minus` | Toolbar zoom buttons |
| `xmark` | `x` | Tab close button |
| `chevron.down` / `chevron.right` | `caret-down` / `caret-right` | Folder disclosure, tab overflow, footer menus |
| `chevron.up.chevron.down` | `caret-up-down` | Pop-up buttons (AppKit draws this; the port draws the glyph) |
| `arrow.left.and.right` | `arrows-left-right` | Swap colors (rotated 45°) |
| `arrow.counterclockwise` | `arrow-counter-clockwise` | Reset colors |
| `square.3.layers.3d` | `stack` | Empty Layers panel |
| `plus.square` | `plus-square` | Layers footer: new layer |
| `folder.badge.plus` | `folder-plus` | Layers footer: group |
| `rectangle.inset.filled` | `square-half` | Layers footer: add mask |
| `sparkles` | `sparkle` | Layers footer: effects |
| `circle.lefthalf.filled` | `circle-half` (fill) | Layers footer: adjustments; Hue/Saturation layer |
| `circle.righthalf.filled` | `circle-half-tilt` (fill) | Invert layer |
| `trash` | `trash` | Layers footer: delete |
| `eye` / `eye.slash` | `eye` / `eye-slash` | Layer and effect visibility |
| `eye.fill` | `eye` (fill) | Show-mask cursor (not used yet) |
| `link` | `link-simple` | Mask link (rotated 45°); Transform lock-ratio toggle |
| `folder` | `folder-simple` | Folder thumbnails |
| `scissors` | `scissors` | Move-pixels cursor (not used yet) |
| `text.alignleft` / `text.aligncenter` / `text.alignright` | `text-align-left` / `text-align-center` / `text-align-right` | Type alignment |
| `triangle.fill` | `triangle` (fill) | Camera Raw clipping indicators (Levels draws its own triangles) |
| `plus.circle.fill` / `minus.circle.fill` | `plus-circle` / `minus-circle` (fill) | Add and Remove eyedropper badges |
| `hand.point.up.left` | `hand-pointing` | Hue/Saturation targeted adjustment |
| `scope` | `crosshair` | Camera Raw targeted adjustment |
| `line.diagonal` | `line-segment` | Camera Raw Draw Guides |
| `multiply` | `x` | New canvas "×" |
| `circle` / `circle.fill` | `circle` / `circle` (fill) | Canvas Size anchor grid |
| `slider.horizontal.3` | `sliders-horizontal` | Levels layer |
| `point.topleft.down.to.point.bottomright.curvepath` | `bezier-curve` | Curves layer (rotated 90° as on the Mac) |
| `plusminus.circle` | `plus-minus` | Exposure layer |
| `paintpalette` | `palette` | Gradient Map layer |
| `circle.grid.3x3` | `dots-nine` | Grain layer |
| `circle.dotted` | `circle-dashed` | Add Noise layer |
| `drop.fill` | `drop` (fill) | Gaussian Blur layer |
| `wind` | `wind` | Motion Blur layer |
| `circle.filled.pattern.diagonalline.rectangle` | `checkerboard` | Black & White layer |
| `scale.3d` | `scales` | Color Balance layer |
| `arrow.turn.down.right` | `arrow-bend-down-right` | Clipping cursor (not used yet) |
| `rectangle.badge.plus` / `rectangle.badge.minus` | `selection-plus` / `selection-slash` | Clipping cursor badges (not used yet) |
| `arrow.triangle.2.circlepath` | `arrows-clockwise` | Rotate cursor (not used yet) |

`circle.dashed` and `circle.dotted` share `circle-dashed`, and `xmark` and `multiply` share `x`, because Phosphor has no closer glyph for the second of each pair.
