//! `cargo run --release --example render_png -- 'SNIPPET' out.png [--dark] [--scale 3]
//!  [--backend raster|svg|vector] [--wire] [--hard]`
//! Renders one Typst snippet to a PNG (handy for looking at output without a window). The vector
//! backend is drawn by a small CPU rasteriser the way a GPU would without multisampling; `--wire`
//! overlays its triangles, and `--hard` draws it without the feathered edge (aliased).

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;

use egui::Color32;
use egui_typst::debug::{rasterise_alpha, wire_edges};
use egui_typst::{Compiler, Style};

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Raster,
    Svg,
    Vector,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut positional = Vec::new();
    let (mut dark, mut wire, mut hard, mut scale, mut backend) =
        (false, false, false, 3.0_f32, Backend::Raster);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dark" => dark = true,
            "--wire" => wire = true,
            "--hard" => hard = true,
            "--scale" => scale = args.next().ok_or("--scale N")?.parse()?,
            "--backend" => {
                backend = match args.next().ok_or("--backend raster|svg|vector")?.as_str() {
                    "raster" => Backend::Raster,
                    "svg" => Backend::Svg,
                    "vector" => Backend::Vector,
                    other => return Err(format!("unknown backend `{other}`").into()),
                }
            }
            _ => positional.push(a),
        }
    }
    let [snippet, out] = <[String; 2]>::try_from(positional)
        .map_err(|_| "usage: render_png 'SNIPPET' out.png [--dark] [--scale N] [--backend raster|svg|vector] [--wire] [--hard]")?;
    let color = if dark {
        Color32::from_gray(235)
    } else {
        Color32::from_gray(20)
    };
    let style = Style {
        color,
        pixels_per_point: scale,
        margin_pt: 8.0,
        ..Style::default()
    };
    let compiler = Compiler::new()?;
    let background = if dark {
        [30_u8, 30, 34]
    } else {
        [250, 250, 250]
    };

    // coverage (0 to 1) per pixel, and the picture's size
    type Edges = Vec<[(f32, f32); 2]>;
    let (width, height, coverage, edges): (usize, usize, Vec<f32>, Edges) = match backend {
        Backend::Raster => {
            let image = compiler.render(&snippet, &style)?;
            let coverage = image
                .pixels
                .iter()
                .map(|p| f32::from(p.a()) / 255.0)
                .collect();
            (image.size[0], image.size[1], coverage, Vec::new())
        }
        Backend::Svg => {
            let svg = compiler.svg(&snippet, style.size_pt, style.margin_pt)?;
            let tree = resvg::usvg::Tree::from_str(
                &svg.tinted(Color32::BLACK),
                &resvg::usvg::Options::default(),
            )?;
            let (w, h) = (
                (svg.size_pt[0] * scale).round() as u32,
                (svg.size_pt[1] * scale).round() as u32,
            );
            let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("empty pixmap")?;
            let k = scale * 0.75; // usvg reads the svg's pt sizes at 96 dpi
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(k, k),
                &mut pixmap.as_mut(),
            );
            let coverage = pixmap
                .data()
                .chunks_exact(4)
                .map(|p| f32::from(p[3]) / 255.0)
                .collect();
            (w as usize, h as usize, coverage, Vec::new())
        }
        Backend::Vector => {
            let vector = compiler.vector(&snippet, style.size_pt, style.margin_pt)?;
            let mesh = if hard {
                vector.mesh_with_feather(0.0, Color32::BLACK)
            } else {
                vector.mesh(scale, Color32::BLACK)
            };
            let (w, h) = (
                (vector.size_pt[0] * scale).round() as usize,
                (vector.size_pt[1] * scale).round() as usize,
            );
            let coverage = rasterise_alpha(&mesh, scale, w, h)
                .iter()
                .map(|a| a / 255.0)
                .collect();
            let edges = if wire {
                wire_edges(&mesh, scale)
            } else {
                Vec::new()
            };
            println!(
                "{} triangles, {} contours, {} skipped",
                mesh.indices.len() / 3,
                vector.contour_count(),
                vector.skipped
            );
            (w, h, coverage, edges)
        }
    };

    let ink = [color.r(), color.g(), color.b()];
    let mut rgb = Vec::with_capacity(width * height * 3);
    for c in &coverage {
        for k in 0..3 {
            let v = f32::from(ink[k]) * c + f32::from(background[k]) * (1.0 - c);
            rgb.push(v.round().clamp(0.0, 255.0) as u8);
        }
    }
    // wireframe: thin red lines along every triangle edge
    for [(x0, y0), (x1, y1)] in edges {
        let steps = ((x1 - x0).abs().max((y1 - y0).abs()).ceil() as usize).max(1);
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let (x, y) = ((x0 + (x1 - x0) * t) as usize, (y0 + (y1 - y0) * t) as usize);
            if x < width && y < height {
                let i = (y * width + x) * 3;
                rgb[i..i + 3].copy_from_slice(&[220, 40, 40]);
            }
        }
    }
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(&out)?),
        width as u32,
        height as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;
    println!("{out}: {width}x{height} px");
    Ok(())
}
