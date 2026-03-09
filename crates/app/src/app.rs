use cv_core::{ColorFilter, DisplayMode, Edge, Interpolation, SharedState};
use eframe::egui;

pub struct ClearViewApp {
    state: SharedState,
}

impl ClearViewApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, state: SharedState) -> Self {
        Self { state }
    }
}

impl eframe::App for ClearViewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("clear-view");
            ui.separator();

            let mut s = self.state.write();

            // Enable / disable toggle
            ui.horizontal(|ui| {
                ui.label("Magnifier");
                let label = if s.enabled { "On" } else { "Off" };
                if ui.button(label).clicked() {
                    s.enabled = !s.enabled;
                }
                ui.label("  (Win += to toggle)");
            });

            ui.add_space(8.0);

            // Zoom slider
            ui.add(
                egui::Slider::new(&mut s.zoom, 1.0..=10.0)
                    .step_by(0.1)
                    .text("Zoom"),
            );

            ui.add_space(4.0);

            // Smooth follow speed
            ui.add(
                egui::Slider::new(&mut s.smooth_speed, 0.01..=1.0)
                    .step_by(0.01)
                    .text("Follow speed"),
            );

            ui.add_space(8.0);

            // Display mode
            ui.label("Display mode");
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut s.display_mode, DisplayMode::Fullscreen,        "Fullscreen");
                ui.radio_value(&mut s.display_mode, DisplayMode::Docked(Edge::Top),    "Top");
                ui.radio_value(&mut s.display_mode, DisplayMode::Docked(Edge::Bottom), "Bottom");
                ui.radio_value(&mut s.display_mode, DisplayMode::Docked(Edge::Left),   "Left");
                ui.radio_value(&mut s.display_mode, DisplayMode::Docked(Edge::Right),  "Right");
            });

            // Panel size — only shown when docked
            if s.display_mode != DisplayMode::Fullscreen {
                ui.add_space(4.0);
                ui.add(
                    egui::Slider::new(&mut s.panel_size, 50..=800)
                        .text("Panel size (px)"),
                );
            }

            ui.add_space(8.0);

            // Colour filter
            ui.label("Colour filter");
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut s.color_filter, ColorFilter::None,              "None");
                ui.radio_value(&mut s.color_filter, ColorFilter::Inverted,          "Inverted");
                ui.radio_value(&mut s.color_filter, ColorFilter::Greyscale,         "Greyscale");
                ui.radio_value(&mut s.color_filter, ColorFilter::GreyscaleInverted, "Grey+Inv");
            });

            ui.add_space(8.0);

            // Interpolation selector
            ui.label("Interpolation");
            ui.horizontal(|ui| {
                ui.radio_value(&mut s.interpolation, Interpolation::Bilinear, "Bilinear");
                ui.radio_value(&mut s.interpolation, Interpolation::Bicubic,  "Bicubic");
            });
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}
