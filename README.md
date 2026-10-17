# egui_typst

Typst math and markup in [egui](https://github.com/emilk/egui). Experimental, a side project.

Compile a Typst snippet once, cache it, draw it. Formulas follow the UI theme and the UI scale.

```rust
ui.add(egui_typst::Typst::new(
    r#"$ "TVaR"_alpha (L) = 1/(1-alpha) integral_alpha^1 "VaR"_u (L) dif u $"#,
).size(18.0));
```

A snippet that does not compile is shown as its error message instead of crashing; sizes and
scales are range-checked; Typst's file ids (interned for good, 65,535 at most) are reused through
one main file, so rendering thousands of snippets does not run out.

## Try it

```sh
cargo run --release --example gallery                                   # a window, light/dark switch
cargo run --release --example render_png -- '$ x^2 $' x.png --dark      # no window needed
```

## Plan

1. **Raster**: Typst -> `typst-render` pixmap -> egui texture, cached per
   snippet/colour/size/scale. Done (`Backend::Raster`).
2. **SVG** (the default, `Backend::Svg`): compile once to SVG with a placeholder text colour; a
   theme change swaps the colour in the string (no recompile) and egui's SVG loader (resvg)
   rasterises at the size and zoom on screen. Done. Checked against the raster two ways: the SVG
   rasterised with resvg differs from Typst's own raster by 0.7 to 2.1 of 255 in mean alpha on
   three real snippets, and through egui's loader (a frame run, no GPU) the uploaded texture has
   the right size, the requested colour on 95%+ of its visible pixels and the same ink as the
   raster within 1%.
3. **Native vector** (`Backend::Vector`): walk Typst's laid-out frame (glyph outlines, rules,
   shapes) into polygons, triangulate them with `lyon` (epaint fills convex shapes only and glyphs
   are not), and emit `egui::Mesh`es. Nothing in egui anti-aliases a raw mesh, so edges are
   feathered by hand: the opaque interior is inset by half a device pixel and a one-pixel ring
   ramps alpha from 1 to 0 across the true edge. Done, and no fork of egui was needed.
   Checked with a CPU rasteriser that draws a mesh the way a GPU does without multisampling
   (`egui_typst::debug`), against Typst's own raster, mean alpha difference out of 255 (four
   formulas; the range over them):

   | scale | hard edges | feathered |
   |---|---|---|
   | 1x | 7.6 to 19.0 | 2.8 to 7.6 |
   | 2x | 3.6 to 8.2 | 0.7 to 1.8 |
   | 3x | 2.7 to 6.1 | 0.45 to 1.2 |

   Feathering lowers the error 2.6x (1x) to 6x (3x) on the TVaR formula. (An earlier version of
   this table reported hard edges as 12 to 28 at every scale: `mesh_with_feather(0.0)` was drawing
   nothing, and the number was the error of a blank picture. Fixed, with a regression test.)
   At 2x and up (every Retina screen) the vector output is as close to Typst's raster as the SVG
   is. Small text at 1x is the weak case: strokes thinner than a pixel make the inset collapse.
   The half-pixel inset is the optimum (0.35 and 0.7 were measurably worse). Not drawn, and
   counted in `Vector::skipped`: images, gradients, tilings, colour glyphs, clipped groups, dashed
   strokes. Math needs none of them. `cargo run --release --example render_png -- '$ alpha $'
   a.png --backend vector --scale 14 --wire` shows the triangulation.
4. Theme-aware colours without re-rendering, text selection and accessible alt text, inline math
   flowing with `RichText`.

## Versions

egui/eframe 0.37, Typst 0.15.

## Licence

MIT or Apache-2.0, at your option. Typst is Apache-2.0.
