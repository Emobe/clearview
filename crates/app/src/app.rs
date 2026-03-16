use cv_core::{ColorFilter, DisplayMode, Edge, Interpolation, SharedState};
use eframe::egui;

pub struct ClearViewApp {
    state: SharedState,
}

impl ClearViewApp {
    pub fn new(cc: &eframe::CreationContext<'_>, state: SharedState) -> Self {
        // Keep the event loop ticking at ~10 Hz even when the settings window is unfocused.
        // Without this, eframe idles into ControlFlow::Wait — the write lock acquired in
        // update() is never released and state.read() in the TTS thread blocks indefinitely.
        // request_repaint_after() inside update() is unreliable for this because it captures
        // cumulative_pass_nr at call time; if the window was focused and many frames rendered
        // before the 100ms fires, the pass_nr check marks it stale and drops the repaint.
        // Calling request_repaint() from a background thread always uses the current pass_nr.
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            ctx.request_repaint();
        });

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
                    egui::Slider::new(&mut s.panel_size, 1..=100)
                        .text("Panel size (%)"),
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

            ui.add_space(8.0);
            ui.separator();
            ui.heading("Screen Reader");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("TTS");
                let label = if s.tts_enabled { "On" } else { "Off" };
                if ui.button(label).clicked() {
                    s.tts_enabled = !s.tts_enabled;
                }
            });

            ui.add_space(4.0);

            ui.add_enabled(
                s.tts_enabled,
                egui::Checkbox::new(&mut s.tts_hover_enabled, "Hover echo"),
            );

            ui.add_space(4.0);

            ui.add_enabled(
                s.tts_enabled,
                egui::Slider::new(&mut s.tts_volume, 0..=100).text("Volume"),
            );
            ui.add_enabled(
                s.tts_enabled,
                egui::Slider::new(&mut s.tts_rate, -10..=10).text("Rate"),
            );
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}
