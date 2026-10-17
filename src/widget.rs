//! The egui side: `ui.add(Typst::new("$x^2$"))` draws a snippet as a texture sized in points,
//! coloured and scaled from the `Ui`, cached per snippet/colour/size/scale in the context.

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{
    Color32, Response, RichText, TextStyle, TextureHandle, TextureOptions, Ui, Vec2, Widget,
};

use crate::{Compiler, Style};

/// Most snippets kept per context; past this the cache starts over (a theme change or a zoom
/// re-renders everything on screen anyway).
const MAX_CACHED: usize = 256;

/// A Typst snippet as a widget. Wrap math in `$ … $` (display) or `$…$` (inline).
pub struct Typst<'a> {
    snippet: &'a str,
    size_pt: Option<f32>,
    color: Option<Color32>,
}

impl<'a> Typst<'a> {
    pub fn new(snippet: &'a str) -> Self {
        Self {
            snippet,
            size_pt: None,
            color: None,
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
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    snippet: String,
    color: [u8; 4],
    size: u32,
    scale: u32,
}

#[derive(Clone)]
enum Entry {
    Ready { texture: TextureHandle, size: Vec2 },
    Failed(String),
}

#[derive(Clone, Default)]
struct Cache(std::sync::Arc<std::sync::Mutex<HashMap<Key, Entry>>>);

fn compiler() -> &'static Result<Compiler, String> {
    static COMPILER: OnceLock<Result<Compiler, String>> = OnceLock::new();
    COMPILER.get_or_init(|| Compiler::new().map_err(|e| e.to_string()))
}

impl Widget for Typst<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let color = self.color.unwrap_or_else(|| ui.visuals().text_color());
        let size_pt = self
            .size_pt
            .unwrap_or_else(|| TextStyle::Body.resolve(ui.style()).size);
        // a zoomed UI has a larger pixels_per_point: render at that scale so it stays sharp
        let scale = ctx.pixels_per_point();
        let key = Key {
            snippet: self.snippet.to_owned(),
            color: color.to_array(),
            size: size_pt.to_bits(),
            scale: scale.to_bits(),
        };

        let cache = ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Cache>(egui::Id::unique("egui_typst::cache"))
                .clone()
        });
        let entry = {
            let mut map = cache
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if map.len() >= MAX_CACHED && !map.contains_key(&key) {
                map.clear();
            }
            map.entry(key.clone())
                .or_insert_with(|| match compiler() {
                    Err(why) => Entry::Failed(why.clone()),
                    Ok(compiler) => match compiler.render(
                        &key.snippet,
                        &Style {
                            color,
                            size_pt,
                            pixels_per_point: scale,
                            ..Style::default()
                        },
                    ) {
                        Ok(image) => {
                            let size =
                                Vec2::new(image.size[0] as f32, image.size[1] as f32) / scale;
                            let texture = ctx.load_texture(
                                format!("egui_typst:{}", key.snippet),
                                image,
                                TextureOptions::LINEAR,
                            );
                            Entry::Ready { texture, size }
                        }
                        Err(e) => Entry::Failed(e.to_string()),
                    },
                })
                .clone()
        };

        match entry {
            Entry::Ready { texture, size } => ui.image((texture.id(), size)),
            Entry::Failed(why) => ui.label(
                RichText::new(why)
                    .color(ui.visuals().error_fg_color)
                    .monospace(),
            ),
        }
    }
}
