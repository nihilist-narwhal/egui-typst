//! `cargo run --release --example timings`: what each step costs on this machine, so the numbers
//! in the manual can be reproduced. Wall-clock averages; indicative, not a benchmark.

use std::time::Instant;

use egui::Color32;
use egui_typst::{Compiler, Style};

const FORMULA: &str = r#"$ "TVaR"_alpha (L) = 1/(1-alpha) integral_alpha^1 "VaR"_u (L) dif u $"#;

/// The formula with a different tail each time, so Typst's memoisation (comemo) cannot answer
/// from a previous run: this is what compiling a snippet the first time costs.
fn fresh(i: u32) -> String {
    format!(r#"$ "TVaR"_alpha (L) = 1/(1-alpha) integral_alpha^1 "VaR"_u (L) dif u + {i} $"#)
}

fn average<T>(runs: u32, mut f: impl FnMut() -> T) -> f64 {
    let start = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(f());
    }
    start.elapsed().as_secs_f64() * 1e3 / f64::from(runs)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let t = Instant::now();
    let compiler = Compiler::new()?;
    println!(
        "{:>9.2} ms  Compiler::new (fonts, library; once per program)",
        t.elapsed().as_secs_f64() * 1e3
    );

    let style = Style {
        pixels_per_point: 2.0,
        ..Style::default()
    };
    // warm up: the first compile fills Typst's caches (fonts, layout of common glyphs)
    compiler.render(&fresh(0), &style)?;
    let n = 50;
    let mut i = 1;
    let mut next = || {
        i += 1;
        fresh(i)
    };
    println!(
        "{:>9.2} ms  compile + rasterise at 2x, a new snippet (Backend::Raster)",
        average(n, || compiler.render(&next(), &style))
    );
    println!(
        "{:>9.2} ms  the same snippet again (Typst's memoisation answers)",
        average(n, || compiler.render(FORMULA, &style))
    );
    let svg = compiler.svg(FORMULA, 14.0, 2.0)?;
    println!(
        "{:>9.2} ms  compile + SVG export, a new snippet (Backend::Svg)",
        average(n, || compiler.svg(&next(), 14.0, 2.0))
    );
    println!(
        "{:>9.4} ms  recolour the SVG (a theme change)",
        average(n * 20, || svg.tinted(Color32::RED))
    );
    let vector = compiler.vector(FORMULA, 14.0, 2.0)?;
    println!(
        "{:>9.2} ms  compile + walk the frame, a new snippet (Backend::Vector)",
        average(n, || compiler.vector(&next(), 14.0, 2.0))
    );
    for ppp in [1.0_f32, 2.0, 3.0] {
        let mesh = vector.mesh(ppp, Color32::BLACK);
        println!(
            "{:>9.2} ms  triangulate + feather at {ppp}x (per colour or scale change): {} triangles, {} vertices",
            average(n, || vector.mesh(ppp, Color32::BLACK)),
            mesh.indices.len() / 3,
            mesh.vertices.len()
        );
    }
    println!(
        "svg size: {} bytes; page {:.1} x {:.1} pt",
        svg.text.len(),
        svg.size_pt[0],
        svg.size_pt[1]
    );
    Ok(())
}
