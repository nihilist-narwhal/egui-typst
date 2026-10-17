//! What a colour or scale change costs, counted in Typst compilations. One test in its own
//! process: the counter is global, so nothing else may compile meanwhile.

use std::error::Error;

use egui::{Color32, Context, Pos2, RawInput, Rect, vec2};
use egui_typst::debug::compile_count;
use egui_typst::{Backend, Typst};

const SNIPPET: &str = r#"$ "TVaR"_alpha (L) = 1/(1-alpha) integral_alpha^1 "VaR"_u (L) dif u $"#;

fn frames(ctx: &Context, backend: Backend, color: Color32, n: usize) {
    for _ in 0..n {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 400.0))),
            ..RawInput::default()
        };
        let _ = ctx.run_ui(input, |ui| {
            ui.add(Typst::new(SNIPPET).size(20.0).color(color).backend(backend));
        });
    }
}

#[test]
fn compilations_per_backend_when_the_colour_or_scale_changes() -> Result<(), Box<dyn Error>> {
    let ctx = Context::default();
    ctx.set_pixels_per_point(2.0);
    let (black, red) = (Color32::BLACK, Color32::from_rgb(255, 0, 0));
    let spent = |ctx: &Context, backend, color, what: &str| {
        let before = compile_count();
        frames(ctx, backend, color, 4);
        let n = compile_count() - before;
        println!("{backend:?}: {what}: {n} compile(s)");
        n
    };

    // SVG: one compile, then the colour is a string swap and the scale is the loader's business
    assert_eq!(spent(&ctx, Backend::Svg, black, "first draw, 4 frames"), 1);
    assert_eq!(spent(&ctx, Backend::Svg, red, "new colour"), 0);
    ctx.set_pixels_per_point(3.0);
    assert_eq!(spent(&ctx, Backend::Svg, red, "new scale"), 0);

    // vector: one compile, then a colour or scale change only rebuilds the mesh
    ctx.set_pixels_per_point(2.0);
    assert_eq!(
        spent(&ctx, Backend::Vector, black, "first draw, 4 frames"),
        1
    );
    assert_eq!(spent(&ctx, Backend::Vector, red, "new colour"), 0);
    ctx.set_pixels_per_point(3.0);
    assert_eq!(spent(&ctx, Backend::Vector, red, "new scale"), 0);

    // raster: the colour and the scale are baked into the pixels, so each change recompiles
    ctx.set_pixels_per_point(2.0);
    assert_eq!(
        spent(&ctx, Backend::Raster, black, "first draw, 4 frames"),
        1
    );
    assert_eq!(spent(&ctx, Backend::Raster, red, "new colour"), 1);
    ctx.set_pixels_per_point(3.0);
    assert_eq!(spent(&ctx, Backend::Raster, red, "new scale"), 1);
    Ok(())
}
