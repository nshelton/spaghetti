//! Rendering state: visual settings, camera, FPS.

use crate::camera::Camera2D;
use crate::fps::FpsCounter;

/// Visual rendering state.
pub struct RenderState {
    /// Colors, opacity, circle mode, etc.
    pub render: crate::settings::RenderSettings,
    /// Camera pan/zoom.
    pub camera: Camera2D,
    /// Rolling frame rate counter.
    pub fps: FpsCounter,
}
