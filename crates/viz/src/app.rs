//! The main eframe application for spaghetti.

use std::collections::HashSet;
use std::time::Instant;

use core_ir::{EdgeKind, Graph, SymbolId, SymbolKind};
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use layout::{LayoutState, Positions};
use tracing::info;

use crate::camera::{self, Camera2D, NODE_HEIGHT, NODE_WIDTH};

/// Edge kind filter state.
struct EdgeKindFilter {
    calls: bool,
    inherits: bool,
    contains: bool,
    overrides: bool,
}

impl Default for EdgeKindFilter {
    fn default() -> Self {
        Self {
            calls: true,
            inherits: true,
            contains: true,
            overrides: true,
        }
    }
}

impl EdgeKindFilter {
    fn active_kinds(&self) -> Vec<EdgeKind> {
        let mut kinds = Vec::new();
        if self.calls {
            kinds.push(EdgeKind::Calls);
        }
        if self.inherits {
            kinds.push(EdgeKind::Inherits);
        }
        if self.contains {
            kinds.push(EdgeKind::Contains);
        }
        if self.overrides {
            kinds.push(EdgeKind::Overrides);
        }
        kinds
    }
}

/// Energy threshold below which the simulation is considered settled and
/// repaints are no longer requested.
const ENERGY_THRESHOLD: f32 = 0.5;

/// Number of force-simulation steps to run each frame while the layout is
/// still settling.
const STEPS_PER_FRAME: u32 = 3;

/// Main application state.
pub struct SpaghettiApp {
    graph: Graph,
    positions: Positions,
    layout_state: LayoutState,
    camera: Camera2D,
    selection: Option<SymbolId>,
    edge_filter: EdgeKindFilter,
    search: String,
    /// The node currently being dragged, if any.
    dragging: Option<SymbolId>,
    /// Whether we have already logged convergence.
    converged: bool,
    /// Timestamp when the app was created (for time-to-convergence).
    created_at: Instant,
    /// Frame counter for periodic per-frame logging.
    frame_count: u64,
}

impl SpaghettiApp {
    /// Create a new app with a live [`LayoutState`] that drives positions
    /// incrementally each frame.
    pub fn new(graph: Graph, layout_state: LayoutState) -> Self {
        let positions = layout_state.positions();
        Self {
            graph,
            positions,
            layout_state,
            camera: Camera2D::default(),
            selection: None,
            edge_filter: EdgeKindFilter::default(),
            search: String::new(),
            dragging: None,
            converged: false,
            created_at: Instant::now(),
            frame_count: 0,
        }
    }

    /// Draw the left panel: search, filters, symbol list.
    fn left_panel(&mut self, ui: &mut egui::Ui) {
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

    /// Draw the right panel: details of selected symbol.
    fn right_panel(&self, ui: &mut egui::Ui) {
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

    /// Draw the central canvas: nodes and edges.
    fn central_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let (response, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());

            let canvas_rect = response.rect;
            let canvas_center = canvas_rect.center();

            // Handle zoom
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_delta != 0.0 {
                let zoom_factor = 1.0 + scroll_delta * 0.002;
                self.camera.apply_zoom(zoom_factor);
            }

            // --- Drag / pan / click interaction ---

            // Drag started: determine whether we are dragging a node or panning.
            if response.drag_started_by(egui::PointerButton::Primary) {
                if let Some(hit) = self.hit_test(response.interact_pointer_pos(), canvas_center) {
                    // Begin dragging a node.
                    self.dragging = Some(hit);
                    self.selection = Some(hit);
                    if let Some(&world) = self.positions.0.get(&hit) {
                        self.layout_state.pin(hit, world);
                    }
                }
            }

            // Ongoing drag.
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(dragged_id) = self.dragging {
                    // Move the pinned node to follow the cursor.
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let world = self.camera.screen_to_world(pointer, canvas_center);
                        self.layout_state.set_position(dragged_id, world);
                    }
                } else {
                    // No node drag — pan the camera.
                    let delta = response.drag_delta();
                    self.camera.offset.x += delta.x / self.camera.zoom;
                    self.camera.offset.y += delta.y / self.camera.zoom;
                }
            }

            // Drag released: unpin the node.
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let Some(dragged_id) = self.dragging.take() {
                    self.layout_state.unpin(dragged_id);
                }
            }

            // Handle click (select) — only when not dragging.
            if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    self.selection = self.hit_test(Some(pointer), canvas_center);
                }
            }

            // --- Run incremental simulation ---
            let active_kinds = self.edge_filter.active_kinds();
            self.layout_state.set_visible_edge_kinds(&active_kinds);

            let step_start = Instant::now();
            self.layout_state.step(STEPS_PER_FRAME);
            let step_elapsed = step_start.elapsed();
            self.positions = self.layout_state.positions();
            self.frame_count += 1;

            let energy = self.layout_state.energy();

            // Log per-frame step time periodically (every 60 frames ≈ once per second)
            if self.frame_count.is_multiple_of(60) && energy > ENERGY_THRESHOLD {
                info!(
                    frame = self.frame_count,
                    step_us = step_elapsed.as_micros(),
                    energy = format!("{:.2}", energy),
                    "per-frame layout step"
                );
            }

            // Log convergence once
            if !self.converged && energy <= ENERGY_THRESHOLD {
                self.converged = true;
                let time_to_converge = self.created_at.elapsed();
                info!(
                    frames = self.frame_count,
                    time_ms = format!("{:.1}", time_to_converge.as_secs_f64() * 1000.0),
                    final_energy = format!("{:.4}", energy),
                    "layout converged"
                );
            }

            // Request repaint while the layout is still settling.
            if energy > ENERGY_THRESHOLD || self.dragging.is_some() {
                ui.ctx().request_repaint();
            }

            // Draw edges
            for edge in &self.graph.edges {
                if !active_kinds.contains(&edge.kind) {
                    continue;
                }
                let from_pos = self.positions.0.get(&edge.from);
                let to_pos = self.positions.0.get(&edge.to);
                if let (Some(&from), Some(&to)) = (from_pos, to_pos) {
                    let screen_from = self.camera.world_to_screen(from, canvas_center);
                    let screen_to = self.camera.world_to_screen(to, canvas_center);

                    let color = edge_color(edge.kind);
                    painter.line_segment([screen_from, screen_to], Stroke::new(1.5, color));
                }
            }

            // Draw nodes
            let node_size = Vec2::new(
                NODE_WIDTH * self.camera.zoom,
                NODE_HEIGHT * self.camera.zoom,
            );
            for (id, sym) in &self.graph.symbols {
                if let Some(&world_pos) = self.positions.0.get(id) {
                    let screen_pos = self.camera.world_to_screen(world_pos, canvas_center);
                    let rect = Rect::from_center_size(screen_pos, node_size);

                    // Background
                    let base = node_color(sym.kind);
                    let bg = if self.selection == Some(*id) {
                        brighten(base, 2.0)
                    } else {
                        base
                    };
                    painter.rect_filled(rect, 4.0, bg);
                    painter.rect_stroke(
                        rect,
                        4.0,
                        Stroke::new(1.0, Color32::from_gray(60)),
                        StrokeKind::Outside,
                    );

                    // Label
                    let font = egui::FontId::proportional(12.0 * self.camera.zoom);
                    let text_color = Color32::WHITE;
                    painter.text(
                        screen_pos,
                        egui::Align2::CENTER_CENTER,
                        &sym.name,
                        font,
                        text_color,
                    );
                }
            }
        });
    }

    /// Hit-test: find which symbol (if any) is under the pointer.
    // TODO: quadtree for efficient hit testing
    fn hit_test(&self, pointer: Option<Pos2>, canvas_center: Pos2) -> Option<SymbolId> {
        camera::hit_test(&self.camera, &self.positions, pointer, canvas_center)
    }
}

impl eframe::App for SpaghettiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.left_panel(ui);
        self.right_panel(ui);
        self.central_panel(ui);
    }
}

fn node_color(kind: SymbolKind) -> Color32 {
    match kind {
        SymbolKind::Class => Color32::from_rgb(30, 55, 80),
        SymbolKind::Struct => Color32::from_rgb(25, 70, 50),
        SymbolKind::Function => Color32::from_rgb(80, 45, 25),
        SymbolKind::Method => Color32::from_rgb(60, 45, 80),
        SymbolKind::Field => Color32::from_rgb(70, 70, 35),
        SymbolKind::Namespace => Color32::from_rgb(45, 45, 45),
        SymbolKind::TemplateInstantiation => Color32::from_rgb(80, 35, 60),
        SymbolKind::TranslationUnit => Color32::from_rgb(35, 35, 35),
        _ => Color32::from_rgb(50, 50, 50),
    }
}

/// Brighten a color by a multiplier, clamping to 255.
fn brighten(color: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (color.r() as f32 * factor).min(255.0) as u8,
        (color.g() as f32 * factor).min(255.0) as u8,
        (color.b() as f32 * factor).min(255.0) as u8,
    )
}

fn edge_color(kind: EdgeKind) -> Color32 {
    match kind {
        EdgeKind::Calls => Color32::from_rgb(220, 180, 80),
        EdgeKind::Inherits => Color32::from_rgb(100, 200, 100),
        EdgeKind::Contains => Color32::from_rgb(150, 150, 150),
        EdgeKind::Overrides => Color32::from_rgb(180, 120, 220),
        EdgeKind::ReadsField => Color32::from_rgb(100, 180, 220),
        EdgeKind::WritesField => Color32::from_rgb(220, 100, 100),
        EdgeKind::Includes => Color32::from_rgb(160, 160, 160),
        EdgeKind::Instantiates => Color32::from_rgb(200, 140, 100),
        EdgeKind::HasType => Color32::from_rgb(140, 140, 200),
        _ => Color32::from_rgb(128, 128, 128),
    }
}
