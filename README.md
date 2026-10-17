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

1. **Raster** (this): Typst -> `typst-render` pixmap -> egui texture, cached per
   snippet/colour/size/scale. Done.
2. **SVG**: compile to SVG and feed egui's image loaders; scales without re-rendering.
3. **Native vector**: walk Typst's laid-out frame (glyph outlines, rules, shapes) into egui `Mesh`
   triangles. Egui's path fill is convex-only and glyph outlines are not, so outlines get
   triangulated first (a fix inside `epaint` is the case for forking egui, if it comes to that).
4. Theme-aware colours without re-rendering, text selection and accessible alt text, inline math
   flowing with `RichText`.

## Versions

egui/eframe 0.37, Typst 0.15.

## Licence

MIT or Apache-2.0, at your option. Typst is Apache-2.0.
