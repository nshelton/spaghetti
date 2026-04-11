//! Right panel: details of the selected symbol + layout controls.

use std::collections::HashSet;

use core_ir::{EdgeKind, SymbolId};

use crate::app::SpaghettiApp;

/// The four edge kinds currently emitted by the clang frontend.
const ACTIVE_EDGE_KINDS: [EdgeKind; 4] = [
    EdgeKind::Calls,
    EdgeKind::Inherits,
    EdgeKind::Contains,
    EdgeKind::Overrides,
];

impl SpaghettiApp {
    /// Draw the right panel: details of selected symbol + layout controls.
    pub(crate) fn right_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("right_panel")
            .default_size(250.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // -- Details section --
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
                                ui.label(format!(
                                    "Location: {}:{}:{}",
                                    file_str, loc.line, loc.col
                                ));
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

                    ui.add_space(16.0);

                    // -- Layout controls section --
                    ui.heading("Layout Controls");
                    ui.separator();

                    let mut changed = false;
                    ui.spacing_mut().slider_width = 120.0;

                    let params = self.layout_state.params_mut();

                    changed |= ui
                        .add(
                            egui::Slider::new(&mut params.repulsion, 100.0..=1_000_000.0)
                                .logarithmic(true)
                                .text("Repulsion"),
                        )
                        .changed();

                    changed |= ui
                        .add(egui::Slider::new(&mut params.damping, 0.01..=0.99).text("Damping"))
                        .changed();

                    changed |= ui
                        .add(egui::Slider::new(&mut params.gravity, 0.0..=0.1).text("Gravity"))
                        .changed();

                    changed |= ui
                        .add(
                            egui::Slider::new(&mut params.max_velocity, 1.0..=200.0)
                                .text("Max vel"),
                        )
                        .changed();

                    ui.add_space(8.0);
                    ui.label("Per-Edge Kind");

                    for kind in &ACTIVE_EDGE_KINDS {
                        let label = format!("{kind:?}");
                        let id = ui.make_persistent_id(format!("edge_kind_{label}"));
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            id,
                            false,
                        )
                        .show_header(ui, |ui| {
                            ui.label(&label);
                        })
                        .body(|ui| {
                            let params = self.layout_state.params_mut();
                            if let Some(ep) = params.edge_params.get_mut(kind) {
                                changed |= ui
                                    .add(
                                        egui::Slider::new(&mut ep.target_distance, 10.0..=500.0)
                                            .text("Target dist"),
                                    )
                                    .changed();

                                changed |= ui
                                    .add(
                                        egui::Slider::new(&mut ep.attraction, 0.001..=0.5)
                                            .logarithmic(true)
                                            .text("Attraction"),
                                    )
                                    .changed();
                            }
                        });
                    }

                    ui.add_space(4.0);
                    if ui.button("Reset to defaults").clicked() {
                        *self.layout_state.params_mut() = layout::ForceParams::default();
                        changed = true;
                    }

                    if changed {
                        self.layout_state.reheat();
                    }
                });
            });
    }
}
