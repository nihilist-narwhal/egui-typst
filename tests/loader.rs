//! Through egui itself, no GPU: running a frame returns the texture uploads egui would send to
//! the renderer, and those hold the pixels the SVG loader (egui_extras + resvg) or the raster
//! backend produced.

use std::error::Error;

use egui::{Color32, ColorImage, Context, ImageData, Pos2, RawInput, Rect, Shape, TextureId, vec2};
use egui_typst::{Backend, Compiler, Typst};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const SNIPPET: &str = r#"$ "TVaR"_alpha (L) = 1/(1-alpha) integral_alpha^1 "VaR"_u (L) dif u $"#;

/// Runs a few frames of `Typst::new(SNIPPET)` and returns the colour images uploaded other than
/// the font atlas (managed texture 0).
fn uploads(ctx: &Context, backend: Backend, color: Color32, frames: usize) -> Vec<ColorImage> {
    let mut images = Vec::new();
    for _ in 0..frames {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 400.0))),
            ..RawInput::default()
        };
        let output = ctx.run_ui(input, |ui| {
            ui.add(Typst::new(SNIPPET).size(24.0).color(color).backend(backend));
        });
        for (id, deltas) in &output.textures_delta.set {
            if *id == TextureId::Managed(0) {
                continue;
            }
            for delta in deltas {
                let ImageData::Color(image) = &delta.image;
                images.push((**image).clone());
            }
        }
    }
    images
}

/// The visible pixels of a premultiplied image, un-premultiplied to (r, g, b) at alpha >= 128.
fn visible_colours(image: &ColorImage) -> Vec<[u8; 3]> {
    image
        .pixels
        .iter()
        .filter(|p| p.a() >= 128)
        .map(|p| {
            let [r, g, b, _] = p.to_srgba_unmultiplied();
            [r, g, b]
        })
        .collect()
}

#[test]
fn the_svg_backend_uploads_a_sharp_correctly_coloured_texture() -> Result<()> {
    let ctx = Context::default();
    ctx.set_pixels_per_point(2.0);
    let red = Color32::from_rgb(255, 0, 0);
    let images = uploads(&ctx, Backend::Svg, red, 6);
    let image = images
        .iter()
        .max_by_key(|i| i.size[0] * i.size[1])
        .ok_or("the svg loader uploaded no texture")?;

    // the page is the snippet's size in points; the loader rasterises it at 2 px per point
    let svg = Compiler::new()?.svg(SNIPPET, 24.0, 2.0)?;
    let expected = [svg.size_pt[0] * 2.0, svg.size_pt[1] * 2.0];
    assert!(
        (image.size[0] as f32 - expected[0]).abs() <= 2.0
            && (image.size[1] as f32 - expected[1]).abs() <= 2.0,
        "uploaded {:?}, expected about {expected:?}",
        image.size
    );

    let visible = visible_colours(image);
    assert!(visible.len() > 200, "{} visible pixels", visible.len());
    let reddish = visible
        .iter()
        .filter(|[r, g, b]| *r > 200 && *g < 60 && *b < 60)
        .count();
    assert!(
        reddish * 100 >= visible.len() * 95,
        "{reddish} of {} visible pixels are the requested red",
        visible.len()
    );
    Ok(())
}

#[test]
fn a_new_colour_uploads_the_new_colour() -> Result<()> {
    let ctx = Context::default();
    ctx.set_pixels_per_point(1.0);
    uploads(&ctx, Backend::Svg, Color32::BLACK, 4);
    let blue = Color32::from_rgb(0, 0, 255);
    let images = uploads(&ctx, Backend::Svg, blue, 4);
    let image = images
        .iter()
        .max_by_key(|i| i.size[0] * i.size[1])
        .ok_or("no texture for the new colour")?;
    let visible = visible_colours(image);
    let bluish = visible
        .iter()
        .filter(|[r, g, b]| *b > 200 && *r < 60 && *g < 60)
        .count();
    assert!(
        !visible.is_empty() && bluish * 100 >= visible.len() * 95,
        "{bluish} of {} visible pixels are blue",
        visible.len()
    );
    Ok(())
}

#[test]
fn both_backends_draw_about_the_same_ink() -> Result<()> {
    let ink = |backend| -> Result<f64> {
        let ctx = Context::default();
        ctx.set_pixels_per_point(2.0);
        let images = uploads(&ctx, backend, Color32::BLACK, 6);
        let image = images
            .iter()
            .max_by_key(|i| i.size[0] * i.size[1])
            .ok_or("no texture")?;
        Ok(image.pixels.iter().map(|p| f64::from(p.a())).sum::<f64>() / 255.0)
    };
    let (svg, raster) = (ink(Backend::Svg)?, ink(Backend::Raster)?);
    println!("ink (sum of alpha): svg {svg:.0}, raster {raster:.0}");
    assert!(
        (svg - raster).abs() / raster < 0.03,
        "svg {svg:.0} vs raster {raster:.0}"
    );
    Ok(())
}

#[test]
fn the_vector_backend_paints_a_mesh_in_the_requested_colour() -> Result<()> {
    let ctx = Context::default();
    ctx.set_pixels_per_point(2.0);
    let red = Color32::from_rgb(255, 0, 0);
    let mut meshes = Vec::new();
    for _ in 0..3 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 400.0))),
            ..RawInput::default()
        };
        let output = ctx.run_ui(input, |ui| {
            ui.add(
                Typst::new(SNIPPET)
                    .size(24.0)
                    .color(red)
                    .backend(Backend::Vector),
            );
        });
        meshes = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                Shape::Mesh(mesh) => Some(mesh.clone()),
                _ => None,
            })
            .collect();
    }
    let mesh = meshes.first().ok_or("no mesh was painted")?;
    assert!(
        mesh.vertices.len() > 200,
        "{} vertices",
        mesh.vertices.len()
    );

    // opaque vertices are the requested colour; the ring's outer vertices are transparent
    let opaque = mesh.vertices.iter().filter(|v| v.color.a() == 255).count();
    assert!(opaque > 100);
    assert!(
        mesh.vertices
            .iter()
            .filter(|v| v.color.a() == 255)
            .all(|v| v.color == red)
    );

    // it sits inside the widget's rectangle (the page size from the top-left of the panel)
    let svg = Compiler::new()?.svg(SNIPPET, 24.0, 2.0)?;
    let bounds = mesh.calc_bounds();
    assert!(
        bounds.width() <= svg.size_pt[0] + 1.0 && bounds.height() <= svg.size_pt[1] + 1.0,
        "{bounds:?} vs {:?}",
        svg.size_pt
    );
    assert!(
        bounds.width() > svg.size_pt[0] * 0.8,
        "the formula fills most of its page: {bounds:?} vs {:?}",
        svg.size_pt
    );
    Ok(())
}
