//! The native vector backend, checked without a GPU: the meshes are rasterised on the CPU the way
//! a GPU would draw them without multisampling (a pixel is drawn when its centre is inside a
//! triangle, with the vertex colours interpolated and blended premultiplied) and compared with
//! Typst's own raster of the same snippet.

use std::error::Error;

use egui::Color32;
use egui_typst::debug::rasterise_alpha;
use egui_typst::{Compiler, Style};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const SNIPPETS: &[&str] = &[
    r#"$ "TVaR"_alpha (L) = 1/(1-alpha) integral_alpha^1 "VaR"_u (L) dif u $"#,
    "$ Sigma = mat(1, rho_(12); rho_(12), 1) $",
    r#"Loss $L = -"result"$ has $"CoV" = sigma / mu$."#,
    "$ sqrt(x^2 + y^2) >= 0 $",
];

/// Mean absolute alpha difference against Typst's raster (out of 255), over the raster's pixels.
fn difference(compiler: &Compiler, snippet: &str, ppp: f32, feather: Option<f32>) -> Result<f64> {
    let style = Style {
        pixels_per_point: ppp,
        ..Style::default()
    };
    let reference = compiler.render(snippet, &style)?;
    let vector = compiler.vector(snippet, style.size_pt, style.margin_pt)?;
    let mesh = match feather {
        Some(f) => vector.mesh_with_feather(f, Color32::BLACK),
        None => vector.mesh(ppp, Color32::BLACK),
    };
    let (w, h) = (reference.size[0], reference.size[1]);
    let drawn = rasterise_alpha(&mesh, ppp, w, h);
    let total: f64 = drawn
        .iter()
        .zip(&reference.pixels)
        .map(|(a, b)| f64::from((a - f32::from(b.a())).abs()))
        .sum();
    Ok(total / (w * h) as f64)
}

#[test]
fn the_vector_meshes_match_the_raster_and_feathering_helps() -> Result<()> {
    let compiler = Compiler::new()?;
    println!(
        "{:<44} {:>5} {:>10} {:>10}",
        "snippet", "ppp", "hard", "feathered"
    );
    for snippet in SNIPPETS {
        for ppp in [1.0_f32, 2.0, 3.0] {
            let hard = difference(&compiler, snippet, ppp, Some(0.0))?;
            let soft = difference(&compiler, snippet, ppp, None)?;
            println!("{:<44.44} {ppp:>5} {hard:>10.3} {soft:>10.3}", snippet);
            assert!(
                soft < hard,
                "feathering should reduce the error ({soft:.3} vs {hard:.3}) for {snippet} at {ppp}x"
            );
            // 2x and up (every Retina screen) is within 2.5/255; small text at 1x is the weak
            // case, where strokes thinner than a pixel defeat the half-pixel inset
            let limit = if ppp >= 2.0 { 2.5 } else { 8.0 };
            assert!(
                soft < limit,
                "{snippet} at {ppp}x differs from the raster by {soft:.3}/255"
            );
        }
    }
    Ok(())
}

#[test]
fn math_needs_nothing_the_vector_backend_skips() -> Result<()> {
    let compiler = Compiler::new()?;
    for snippet in SNIPPETS {
        let vector = compiler.vector(snippet, 14.0, 2.0)?;
        assert_eq!(vector.skipped, 0, "{snippet}");
        assert!(
            vector.contour_count() > 3,
            "{snippet}: {} contours",
            vector.contour_count()
        );
    }
    Ok(())
}

#[test]
fn hostile_input_is_an_error_not_a_panic() -> Result<()> {
    let compiler = Compiler::new()?;
    assert!(compiler.vector("$ unclosed", 14.0, 2.0).is_err());
    assert!(compiler.vector("$x$", f32::NAN, 2.0).is_err());
    // an enormous formula either renders within the caps or is refused
    let big = format!("$ {} $", "x + ".repeat(3_000) + "y");
    let _ = compiler.vector(&big, 14.0, 2.0);
    // hostile feathers are clamped
    let vector = compiler.vector("$x$", 14.0, 2.0)?;
    let _ = vector.mesh_with_feather(f32::NAN, Color32::BLACK);
    let _ = vector.mesh_with_feather(f32::INFINITY, Color32::BLACK);
    let _ = vector.mesh_with_feather(-1.0, Color32::BLACK);
    Ok(())
}

#[test]
fn a_hard_edged_mesh_still_draws_the_glyphs() -> Result<()> {
    // `mesh_with_feather(0.0)` once drew nothing (a zero tolerance made lyon refuse the fill), so
    // the "hard edge" error was just the error of a blank picture: it did not change with scale.
    // A real hard-edged mesh covers about as much ink as the reference, and its error shrinks as
    // the scale grows.
    let compiler = Compiler::new()?;
    let snippet = SNIPPETS[0];
    let mut previous = f64::MAX;
    for ppp in [1.0_f32, 2.0, 3.0] {
        let style = Style {
            pixels_per_point: ppp,
            ..Style::default()
        };
        let reference = compiler.render(snippet, &style)?;
        let vector = compiler.vector(snippet, style.size_pt, style.margin_pt)?;
        let mesh = vector.mesh_with_feather(0.0, Color32::BLACK);
        let drawn = rasterise_alpha(&mesh, ppp, reference.size[0], reference.size[1]);
        let ink: f64 = drawn.iter().map(|a| f64::from(*a)).sum();
        let want: f64 = reference.pixels.iter().map(|p| f64::from(p.a())).sum();
        assert!(
            ink > 0.7 * want && ink < 1.3 * want,
            "{ppp}x: {ink:.0} ink against {want:.0}"
        );
        let error = difference(&compiler, snippet, ppp, Some(0.0))?;
        assert!(
            error < previous,
            "{ppp}x: the hard-edge error should shrink with scale ({error:.3} after {previous:.3})"
        );
        previous = error;
    }
    Ok(())
}
