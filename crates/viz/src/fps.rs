//! Lightweight FPS counter overlay.

use std::collections::VecDeque;
use std::time::Instant;

/// Tracks frame timestamps and computes a rolling average FPS.
pub(crate) struct FpsCounter {
    /// Recent frame timestamps (ring buffer).
    timestamps: VecDeque<Instant>,
    /// Window size for the rolling average.
    window: usize,
}

impl FpsCounter {
    /// Create a new counter averaging over the last `window` frames.
    pub fn new(window: usize) -> Self {
        Self {
            timestamps: VecDeque::with_capacity(window + 1),
            window,
        }
    }

    /// Record a frame tick. Call once per frame.
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.timestamps.push_back(now);
        while self.timestamps.len() > self.window {
            self.timestamps.pop_front();
        }
    }

    /// Current FPS estimate, or `None` if fewer than 2 frames recorded.
    pub fn fps(&self) -> Option<f32> {
        if self.timestamps.len() < 2 {
            return None;
        }
        let elapsed = self
            .timestamps
            .back()?
            .duration_since(*self.timestamps.front()?);
        let secs = elapsed.as_secs_f32();
        if secs < f32::EPSILON {
            return None;
        }
        Some((self.timestamps.len() - 1) as f32 / secs)
    }
}

/// Paint the FPS overlay in the top-right corner of the given rect.
pub(crate) fn paint_fps_overlay(ui: &egui::Ui, rect: egui::Rect, fps: Option<f32>) {
    let text = match fps {
        Some(fps) => format!("{fps:.0} FPS"),
        None => "-- FPS".to_string(),
    };
    let font = egui::FontId::monospace(12.0);
    let pos = rect.right_top() + egui::vec2(-8.0, 8.0);
    ui.painter().text(
        pos,
        egui::Align2::RIGHT_TOP,
        text,
        font,
        egui::Color32::from_white_alpha(180),
    );
}
