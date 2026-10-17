//! Typst math and markup in egui (experimental).
//!
//! Milestone 1: compile a Typst snippet once and rasterise it into an [`egui::ColorImage`], ready to
//! upload as a texture. The snippet is typeset on an auto-sized, margin-light page whose text is
//! the colour you pass, so a formula follows the UI theme (re-render when the theme changes).
//!
//! ```no_run
//! let image = egui_typst::render(
//!     "$ \"TVaR\"_alpha (L) = 1/(1-alpha) integral_alpha^1 \"VaR\"_u (L) dif u $",
//!     &egui_typst::Style::default(),
//! );
//! assert!(image.is_ok());
//! ```
//!
//! One [`Compiler`] holds the fonts and the library, which are expensive to build; keep it for the
//! life of the program. Typst interns every file id for good (65,535 at most), so the compiler
//! reuses a single id and swaps the source text in and out rather than minting one per snippet.

mod widget;

pub use widget::Typst;

use std::sync::Mutex;

use egui::{Color32, ColorImage};
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::{LazyHash, Scalar};
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;
use typst_render::RenderOptions;

/// How a snippet is typeset and drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// The text colour (formulas included).
    pub color: Color32,
    /// The text size in points; one point is one egui point at `pixels_per_point` 1.0.
    pub size_pt: f32,
    /// The page margin around the snippet, in points.
    pub margin_pt: f32,
    /// Raster pixels per point: pass `ctx.pixels_per_point()` (times any zoom) for crisp output.
    pub pixels_per_point: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: Color32::BLACK,
            size_pt: 14.0,
            margin_pt: 2.0,
            pixels_per_point: 2.0,
        }
    }
}

/// Why a snippet did not render.
#[derive(Debug)]
pub enum Error {
    /// The snippet has errors; each is a Typst message.
    Compile(Vec<String>),
    /// The snippet typeset to no page.
    Empty,
    /// The style asks for something unreasonable (a size or scale outside the supported range).
    Style(&'static str),
    /// The page would be too large to rasterise.
    TooLarge { width: u32, height: u32 },
    /// The compiler could not be built (its main file path).
    Setup(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Compile(messages) => write!(f, "typst: {}", messages.join("; ")),
            Error::Empty => write!(f, "typst: the snippet produced no page"),
            Error::Style(why) => write!(f, "style: {why}"),
            Error::TooLarge { width, height } => {
                write!(f, "the snippet would be {width}x{height} pixels")
            }
            Error::Setup(why) => write!(f, "setup: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Most pixels along one side of a rendered snippet.
pub const MAX_SIDE_PX: u32 = 8_192;

/// The fonts, library and a reusable main file: what compiling a snippet needs.
pub struct Compiler {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: FileId,
    source: Mutex<Source>,
}

impl Compiler {
    /// A compiler with the fonts bundled by `typst-assets` (a text face and a math face).
    pub fn new() -> Result<Self, Error> {
        let mut book = FontBook::new();
        let mut fonts = Vec::new();
        for data in typst_assets::fonts() {
            for font in Font::iter(Bytes::new(data)) {
                book.push(font.info().clone());
                fonts.push(font);
            }
        }
        let path = VirtualPath::new("main.typ").map_err(|e| Error::Setup(format!("{e:?}")))?;
        let main = RootedPath::new(VirtualRoot::Project, path).intern();
        Ok(Self {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(book),
            fonts,
            main,
            source: Mutex::new(Source::new(main, String::new())),
        })
    }

    /// Typesets `snippet` (Typst markup; wrap math in `$ … $`) and rasterises its page.
    pub fn render(&self, snippet: &str, style: &Style) -> Result<ColorImage, Error> {
        if !(1.0..=200.0).contains(&style.size_pt) || !(0.0..=100.0).contains(&style.margin_pt) {
            return Err(Error::Style(
                "size_pt must be 1 to 200 and margin_pt 0 to 100",
            ));
        }
        if !(0.25..=16.0).contains(&style.pixels_per_point) {
            return Err(Error::Style("pixels_per_point must be 0.25 to 16"));
        }
        let [r, g, b, a] = style.color.to_srgba_unmultiplied();
        let text = format!(
            "#set page(width: auto, height: auto, margin: {}pt, fill: none)\n#set text(size: {}pt, fill: rgb({r}, {g}, {b}, {a}))\n{snippet}",
            style.margin_pt, style.size_pt
        );
        let document = self.compile(&text)?;
        let page = document.pages().first().ok_or(Error::Empty)?;
        let options = RenderOptions {
            pixel_per_pt: Scalar::new(f64::from(style.pixels_per_point)),
            ..RenderOptions::default()
        };
        let pixmap = typst_render::render(page, &options);
        let (width, height) = (pixmap.width(), pixmap.height());
        if width > MAX_SIDE_PX || height > MAX_SIDE_PX {
            return Err(Error::TooLarge { width, height });
        }
        // tiny-skia's pixels are premultiplied RGBA, which is what egui textures hold
        Ok(ColorImage::from_rgba_premultiplied(
            [width as usize, height as usize],
            pixmap.data(),
        ))
    }

    fn compile(&self, text: &str) -> Result<PagedDocument, Error> {
        {
            let mut source = self
                .source
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            source.replace(text);
        }
        let output = typst::compile::<PagedDocument>(self).output;
        output.map_err(|errors| Error::Compile(errors.iter().map(message).collect()))
    }
}

fn message(diagnostic: &SourceDiagnostic) -> String {
    diagnostic.message.to_string()
}

impl World for Compiler {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self
                .source
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone())
        } else {
            Err(FileError::NotFound(std::path::PathBuf::new()))
        }
    }

    fn file(&self, _id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(std::path::PathBuf::new()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Renders with a process-wide [`Compiler`] (built on first use). Fine for a demo; own a
/// [`Compiler`] when you care about when the fonts load.
pub fn render(snippet: &str, style: &Style) -> Result<ColorImage, Error> {
    static COMPILER: std::sync::OnceLock<Result<Compiler, String>> = std::sync::OnceLock::new();
    match COMPILER.get_or_init(|| Compiler::new().map_err(|e| e.to_string())) {
        Ok(compiler) => compiler.render(snippet, style),
        Err(why) => Err(Error::Setup(why.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Result<T> = std::result::Result<T, Error>;

    #[test]
    fn a_formula_renders_to_visible_pixels() -> Result<()> {
        let image = render("$ sum_(i=1)^n i = (n(n+1))/2 $", &Style::default())?;
        assert!(image.size[0] > 20 && image.size[1] > 10, "{:?}", image.size);
        assert!(image.pixels.iter().any(|p| p.a() > 0), "something is drawn");
        Ok(())
    }

    #[test]
    fn errors_are_reported_not_panicked() {
        assert!(matches!(
            render("$ unclosed", &Style::default()),
            Err(Error::Compile(_))
        ));
    }

    #[test]
    fn hostile_styles_are_refused() {
        let style = Style {
            pixels_per_point: 1e9,
            ..Style::default()
        };
        assert!(matches!(render("$x$", &style), Err(Error::Style(_))));
        let style = Style {
            size_pt: f32::NAN,
            ..Style::default()
        };
        assert!(matches!(render("$x$", &style), Err(Error::Style(_))));
    }

    #[test]
    fn many_snippets_reuse_one_file_id() -> Result<()> {
        let compiler = Compiler::new()?;
        for i in 0..200 {
            compiler.render(&format!("$ x_{i} $"), &Style::default())?;
        }
        Ok(())
    }
}
