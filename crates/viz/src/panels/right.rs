//! Right panel: details of the selected symbol.

use std::collections::HashSet;

use core_ir::SymbolId;

use crate::app::SpaghettiApp;

impl SpaghettiApp {
    /// Draw the right panel: details of selected symbol.
    pub(crate) fn right_panel(&self, ui: &mut egui::Ui) {
        egui::Panel::right("right_panel")
            .default_size(250.0)
            .show_inside(ui, |ui| {
                ui.heading("Details");
                ui.separator();

                if let Some(sel_id) = self.selection {
                    if let Some(sym) = self.graph.symbols.get(&sel_id) {
                        ui.label(format!("Name: {}", sym.name));
                        ui.label(format!("Qualified: {}", sym.qualified_name));
                        ui.label(format!("Kind: {:?}", sym.kind));

                        if let Some(loc) = &sym.location {
                            let file_str =
                                self.graph.files.resolve(loc.file).unwrap_or("<unknown>");
                            ui.label(format!("Location: {}:{}:{}", file_str, loc.line, loc.col));
                        }

                        if !sym.attrs.is_empty() {
                            ui.label(format!("Attrs: {:?}", sym.attrs));
                        }

                        ui.separator();
                        ui.label("Neighbors:");
                        let active = self.edge_filter.active_kinds();
                        let neighbors: Vec<_> = self.graph.neighbors(sel_id, &active).collect();
                        let seen: HashSet<SymbolId> = neighbors.iter().copied().collect();
                        for nid in seen {
                            if let Some(nsym) = self.graph.symbols.get(&nid) {
                                ui.label(format!("  {:?} {}", nsym.kind, nsym.qualified_name));
                            }
                        }
                    }
                } else {
                    ui.label("Click a node to see details.");
                }
            });
    }
}
