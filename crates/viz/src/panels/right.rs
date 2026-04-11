//! Right panel: details of the selected symbol + layout controls.

use crate::app::{SpaghettiApp, ALL_EDGE_KINDS};

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
                            } else {
                                ui.label("Location: <external>");
                            }

                            if !sym.attrs.is_empty() {
                                ui.label(format!("Attrs: {:?}", sym.attrs));
                            }

                            // Collapse/Expand button for container nodes.
                            if self.layout_state.is_container(sel_id) {
                                ui.separator();
                                let children = self.layout_state.children_of(sel_id);
                                let n = children.len();
                                if self.layout_state.is_expanded(sel_id) {
                                    ui.label(format!("Children: {n} (expanded)"));
                                    if ui.button("Collapse").clicked() {
                                        self.layout_state.collapse(sel_id);
                                        self.sync_hidden_symbols();
                                    }
                                } else {
                                    ui.label(format!("Children: {n} (collapsed)"));
                                    if ui.button("Expand").clicked() {
                                        self.layout_state.expand(sel_id);
                                        self.sync_hidden_symbols();
                                    }
                                }
                            }

                            // -- Edge summary by type --
                            ui.separator();
                            ui.heading("Connections");

                            let active = self.edge_filter.active_kinds();

                            for &kind in &ALL_EDGE_KINDS {
                                if !active.contains(&kind) {
                                    continue;
                                }

                                // Collect outgoing and incoming edges of this kind.
                                let mut outgoing: Vec<(&str, bool)> = Vec::new();
                                let mut incoming: Vec<(&str, bool)> = Vec::new();

                                for edge in &self.graph.edges {
                                    if edge.kind != kind {
                                        continue;
                                    }
                                    if edge.from == sel_id {
                                        if let Some(target) = self.graph.symbols.get(&edge.to) {
                                            let is_external = self.graph.is_external(edge.to);
                                            outgoing.push((&target.qualified_name, is_external));
                                        }
                                    } else if edge.to == sel_id {
                                        if let Some(source) = self.graph.symbols.get(&edge.from) {
                                            let is_external = self.graph.is_external(edge.from);
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

                    crate::widgets::toggle_button(ui, &mut self.render.circle_mode, "Circle mode");
                    if self.render.circle_mode {
                        ui.add(
                            egui::Slider::new(&mut self.render.circle_radius, 1.0..=50.0)
                                .text("Radius"),
                        );
                    }

                    ui.add(
                        egui::Slider::new(&mut self.render.node_opacity, 0.0..=1.0)
                            .text("Node opacity"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.render.edge_opacity, 0.0..=1.0)
                            .text("Edge opacity"),
                    );

                    ui.add_space(8.0);

                    // Node colors
                    let nid = ui.make_persistent_id("render_node_colors");
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        nid,
                        false,
                    )
                    .show_header(ui, |ui| {
                        ui.label("Node Colors");
                    })
                    .body(|ui| {
                        for kind_name in &[
                            "Class",
                            "Struct",
                            "Function",
                            "Method",
                            "Field",
                            "Namespace",
                            "TemplateInstantiation",
                            "TranslationUnit",
                        ] {
                            color_swatch_row(ui, &mut self.render.node_colors, kind_name);
                        }
                    });

                    // Edge types: toggleable color swatches that also control filtering.
                    ui.add_space(4.0);
                    ui.label("Edge Types");
                    for &kind in &ALL_EDGE_KINDS {
                        let kind_name = format!("{kind:?}");
                        let enabled = self.edge_filter.is_enabled(kind);
                        if edge_toggle_swatch(ui, &mut self.render.edge_colors, &kind_name, enabled)
                        {
                            self.edge_filter.toggle(kind);
                        }
                    }

                    ui.add_space(4.0);
                    if ui.button("Reset colors").clicked() {
                        let defaults = crate::settings::RenderSettings::default();
                        self.render.node_colors = defaults.node_colors;
                        self.render.edge_colors = defaults.edge_colors;
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

                    for kind in &ALL_EDGE_KINDS {
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

                    ui.add_space(8.0);
                    ui.label("Containment");

                    let params = self.layout_state.params_mut();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut params.containment_strength, 0.0..=0.2)
                                .text("Strength"),
                        )
                        .changed();

                    ui.add_space(8.0);
                    ui.label("Location Affinity");

                    let params = self.layout_state.params_mut();
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
                        *self.layout_state.params_mut() = layout::ForceParams::default();
                        changed = true;
                    }

                    if ui
                        .button("Randomize layout (R)")
                        .on_hover_text("Scatter all nodes to random positions")
                        .clicked()
                    {
                        self.layout_state.randomize();
                    }

                    if changed {
                        self.layout_state.reheat();
                    }
                });
            });
    }
}

/// Draw a single color swatch row: a wide colored rectangle with the label
/// inside it, clicking opens a color picker.
fn color_swatch_row(
    ui: &mut egui::Ui,
    colors: &mut std::collections::HashMap<String, crate::settings::Rgb>,
    name: &str,
) {
    let rgb = colors.entry(name.to_string()).or_insert([80, 80, 80]);
    let mut color = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);

    // Draw a colored button that fills the available width.
    let available = ui.available_width();
    let size = egui::vec2(available, 20.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    // Paint the swatch background.
    ui.painter().rect_filled(rect, 3.0, color);

    // Paint the label text on top.
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(12.0),
        egui::Color32::WHITE,
    );

    // Open a color popup on click.
    egui::Popup::from_toggle_button_response(&response).show(|ui: &mut egui::Ui| {
        ui.set_min_width(200.0);
        egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque);
        *rgb = [color.r(), color.g(), color.b()];
    });
}

/// Draw an edge toggle swatch: a wide colored rectangle with the label inside.
/// Left-click toggles the edge type on/off. Right-click opens the color picker.
/// Returns `true` if the toggle state changed.
fn edge_toggle_swatch(
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

    // Paint the swatch: full color when enabled, dark grey when disabled.
    let bg = if enabled {
        color
    } else {
        egui::Color32::from_gray(40)
    };
    ui.painter().rect_filled(rect, 3.0, bg);

    // Text color: white when enabled, dim grey when disabled.
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

    // Right-click opens color picker via context menu.
    response
        .clone()
        .on_hover_text("Click: toggle, Right-click: color");
    response.context_menu(|ui: &mut egui::Ui| {
        ui.set_min_width(200.0);
        egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque);
        *rgb = [color.r(), color.g(), color.b()];
    });

    // Left-click toggles.
    response.clicked()
}
