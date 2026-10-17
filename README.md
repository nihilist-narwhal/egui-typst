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
3. **Native vector**: walk Typst's laid-out frame (glyph outlines, rules, shapes) into egui `Mesh`
   triangles. Egui's path fill is convex-only and glyph outlines are not, so outlines get
   triangulated first (a fix inside `epaint` is the case for forking egui, if it comes to that).
4. Theme-aware colours without re-rendering, text selection and accessible alt text, inline math
   flowing with `RichText`.

## Versions

egui/eframe 0.37, Typst 0.15.

## Licence

MIT or Apache-2.0, at your option. Typst is Apache-2.0.
