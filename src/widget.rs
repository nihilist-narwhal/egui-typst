//! The egui side: `ui.add(Typst::new("$x^2$"))` draws a snippet sized in points, coloured from the
//! `Ui`, cached in the context.
//!
//! Two backends draw the same page:
//!
//! - [`Backend::Svg`] (the default): the snippet is compiled once, to SVG with a placeholder text
//!   colour; a theme change swaps the colour in the string (no recompile) and egui's SVG loader
//!   rasterises it at the size and scale on screen, so it stays sharp under zoom.
//! - [`Backend::Raster`]: Typst's own rasteriser at the current `pixels_per_point`; recompiled when
//!   the colour or the scale changes. Kept as the reference the SVG is tested against.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use egui::{
    Color32, Image, Mesh, Response, RichText, Sense, Shape, TextStyle, TextureHandle,
    TextureOptions, Ui, Vec2, Widget, load::Bytes,
};

use crate::{Compiler, MAX_PIXELS, MAX_SIDE_PX, Style, Svg, Vector};

/// Most snippets kept per context and kind; past this the cache starts over (a theme change or a
/// zoom re-renders everything on screen anyway).
const MAX_CACHED: usize = 256;

/// How a snippet is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Backend {
    /// Compile to SVG once; recolour by string swap; rasterise with egui's SVG loader.
    #[default]
    Svg,
    /// Typst's rasteriser, at the context's `pixels_per_point`.
    Raster,
    /// Glyph outlines and rules triangulated into egui meshes ([`crate::Vector`]): sharp at any
    /// zoom without a texture, recoloured per frame for free. A mesh is built per scale and
    /// colour and copied (translated) each frame, so very large formulas cost more than a texture.
    Vector,
}

/// A Typst snippet as a widget. Wrap math in `$ … $` (display) or `$…$` (inline).
pub struct Typst<'a> {
    snippet: &'a str,
    size_pt: Option<f32>,
    color: Option<Color32>,
    backend: Backend,
}

impl<'a> Typst<'a> {
    pub fn new(snippet: &'a str) -> Self {
        Self {
            snippet,
            size_pt: None,
            color: None,
            backend: Backend::default(),
        }
    }

    /// The text size in points (default: the `Ui`'s body text size).
    pub fn size(mut self, size_pt: f32) -> Self {
        self.size_pt = Some(size_pt);
        self
    }

    /// The text colour (default: the `Ui`'s text colour).
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// How to draw it (default: [`Backend::Svg`]).
    pub fn backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        self
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RasterKey {
    snippet: String,
    color: [u8; 4],
    size: u32,
    scale: u32,
}

/// What compiling to SVG depends on: not the colour, not the scale.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SvgKey {
    snippet: String,
    size: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TintKey {
    svg: SvgKey,
    rgb: [u8; 3],
}

#[derive(Clone)]
enum Raster {
    Ready { texture: TextureHandle, size: Vec2 },
    Failed(String),
}

#[derive(Clone)]
enum Compiled {
    Ready(Arc<Svg>),
    Failed(String),
}

/// An SVG in one colour, as the image loader will be handed it.
#[derive(Clone)]
struct Tinted {
    uri: String,
    bytes: Arc<[u8]>,
}

#[derive(Default)]
struct Inner {
    raster: HashMap<RasterKey, Raster>,
    compiled: HashMap<SvgKey, Compiled>,
    tinted: HashMap<TintKey, Tinted>,
    vectors: HashMap<SvgKey, Polygons>,
    meshes: HashMap<MeshKey, Arc<Mesh>>,
}

#[derive(Clone)]
enum Polygons {
    Ready(Arc<Vector>),
    Failed(String),
}

/// A mesh depends on the scale (the anti-aliasing ring is a device pixel wide) and the colour.
#[derive(Clone, PartialEq, Eq, Hash)]
struct MeshKey {
    vector: SvgKey,
    color: [u8; 4],
    scale: u32,
}

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<Inner>>);

fn compiler() -> &'static Result<Compiler, String> {
    static COMPILER: OnceLock<Result<Compiler, String>> = OnceLock::new();
    COMPILER.get_or_init(|| Compiler::new().map_err(|e| e.to_string()))
}

fn failed(ui: &mut Ui, why: String) -> Response {
    ui.label(
        RichText::new(why)
            .color(ui.visuals().error_fg_color)
            .monospace(),
    )
}

impl Widget for Typst<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let color = self.color.unwrap_or_else(|| ui.visuals().text_color());
        let size_pt = self
            .size_pt
            .unwrap_or_else(|| TextStyle::Body.resolve(ui.style()).size);
        let cache = ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Cache>(egui::Id::unique("egui_typst::cache"))
                .clone()
        });
        let mut inner = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
        match self.backend {
            Backend::Raster => draw_raster(ui, &mut inner, self.snippet, color, size_pt),
            Backend::Svg => draw_svg(ui, &mut inner, self.snippet, color, size_pt),
            Backend::Vector => draw_vector(ui, &mut inner, self.snippet, color, size_pt),
        }
    }
}

fn draw_raster(
    ui: &mut Ui,
    inner: &mut Inner,
    snippet: &str,
    color: Color32,
    size_pt: f32,
) -> Response {
    let ctx = ui.ctx().clone();
    // a zoomed UI has a larger pixels_per_point: render at that scale so it stays sharp
    let scale = ctx.pixels_per_point();
    let key = RasterKey {
        snippet: snippet.to_owned(),
        color: color.to_array(),
        size: size_pt.to_bits(),
        scale: scale.to_bits(),
    };
    if inner.raster.len() >= MAX_CACHED && !inner.raster.contains_key(&key) {
        inner.raster.clear();
    }
    let entry = inner
        .raster
        .entry(key.clone())
        .or_insert_with(|| match compiler() {
            Err(why) => Raster::Failed(why.clone()),
            Ok(compiler) => {
                let style = Style {
                    color,
                    size_pt,
                    pixels_per_point: scale,
                    ..Style::default()
                };
                match compiler.render(snippet, &style) {
                    Ok(image) => {
                        let size = Vec2::new(image.size[0] as f32, image.size[1] as f32) / scale;
                        let texture = ctx.load_texture(
                            format!("egui_typst:{snippet}"),
                            image,
                            TextureOptions::LINEAR,
                        );
                        Raster::Ready { texture, size }
                    }
                    Err(e) => Raster::Failed(e.to_string()),
                }
            }
        })
        .clone();
    match entry {
        Raster::Ready { texture, size } => ui.image((texture.id(), size)),
        Raster::Failed(why) => failed(ui, why),
    }
}

fn draw_svg(
    ui: &mut Ui,
    inner: &mut Inner,
    snippet: &str,
    color: Color32,
    size_pt: f32,
) -> Response {
    let ctx = ui.ctx().clone();
    egui_extras::install_image_loaders(&ctx);
    let svg_key = SvgKey {
        snippet: snippet.to_owned(),
        size: size_pt.to_bits(),
    };
    if inner.compiled.len() >= MAX_CACHED && !inner.compiled.contains_key(&svg_key) {
        inner.compiled.clear();
    }
    let compiled = inner
        .compiled
        .entry(svg_key.clone())
        .or_insert_with(|| match compiler() {
            Err(why) => Compiled::Failed(why.clone()),
            Ok(compiler) => match compiler.svg(snippet, size_pt, Style::default().margin_pt) {
                Ok(svg) => Compiled::Ready(Arc::new(svg)),
                Err(e) => Compiled::Failed(e.to_string()),
            },
        })
        .clone();
    let svg = match compiled {
        Compiled::Ready(svg) => svg,
        Compiled::Failed(why) => return failed(ui, why),
    };

    // the loader rasterises at the size on screen: refuse a page that would be an enormous texture
    let scale = ctx.pixels_per_point();
    let (px_w, px_h) = (svg.size_pt[0] * scale, svg.size_pt[1] * scale);
    if px_w > MAX_SIDE_PX as f32
        || px_h > MAX_SIDE_PX as f32
        || f64::from(px_w) * f64::from(px_h) > MAX_PIXELS as f64
    {
        return failed(
            ui,
            format!("the snippet would be {px_w:.0}x{px_h:.0} pixels at this scale"),
        );
    }

    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let tint_key = TintKey {
        svg: svg_key,
        rgb: [r, g, b],
    };
    if inner.tinted.len() >= MAX_CACHED && !inner.tinted.contains_key(&tint_key) {
        for old in inner.tinted.values() {
            ctx.forget_image(&old.uri);
        }
        inner.tinted.clear();
    }
    let tinted = inner
        .tinted
        .entry(tint_key.clone())
        .or_insert_with(|| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            tint_key.hash(&mut hasher);
            Tinted {
                uri: format!("bytes://egui_typst/{:016x}.svg", hasher.finish()),
                bytes: Arc::from(svg.tinted(color).into_bytes()),
            }
        })
        .clone();

    // the colour's alpha is applied as an image tint (the SVG carries the opaque colour)
    let tint = if a == 255 {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(a)
    };
    let size = Vec2::new(svg.size_pt[0], svg.size_pt[1]);
    ui.add(
        Image::from_bytes(tinted.uri, Bytes::Shared(tinted.bytes))
            .fit_to_exact_size(size)
            .tint(tint),
    )
}

fn draw_vector(
    ui: &mut Ui,
    inner: &mut Inner,
    snippet: &str,
    color: Color32,
    size_pt: f32,
) -> Response {
    let scale = ui.ctx().pixels_per_point();
    let key = SvgKey {
        snippet: snippet.to_owned(),
        size: size_pt.to_bits(),
    };
    if inner.vectors.len() >= MAX_CACHED && !inner.vectors.contains_key(&key) {
        inner.vectors.clear();
        inner.meshes.clear();
    }
    let polygons = inner
        .vectors
        .entry(key.clone())
        .or_insert_with(|| match compiler() {
            Err(why) => Polygons::Failed(why.clone()),
            Ok(compiler) => match compiler.vector(snippet, size_pt, Style::default().margin_pt) {
                Ok(vector) => Polygons::Ready(Arc::new(vector)),
                Err(e) => Polygons::Failed(e.to_string()),
            },
        })
        .clone();
    let vector = match polygons {
        Polygons::Ready(vector) => vector,
        Polygons::Failed(why) => return failed(ui, why),
    };

    let mesh_key = MeshKey {
        vector: key,
        color: color.to_array(),
        scale: scale.to_bits(),
    };
    if inner.meshes.len() >= MAX_CACHED && !inner.meshes.contains_key(&mesh_key) {
        inner.meshes.clear();
    }
    let mesh = inner
        .meshes
        .entry(mesh_key)
        .or_insert_with(|| Arc::new(vector.mesh(scale, color)))
        .clone();

    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(vector.size_pt[0], vector.size_pt[1]),
        Sense::hover(),
    );
    if ui.is_rect_visible(rect) {
        let mut placed = (*mesh).clone();
        placed.translate(rect.min.to_vec2());
        ui.painter().add(Shape::mesh(placed));
    }
    response
}
