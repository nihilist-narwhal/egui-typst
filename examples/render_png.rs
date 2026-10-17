//! `cargo run --release --example render_png -- 'SNIPPET' out.png [--dark] [--scale 3]`
//! Renders one Typst snippet to a PNG (handy for looking at output without a window).

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;

use egui::Color32;
use egui_typst::{Compiler, Style};

fn main() -> Result<(), Box<dyn Error>> {
    let mut positional = Vec::new();
    let (mut dark, mut scale) = (false, 3.0_f32);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dark" => dark = true,
            "--scale" => scale = args.next().ok_or("--scale N")?.parse()?,
            _ => positional.push(a),
        }
    }
    let [snippet, out] = <[String; 2]>::try_from(positional)
        .map_err(|_| "usage: render_png 'SNIPPET' out.png [--dark] [--scale N]")?;
    let style = Style {
        color: if dark {
            Color32::from_gray(235)
        } else {
            Color32::from_gray(20)
        },
        pixels_per_point: scale,
        margin_pt: 8.0,
        ..Style::default()
    };
    let image = Compiler::new()?.render(&snippet, &style)?;

    // composite over the theme's background so the PNG reads like it would on screen
    let background = if dark {
        [30_u8, 30, 34]
    } else {
        [250, 250, 250]
    };
    let mut rgb = Vec::with_capacity(image.pixels.len() * 3);
    for p in &image.pixels {
        let a = u16::from(p.a());
        for (channel, back) in [
            (p.r(), background[0]),
            (p.g(), background[1]),
            (p.b(), background[2]),
        ] {
            // premultiplied over: out = src + back * (1 - a)
            rgb.push((u16::from(channel) + u16::from(back) * (255 - a) / 255).min(255) as u8);
        }
    }
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(&out)?),
        image.size[0] as u32,
        image.size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;
    println!("{out}: {}x{} px", image.size[0], image.size[1]);
    Ok(())
}
