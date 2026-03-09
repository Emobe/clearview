use parking_lot::RwLock;
use std::sync::{Arc, Mutex};

pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Raw BGRA8 pixel data, row-major, top-down.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ColorFilter {
    #[default]
    None,
    Inverted,
    Greyscale,
    GreyscaleInverted,
}

impl ColorFilter {
    pub fn as_u32(self) -> u32 {
        match self {
            Self::None              => 0,
            Self::Inverted          => 1,
            Self::Greyscale         => 2,
            Self::GreyscaleInverted => 3,
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayMode {
    Fullscreen,
    Docked(Edge),
}

impl Default for DisplayMode {
    fn default() -> Self {
        Self::Fullscreen
    }
}

pub struct AppState {
    pub enabled: bool,
    /// Magnification factor (1.0–10.0).
    pub zoom: f32,
    /// Lerp speed per logical 60 Hz tick (0.01–1.0).
    pub smooth_speed: f32,
    pub interpolation: Interpolation,
    pub display_mode: DisplayMode,
    /// Panel thickness in pixels (50–800). Ignored in Fullscreen mode.
    pub panel_size: u32,
    pub color_filter: ColorFilter,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            enabled: false,
            zoom: 2.0,
            smooth_speed: 0.15,
            interpolation: Interpolation::Bilinear,
            display_mode: DisplayMode::Fullscreen,
            panel_size: 300,
            color_filter: ColorFilter::None,
        }
    }
}

pub type SharedState = Arc<RwLock<AppState>>;
pub type FrameState = Arc<Mutex<Option<Arc<Frame>>>>;

pub fn new_shared() -> SharedState {
    Arc::new(RwLock::new(AppState::default()))
}
