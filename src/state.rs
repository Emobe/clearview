use parking_lot::RwLock;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Interpolation {
    Bilinear,
    Lanczos, // placeholder – not yet implemented
}

impl std::fmt::Display for Interpolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Interpolation::Bilinear => write!(f, "Bilinear"),
            Interpolation::Lanczos => write!(f, "Lanczos"),
        }
    }
}

pub struct AppState {
    pub enabled: bool,
    /// Magnification factor (1.0–10.0).
    pub zoom: f32,
    /// Lerp speed per logical 60 Hz tick (0.01–1.0).
    pub smooth_speed: f32,
    pub interpolation: Interpolation,
    /// Smoothed mouse position in normalised screen coords (0..1).
    pub viewport_center: [f32; 2],
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            enabled: false,
            zoom: 2.0,
            smooth_speed: 0.15,
            interpolation: Interpolation::Bilinear,
            viewport_center: [0.5, 0.5],
        }
    }
}

pub type SharedState = Arc<RwLock<AppState>>;

pub fn new_shared() -> SharedState {
    Arc::new(RwLock::new(AppState::default()))
}
