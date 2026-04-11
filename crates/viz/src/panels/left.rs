//! Left panel: search bar, edge filters, and symbol list.

use crate::app::SpaghettiApp;
use crate::widgets::toggle_button;

impl SpaghettiApp {
    /// Draw the left panel: search, filters, symbol list.
    pub(crate) fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("left_panel")
            .default_size(220.0)
            .show_inside(ui, |ui| {
                ui.heading("spaghetti");
                ui.label(format!(
                    "{} nodes, {} edges",
                    self.graph.symbol_count(),
                    self.graph.edge_count()
                ));
                ui.separator();

                // Search
                ui.label("Search:");
                ui.text_edit_singleline(&mut self.search);
                ui.separator();

                // Edge filters
                ui.label("Edge Filters:");
                toggle_button(ui, &mut self.edge_filter.calls, "Calls");
                toggle_button(ui, &mut self.edge_filter.inherits, "Inherits");
                toggle_button(ui, &mut self.edge_filter.contains, "Contains");
                toggle_button(ui, &mut self.edge_filter.overrides, "Overrides");
                ui.separator();

                // Visibility filter
                toggle_button(ui, &mut self.hide_externals, "Hide external/stdlib");
                ui.separator();

                // Symbol list
                ui.label("Symbols:");
                let search_lower = self.search.to_lowercase();
                let hide_ext = self.hide_externals;
                let files = &self.graph.files;
                let matches: Vec<_> = self
                    .graph
                    .symbols
                    .values()
                    .filter(|s| {
                        // Filter external symbols when checkbox is on.
                        if hide_ext {
                            let is_ext = match &s.location {
                                Some(loc) => {
                                    let path = files.resolve(loc.file).unwrap_or("");
                                    path.starts_with('/')
                                }
                                None => true,
                            };
                            if is_ext {
                                return false;
                            }
                        }
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
