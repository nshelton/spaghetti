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
                    let mut node_counts: std::collections::HashMap<core_ir::SymbolKind, usize> =
                        std::collections::HashMap::new();
                    for sym in self.graph.graph.symbols.values() {
                        *node_counts.entry(sym.kind).or_insert(0) += 1;
                    }
                    let mut node_filter_changed = false;
                    for &kind in &ALL_SYMBOL_KINDS {
                        let count = node_counts.get(&kind).copied().unwrap_or(0);
                        let kind_name = format!("{kind:?}");
                        let label = format!("{kind_name} ({count})");
                        let enabled = self.filters.node_filter.is_enabled(kind);
                        if toggle_swatch(
                            ui,
                            &mut self.render.render.node_colors,
                            &kind_name,
                            &label,
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

                    let mut edge_counts: std::collections::HashMap<core_ir::EdgeKind, usize> =
                        std::collections::HashMap::new();
                    for edge in &self.graph.graph.edges {
                        *edge_counts.entry(edge.kind).or_insert(0) += 1;
                    }
                    let mut edge_filter_changed = false;
                    for &kind in &ALL_EDGE_KINDS {
                        let count = edge_counts.get(&kind).copied().unwrap_or(0);
                        let kind_name = format!("{kind:?}");
                        let label = format!("{kind_name} ({count})");
                        let enabled = self.filters.edge_filter.is_enabled(kind);
                        if toggle_swatch(
                            ui,
                            &mut self.render.render.edge_colors,
                            &kind_name,
                            &label,
                            enabled,
                        ) {
                            self.filters.edge_filter.toggle(kind);
                            edge_filter_changed = true;
                        }
                        if enabled {
                            if let Some(spring) = self
                                .simulation
                                .layout_state
                                .force_mut::<layout::forces::SpringAttraction>()
                            {
                                let ep = spring.edge_params.entry(kind).or_insert(default_ep);
                                ui.indent(format!("edge_sliders_{kind:?}"), |ui| {
                                    changed |= ui
                                        .add(
                                            egui::Slider::new(&mut ep.target_distance, 1.0..=100.0)
                                                .text("Dist"),
                                        )
                                        .changed();
                                    changed |= ui
                                        .add(
                                            egui::Slider::new(&mut ep.attraction, 0.0..=100.0)
                                                .text("Attract"),
                                        )
                                        .changed();
                                });
                            }
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

                    // -- Layout Forces section --
                    ui.heading("Layout Forces");
                    ui.separator();

                    let state = &mut self.simulation.layout_state;

                    force_section::<layout::forces::Repulsion>(
                        ui,
                        state,
                        "force_repulsion",
                        "Repulsion",
                        &mut changed,
                        |ui, r, changed| {
                            *changed |= ui
                                .add(
                                    egui::Slider::new(&mut r.strength, 100.0..=1_000_000.0)
                                        .logarithmic(true)
                                        .text("Strength"),
                                )
                                .changed();
                        },
                    );

                    // Edge Springs: per-kind sliders live in the Edge Types
                    // section above; this block is just the on/off checkbox.
                    force_section::<layout::forces::SpringAttraction>(
                        ui,
                        state,
                        "force_attraction",
                        "Edge Springs",
                        &mut changed,
                        |ui, _, _| {
                            ui.label("Per-edge-kind sliders are above in Edge Types.");
                        },
                    );

                    force_section::<layout::forces::Gravity>(
                        ui,
                        state,
                        "force_gravity",
                        "Gravity",
                        &mut changed,
                        |ui, g, changed| {
                            *changed |= ui
                                .add(egui::Slider::new(&mut g.strength, 0.0..=0.1).text("Strength"))
                                .changed();
                        },
                    );

                    force_section::<layout::forces::LocationAffinity>(
                        ui,
                        state,
                        "force_location",
                        "Location Affinity",
                        &mut changed,
                        |ui, l, changed| {
                            *changed |= ui
                                .add(egui::Slider::new(&mut l.strength, 0.0..=2.0).text("Strength"))
                                .changed();
                            *changed |= ui
                                .add(egui::Slider::new(&mut l.falloff, 0.0..=1.0).text("Falloff"))
                                .changed();
                        },
                    );

                    force_section::<layout::forces::Containment>(
                        ui,
                        state,
                        "force_containment",
                        "Containment",
                        &mut changed,
                        |ui, c, changed| {
                            *changed |= ui
                                .add(egui::Slider::new(&mut c.strength, 0.0..=2.0).text("Strength"))
                                .changed();
                        },
                    );

                    force_section::<layout::forces::ContainerRepulsion>(
                        ui,
                        state,
                        "force_container_repulsion",
                        "Container Repulsion",
                        &mut changed,
                        |ui, cr, changed| {
                            *changed |= ui
                                .add(
                                    egui::Slider::new(&mut cr.strength, 10.0..=100_000.0)
                                        .logarithmic(true)
                                        .text("Strength"),
                                )
                                .changed();
                        },
                    );

                    ui.add_space(16.0);

                    // -- Simulation controls --
                    ui.heading("Simulation");
                    ui.separator();

                    {
                        let state = &mut self.simulation.layout_state;
                        changed |= ui
                            .add(egui::Slider::new(&mut state.damping, 0.01..=0.99).text("Damping"))
                            .changed();
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut state.max_velocity, 1.0..=200.0)
                                    .text("Max vel"),
                            )
                            .changed();
                    }

                    ui.add_space(4.0);
                    if ui.button("Reset to defaults").clicked() {
                        self.simulation
                            .layout_state
                            .import_params(&layout::ForceParams::default());
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
    label: &str,
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
        label,
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

/// Draw a collapsing header for a single layout force.
///
/// Reads the force's `enabled` flag from the pipeline, renders a
/// checkbox + body, then writes the flag back. The `body` closure
/// receives a fresh mutable borrow of the concrete force and a change
/// flag it should set when any slider moves a value. Returns without
/// drawing anything if the pipeline has no force of type `T`.
fn force_section<T: layout::forces::Force>(
    ui: &mut egui::Ui,
    state: &mut layout::LayoutState,
    id_str: &str,
    label: &str,
    changed: &mut bool,
    body: impl FnOnce(&mut egui::Ui, &mut T, &mut bool),
) {
    let Some(mut enabled) = state.force::<T>().map(|f| f.enabled()) else {
        return;
    };
    let id = ui.make_persistent_id(id_str);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            if ui.checkbox(&mut enabled, label).changed() {
                *changed = true;
            }
        })
        .body(|ui| {
            if !enabled {
                ui.disable();
            }
            if let Some(f) = state.force_mut::<T>() {
                body(ui, f, changed);
            }
        });
    if let Some(f) = state.force_mut::<T>() {
        f.set_enabled(enabled);
    }
}
