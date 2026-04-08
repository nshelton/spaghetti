//! Log console panel: displays captured tracing output.

use tracing::Level;

use crate::app::SpaghettiApp;

impl SpaghettiApp {
    /// Draw the log console panel (toggled via View > Console).
    pub(crate) fn console_panel(&mut self, ui: &mut egui::Ui) {
        if !self.show_console {
            return;
        }

        egui::Panel::bottom("console_panel")
            .default_size(180.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Console");
                    ui.separator();

                    // Level filter
                    egui::ComboBox::from_id_salt("log_level_filter")
                        .selected_text(format!("{}", self.console_level_filter))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.console_level_filter,
                                Level::ERROR,
                                "ERROR",
                            );
                            ui.selectable_value(
                                &mut self.console_level_filter,
                                Level::WARN,
                                "WARN",
                            );
                            ui.selectable_value(
                                &mut self.console_level_filter,
                                Level::INFO,
                                "INFO",
                            );
                            ui.selectable_value(
                                &mut self.console_level_filter,
                                Level::DEBUG,
                                "DEBUG",
                            );
                        });

                    if ui.button("Clear").clicked() {
                        if let Ok(mut buf) = self.log_buffer.lock() {
                            buf.clear();
                        }
                    }
                });

                ui.separator();

                let scroll_area = egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false; 2]);

                scroll_area.show(ui, |ui| {
                    if let Ok(buf) = self.log_buffer.lock() {
                        for entry in buf.entries() {
                            if entry.level > self.console_level_filter {
                                continue;
                            }
                            let color = level_color(entry.level);
                            ui.horizontal(|ui| {
                                ui.colored_label(egui::Color32::from_gray(120), &entry.timestamp);
                                ui.colored_label(color, format!("{:5}", entry.level));
                                ui.label(&entry.message);
                            });
                        }
                    }
                });
            });
    }
}

/// Map a tracing level to a display color.
fn level_color(level: Level) -> egui::Color32 {
    match level {
        Level::ERROR => egui::Color32::from_rgb(220, 80, 80),
        Level::WARN => egui::Color32::from_rgb(220, 180, 60),
        Level::INFO => egui::Color32::from_rgb(140, 200, 140),
        Level::DEBUG => egui::Color32::from_rgb(140, 140, 200),
        Level::TRACE => egui::Color32::from_gray(160),
    }
}
