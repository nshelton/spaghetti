//! Left panel: search bar, edge filters, and symbol list.

use crate::app::SpaghettiApp;

impl SpaghettiApp {
    /// Draw the left panel: search, filters, symbol list.
    pub(crate) fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("left_panel")
            .default_size(220.0)
            .show_inside(ui, |ui| {
                ui.heading("spaghetti");
                ui.separator();

                // Search
                ui.label("Search:");
                ui.text_edit_singleline(&mut self.search);
                ui.separator();

                // Edge filters
                ui.label("Edge Filters:");
                ui.checkbox(&mut self.edge_filter.calls, "Calls");
                ui.checkbox(&mut self.edge_filter.inherits, "Inherits");
                ui.checkbox(&mut self.edge_filter.contains, "Contains");
                ui.checkbox(&mut self.edge_filter.overrides, "Overrides");
                ui.separator();

                // Symbol list
                ui.label("Symbols:");
                let search_lower = self.search.to_lowercase();
                let matches: Vec<_> = self
                    .graph
                    .symbols
                    .values()
                    .filter(|s| {
                        search_lower.is_empty()
                            || s.name.to_lowercase().contains(&search_lower)
                            || s.qualified_name.to_lowercase().contains(&search_lower)
                    })
                    .collect();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for sym in matches {
                        let label = format!("{:?} {}", sym.kind, sym.qualified_name);
                        let selected = self.selection == Some(sym.id);
                        if ui.selectable_label(selected, &label).clicked() {
                            self.selection = Some(sym.id);
                        }
                    }
                });
            });
    }
}
