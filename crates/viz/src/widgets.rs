//! Reusable UI widgets for the spaghetti visualizer.

use egui::{Color32, Response, Ui};

/// A toggle button that shows green when active and blends into the background when inactive.
/// Drop-in replacement for `ui.checkbox()`.
pub fn toggle_button(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let active_bg = Color32::from_rgb(60, 160, 60);
    let active_text = Color32::WHITE;

    let response = if *value {
        let button =
            egui::Button::new(egui::RichText::new(label).color(active_text)).fill(active_bg);
        ui.add(button)
    } else {
        ui.add(egui::Button::new(label))
    };

    if response.clicked() {
        *value = !*value;
    }

    response
}
