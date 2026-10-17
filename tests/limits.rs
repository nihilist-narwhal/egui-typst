//! Hostile page sizes are errors, never an allocation that aborts the process. (Before the check
//! moved ahead of the rasteriser, `#box(width: 1e6pt, height: 1e6pt)` asked for 16 TB.)

use std::error::Error;

use egui_typst::{Compiler, Error as TypstError, Style};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const HUGE: &[&str] = &[
    "#box(width: 1e6pt, height: 1e6pt)",
    "$ x + y $ #h(1e9pt)",
    "#box(width: 1e300pt, height: 1pt)",
];

#[test]
fn a_huge_page_is_too_large_on_every_backend() -> Result<()> {
    let compiler = Compiler::new()?;
    let style = Style::default();
    for snippet in HUGE {
        assert!(
            matches!(
                compiler.render(snippet, &style),
                Err(TypstError::TooLarge { .. })
            ),
            "raster: {snippet}"
        );
        assert!(
            matches!(
                compiler.svg(snippet, 14.0, 2.0),
                Err(TypstError::TooLarge { .. })
            ),
            "svg: {snippet}"
        );
        assert!(
            matches!(
                compiler.vector(snippet, 14.0, 2.0),
                Err(TypstError::TooLarge { .. })
            ),
            "vector: {snippet}"
        );
    }
    Ok(())
}

#[test]
fn a_5000_pt_page_is_a_raster_error_but_a_legal_svg() -> Result<()> {
    let compiler = Compiler::new()?;
    let snippet = "#box(width: 5000pt, height: 5000pt)";
    // 10,000 px a side at the default 2 px/pt: over the raster's limit; as SVG the limit is in
    // points, so it compiles, and the *widget* decides from the scale on screen (next test)
    assert!(matches!(
        compiler.render(snippet, &Style::default()),
        Err(TypstError::TooLarge { .. })
    ));
    assert!(compiler.svg(snippet, 14.0, 2.0).is_ok());
    Ok(())
}

#[test]
fn the_widget_refuses_a_page_that_would_be_an_enormous_texture() {
    use egui::{Context, ImageData, Pos2, RawInput, Rect, TextureId, vec2};
    let ctx = Context::default();
    ctx.set_pixels_per_point(2.0);
    let mut biggest_upload = 0;
    for _ in 0..4 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 400.0))),
            ..RawInput::default()
        };
        let output = ctx.run_ui(input, |ui| {
            ui.add(egui_typst::Typst::new(
                "#box(width: 5000pt, height: 5000pt)",
            ));
        });
        for (id, deltas) in &output.textures_delta.set {
            if *id == TextureId::Managed(0) {
                continue;
            }
            for delta in deltas {
                let ImageData::Color(image) = &delta.image;
                biggest_upload = biggest_upload.max(image.size[0] * image.size[1]);
            }
        }
    }
    assert_eq!(
        biggest_upload, 0,
        "no texture may be uploaded for it (saw {biggest_upload} pixels)"
    );
}

#[test]
fn the_pixel_budget_depends_on_the_scale() -> Result<()> {
    let compiler = Compiler::new()?;
    // 1000 x 1000 pt is fine at 1 px/pt (1 MP) and too large at 16 px/pt (256 MP)
    let snippet = "#box(width: 1000pt, height: 1000pt)";
    assert!(
        compiler
            .render(
                snippet,
                &Style {
                    pixels_per_point: 1.0,
                    ..Style::default()
                }
            )
            .is_ok()
    );
    assert!(matches!(
        compiler.render(
            snippet,
            &Style {
                pixels_per_point: 16.0,
                ..Style::default()
            }
        ),
        Err(TypstError::TooLarge { .. })
    ));
    Ok(())
}

#[test]
fn typst_refuses_runaway_code_quickly() -> Result<()> {
    let compiler = Compiler::new()?;
    let style = Style::default();
    assert!(compiler.render("#while true { }", &style).is_err());
    assert!(
        compiler
            .render("#let f(n) = f(n + 1)\n#f(0)", &style)
            .is_err()
    );
    Ok(())
}

#[test]
fn snippets_cannot_read_files_or_import_packages() -> Result<()> {
    let compiler = Compiler::new()?;
    let style = Style::default();
    assert!(compiler.render("#read(\"/etc/passwd\")", &style).is_err());
    assert!(
        compiler
            .render("#import \"@preview/cetz:0.3.0\"", &style)
            .is_err()
    );
    assert!(
        compiler
            .render("#image(\"/etc/hostname\")", &style)
            .is_err()
    );
    Ok(())
}
