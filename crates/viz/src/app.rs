//! The main eframe application for spaghetti.

use std::collections::HashSet;

use core_ir::{EdgeKind, Graph, SymbolId, SymbolKind};
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};
use glam::Vec2 as GVec2;
use layout::{ForceParams, LayoutState, Positions};

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

/// 2D camera for pan and zoom.
struct Camera2D {
    offset: Vec2,
    zoom: f32,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera2D {
    /// Transform a world position to screen position.
    fn world_to_screen(&self, world: GVec2, canvas_center: Pos2) -> Pos2 {
        let x = canvas_center.x + (world.x + self.offset.x) * self.zoom;
        let y = canvas_center.y + (world.y + self.offset.y) * self.zoom;
        Pos2::new(x, y)
    }

    /// Transform a screen position to world position.
    fn screen_to_world(&self, screen: Pos2, canvas_center: Pos2) -> GVec2 {
        let x = (screen.x - canvas_center.x) / self.zoom - self.offset.x;
        let y = (screen.y - canvas_center.y) / self.zoom - self.offset.y;
        GVec2::new(x, y)
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
}

const NODE_WIDTH: f32 = 120.0;
const NODE_HEIGHT: f32 = 30.0;

impl SpaghettiApp {
    /// Create a new app with the given graph and pre-computed positions.
    pub fn new(graph: Graph, positions: Positions) -> Self {
        let layout_state = LayoutState::new(&graph, 42, ForceParams::default());
        Self {
            graph,
            positions,
            layout_state,
            camera: Camera2D::default(),
            selection: None,
            edge_filter: EdgeKindFilter::default(),
            search: String::new(),
            dragging: None,
        }
    }

    /// Create a new app with a live [`LayoutState`] that drives positions
    /// incrementally (used for the interactive path).
    pub fn with_layout_state(graph: Graph, layout_state: LayoutState) -> Self {
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
                self.camera.zoom = (self.camera.zoom * zoom_factor).clamp(0.1, 10.0);
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
            self.layout_state.step(STEPS_PER_FRAME);
            self.positions = self.layout_state.positions();

            // Request repaint while the layout is still settling.
            if self.layout_state.energy() > ENERGY_THRESHOLD || self.dragging.is_some() {
                ui.ctx().request_repaint();
            }

            // Draw edges
            let active_kinds = self.edge_filter.active_kinds();
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
                    let bg = if self.selection == Some(*id) {
                        Color32::from_rgb(80, 140, 220)
                    } else {
                        node_color(sym.kind)
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
        let pointer = pointer?;
        let world = self.camera.screen_to_world(pointer, canvas_center);

        let half_w = NODE_WIDTH / 2.0;
        let half_h = NODE_HEIGHT / 2.0;

        for (id, pos) in &self.positions.0 {
            if (world.x - pos.x).abs() < half_w && (world.y - pos.y).abs() < half_h {
                return Some(*id);
            }
        }
        None
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
        SymbolKind::Class => Color32::from_rgb(70, 130, 180),
        SymbolKind::Struct => Color32::from_rgb(60, 160, 120),
        SymbolKind::Function => Color32::from_rgb(180, 100, 60),
        SymbolKind::Method => Color32::from_rgb(140, 100, 180),
        SymbolKind::Field => Color32::from_rgb(160, 160, 80),
        SymbolKind::Namespace => Color32::from_rgb(100, 100, 100),
        SymbolKind::TemplateInstantiation => Color32::from_rgb(180, 80, 140),
        SymbolKind::TranslationUnit => Color32::from_rgb(80, 80, 80),
        _ => Color32::from_rgb(120, 120, 120),
    }
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
