//! `cargo run --release --example gallery`: a few formulas in a window, with a light/dark switch.

use egui_typst::{Backend, Typst};

const FORMULAS: &[(&str, &str)] = &[
    (
        "TVaR as an average of VaRs",
        "$ \"TVaR\"_alpha (L) = 1/(1-alpha) integral_alpha^1 \"VaR\"_u (L) dif u $",
    ),
    (
        "Euler allocation of the group's TVaR",
        "$ \"TVaR\"_alpha (L) = sum_k EE[L_k | L >= \"VaR\"_alpha (L)] $",
    ),
    (
        "Capital on the VaR basis",
        "$ \"Capital\" = \"VaR\"_(99.5%) (L) - EE[L] $",
    ),
    (
        "Loss ratio contribution",
        "$ \"LR\"_k = (EE[L_k]) / P quad \"and\" quad sum_k \"LR\"_k = \"LR\" $",
    ),
    (
        "A matrix",
        "$ Sigma = mat(1, rho_(12), rho_(13); rho_(12), 1, rho_(23); rho_(13), rho_(23), 1) $",
    ),
    (
        "Inline in a sentence",
        "A loss $L = -\"result\"$ with mean $mu$ and standard deviation $sigma$ has $\"CoV\" = sigma / mu$.",
    ),
    ("A mistake, shown instead of crashing", "$ unclosed"),
];

struct Gallery {
    dark: bool,
    backend: Backend,
    zoom: f32,
}

impl eframe::App for Gallery {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().set_zoom_factor(self.zoom);
        ui.ctx().set_visuals(if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("egui_typst");
                ui.checkbox(&mut self.dark, "dark");
                for (backend, label) in [
                    (Backend::Svg, "svg"),
                    (Backend::Vector, "vector"),
                    (Backend::Raster, "raster"),
                ] {
                    ui.selectable_value(&mut self.backend, backend, label);
                }
                ui.add(egui::Slider::new(&mut self.zoom, 0.5..=4.0).text("zoom"));
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (title, snippet) in FORMULAS {
                    ui.label(egui::RichText::new(*title).weak());
                    ui.add(Typst::new(snippet).size(18.0).backend(self.backend));
                    ui.add_space(10.0);
                }
            });
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "egui_typst gallery",
        eframe::NativeOptions::default(),
        Box::new(|_| {
            Ok(Box::new(Gallery {
                dark: false,
                backend: Backend::Svg,
                zoom: 1.0,
            }))
        }),
    )
}
