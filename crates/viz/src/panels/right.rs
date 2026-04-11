//! Right panel: details of the selected symbol + layout controls.

use crate::app::SpaghettiApp;
use crate::state::filter_state::{ALL_EDGE_KINDS, ALL_SYMBOL_KINDS};

impl SpaghettiApp {
    /// Draw the right panel: details of selected symbol + layout controls.
    pub(crate) fn right_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("right_panel")
            .default_size(300.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // -- Details section --
                    ui.heading("Details");
                    ui.separator();

                    if let Some(sel_id) = self.interaction.selection {
                        if let Some(sym) = self.graph.graph.symbols.get(&sel_id) {
                            ui.label(format!("Name: {}", sym.name));
                            ui.label(format!("Qualified: {}", sym.qualified_name));
                            ui.label(format!("Kind: {:?}", sym.kind));

                            if let Some(loc) = &sym.location {
                                let file_str = self
                                    .graph
                                    .graph
                                    .files
                                    .resolve(loc.file)
                                    .unwrap_or("<unknown>");
                                ui.label(format!(
                                    "Location: {}:{}:{}",
                                    file_str, loc.line, loc.col
                                ));
                            } else {
                                ui.label("Location: <external>");
                            }

                            if !sym.attrs.is_empty() {
                                ui.label(format!("Attrs: {:?}", sym.attrs));
                            }

                            // Collapse/Expand button for container nodes.
                            if self.simulation.layout_state.is_container(sel_id) {
                                ui.separator();
                                let children = self.simulation.layout_state.children_of(sel_id);
                                let n = children.len();
                                if self.simulation.layout_state.is_expanded(sel_id) {
                                    ui.label(format!("Children: {n} (expanded)"));
                                    if ui.button("Collapse").clicked() {
                                        self.simulation.layout_state.collapse(sel_id);
                                        self.filters
                                            .sync_hidden_symbols(&self.graph, &mut self.simulation);
                                    }
                                } else {
                                    ui.label(format!("Children: {n} (collapsed)"));
                                    if ui.button("Expand").clicked() {
                                        self.simulation.layout_state.expand(sel_id);
                                        self.filters
                                            .sync_hidden_symbols(&self.graph, &mut self.simulation);
                                    }
                                }
                            }

                            // -- Edge summary by type --
                            ui.separator();
                            ui.heading("Connections");

                            let active = self.filters.edge_filter.active_kinds();

                            for &kind in &ALL_EDGE_KINDS {
                                if !active.contains(&kind) {
                                    continue;
                                }

                                let mut outgoing: Vec<(&str, bool)> = Vec::new();
                                let mut incoming: Vec<(&str, bool)> = Vec::new();

                                for edge in &self.graph.graph.edges {
                                    if edge.kind != kind {
                                        continue;
                                    }
                                    if edge.from == sel_id {
                                        if let Some(target) = self.graph.graph.symbols.get(&edge.to)
                                        {
                                            let is_external = self.graph.graph.is_external(edge.to);
                                            outgoing.push((&target.qualified_name, is_external));
                                        }
                                    } else if edge.to == sel_id {
                                        if let Some(source) =
                                            self.graph.graph.symbols.get(&edge.from)
                                        {
                                            let is_external =
                                                self.graph.graph.is_external(edge.from);
                                            incoming.push((&source.qualified_name, is_external));
                                        }
                                    }
                                }

                                let total = outgoing.len() + incoming.len();
                                if total == 0 {
                                    continue;
                                }

                                let header = format!("{kind:?} ({total})");
                                let id = ui.make_persistent_id(format!("conn_{kind:?}"));
                                egui::collapsing_header::CollapsingState::load_with_default_open(
                                    ui.ctx(),
                                    id,
                                    false,
                                )
                                .show_header(ui, |ui| {
                                    ui.label(&header);
                                })
                                .body(|ui| {
                                    if !outgoing.is_empty() {
                                        ui.label(format!("  Outgoing ({})", outgoing.len()));
                                        for (name, is_ext) in &outgoing {
                                            let label = if *is_ext {
                                                format!("    \u{2192} {name} [ext]")
                                            } else {
                                                format!("    \u{2192} {name}")
                                            };
                                            ui.label(label);
                                        }
                                    }
                                    if !incoming.is_empty() {
                                        ui.label(format!("  Incoming ({})", incoming.len()));
                                        for (name, is_ext) in &incoming {
                                            let label = if *is_ext {
                                                format!("    \u{2190} {name} [ext]")
                                            } else {
                                                format!("    \u{2190} {name}")
                                            };
                                            ui.label(label);
                                        }
                                    }
                                });
                            }
                        }
                    } else {
                        ui.label("Click a node to see details.");
                    }

                    ui.add_space(16.0);

                    // -- Rendering controls section --
                    ui.heading("Rendering");
                    ui.separator();

                    crate::widgets::toggle_button(
                        ui,
                        &mut self.render.render.circle_mode,
                        "Circle mode",
                    );
                    if self.render.render.circle_mode {
                        ui.add(
                            egui::Slider::new(&mut self.render.render.circle_radius, 1.0..=50.0)
                                .text("Radius"),
                        );
                    }

                    ui.add(
                        egui::Slider::new(&mut self.render.render.node_opacity, 0.0..=1.0)
                            .text("Node opacity"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.render.render.edge_opacity, 0.0..=1.0)
                            .text("Edge opacity"),
                    );

                    ui.add_space(8.0);

                    // Node types: toggleable color swatches that also control filtering.
                    ui.label("Node Types");
                    let mut node_filter_changed = false;
                    for &kind in &ALL_SYMBOL_KINDS {
                        let kind_name = format!("{kind:?}");
                        let enabled = self.filters.node_filter.is_enabled(kind);
                        if toggle_swatch(
                            ui,
                            &mut self.render.render.node_colors,
                            &kind_name,
                            enabled,
                        ) {
                            self.filters.node_filter.toggle(kind);
                            node_filter_changed = true;
                        }
                    }
                    if node_filter_changed {
                        self.filters
                            .sync_hidden_to_layout(&self.graph, &mut self.simulation);
                    }
                    if ui
                        .checkbox(&mut self.filters.hide_edgeless, "Hide edgeless nodes")
                        .changed()
                    {
                        self.filters
                            .sync_hidden_to_layout(&self.graph, &mut self.simulation);
                    }

                    // Edge types: toggleable color swatches with per-kind force sliders.
                    ui.add_space(4.0);
                    ui.label("Edge Types");

                    let mut changed = false;
                    ui.spacing_mut().slider_width = 120.0;

                    let default_ep = layout::EdgeKindParams {
                        target_distance: 150.0,
                        attraction: 0.01,
                    };

                    let mut edge_filter_changed = false;
                    for &kind in &ALL_EDGE_KINDS {
                        let kind_name = format!("{kind:?}");
                        let enabled = self.filters.edge_filter.is_enabled(kind);
                        if toggle_swatch(
                            ui,
                            &mut self.render.render.edge_colors,
                            &kind_name,
                            enabled,
                        ) {
                            self.filters.edge_filter.toggle(kind);
                            edge_filter_changed = true;
                        }
                        if enabled {
                            let ep = self
                                .simulation
                                .layout_state
                                .params_mut()
                                .edge_params
                                .entry(kind)
                                .or_insert(default_ep);
                            ui.indent(format!("edge_sliders_{kind:?}"), |ui| {
                                changed |= ui
                                    .add(
                                        egui::Slider::new(&mut ep.target_distance, 1.0..=100.0)
                                            .text("Dist"),
                                    )
                                    .changed();
                                changed |= ui
                                    .add(
                                        egui::Slider::new(&mut ep.attraction, 0.0..=2.0)
                                            .text("Attract"),
                                    )
                                    .changed();
                            });
                        }
                    }

                    if edge_filter_changed && self.filters.hide_edgeless {
                        self.filters
                            .sync_hidden_to_layout(&self.graph, &mut self.simulation);
                    }

                    ui.add_space(4.0);
                    if ui.button("Reset colors").clicked() {
                        let defaults = crate::settings::RenderSettings::default();
                        self.render.render.node_colors = defaults.node_colors;
                        self.render.render.edge_colors = defaults.edge_colors;
                    }

                    ui.add_space(16.0);

                    // -- Layout controls section --
                    ui.heading("Layout Controls");
                    ui.separator();

                    let params = self.simulation.layout_state.params_mut();

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

                    let default_ep = layout::EdgeKindParams {
                        target_distance: 150.0,
                        attraction: 0.01,
                    };

                    for kind in &ALL_EDGE_KINDS {
                        let label = format!("{kind:?}");
                        ui.add_space(4.0);
                        ui.label(&label);

                        let params = self.simulation.layout_state.params_mut();
                        let ep = params.edge_params.entry(*kind).or_insert(default_ep);
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut ep.target_distance, 1.0..=100.0)
                                    .text("Target dist"),
                            )
                            .changed();

                        changed |= ui
                            .add(
                                egui::Slider::new(&mut ep.attraction, 0.0..=1.0).text("Attraction"),
                            )
                            .changed();
                    }

                    ui.add_space(8.0);
                    ui.label("Containment");

                    let params = self.simulation.layout_state.params_mut();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut params.containment_strength, 0.0..=0.2)
                                .text("Strength"),
                        )
                        .changed();

                    ui.add_space(8.0);
                    ui.label("Location Affinity");

                    let params = self.simulation.layout_state.params_mut();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut params.location_strength, 0.0..=2.0)
                                .text("Strength"),
                        )
                        .changed();

                    changed |= ui
                        .add(
                            egui::Slider::new(&mut params.location_falloff, 0.0..=1.0)
                                .text("Falloff"),
                        )
                        .changed();

                    ui.add_space(4.0);
                    if ui.button("Reset to defaults").clicked() {
                        *self.simulation.layout_state.params_mut() = layout::ForceParams::default();
                        changed = true;
                    }

                    if ui
                        .button("Randomize layout (R)")
                        .on_hover_text("Scatter all nodes to random positions")
                        .clicked()
                    {
                        self.simulation.layout_state.randomize();
                    }

                    if ui
                        .button("Juggle (J)")
                        .on_hover_text("Slightly nudge all nodes — press repeatedly for more")
                        .clicked()
                    {
                        self.simulation.layout_state.juggle();
                    }

                    let pause_label = if self.simulation.paused {
                        "Resume (P)"
                    } else {
                        "Pause (P)"
                    };
                    if ui
                        .button(pause_label)
                        .on_hover_text("Pause or resume the force-directed simulation")
                        .clicked()
                    {
                        self.simulation.paused = !self.simulation.paused;
                    }

                    if changed {
                        self.simulation.layout_state.reheat();
                    }
                });
            });
    }
}

/// Draw a toggle swatch: a wide colored rectangle with the label inside.
/// Left-click toggles the type on/off. Right-click opens the color picker.
/// Returns `true` if the toggle state changed.
fn toggle_swatch(
    ui: &mut egui::Ui,
    colors: &mut std::collections::HashMap<String, crate::settings::Rgb>,
    name: &str,
    enabled: bool,
) -> bool {
    let rgb = colors.entry(name.to_string()).or_insert([80, 80, 80]);
    let mut color = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);

    let available = ui.available_width();
    let size = egui::vec2(available, 20.0);
    let (rect, response) =
        ui.allocate_exact_size(size, egui::Sense::click().union(egui::Sense::click()));

    let bg = if enabled {
        color
    } else {
        egui::Color32::from_gray(40)
    };
    ui.painter().rect_filled(rect, 3.0, bg);

    let text_color = if enabled {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_gray(120)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(12.0),
        text_color,
    );

    response
        .clone()
        .on_hover_text("Click: toggle, Right-click: color");
    response.context_menu(|ui: &mut egui::Ui| {
        ui.set_min_width(200.0);
        egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque);
        *rgb = [color.r(), color.g(), color.b()];
    });

    response.clicked()
}
