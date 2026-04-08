//! Central canvas panel: node and edge rendering, pan/zoom, dragging, selection.

use core_ir::{EdgeKind, SymbolKind};
use egui::{Color32, Rect, Stroke, StrokeKind, Vec2};

use crate::app::{SpaghettiApp, ENERGY_THRESHOLD, STEPS_PER_FRAME};
use crate::camera::{NODE_HEIGHT, NODE_WIDTH};

impl SpaghettiApp {
    /// Draw the central canvas: nodes and edges with interactive dragging.
    pub(crate) fn central_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let (response, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());

            let canvas_center = response.rect.center();

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
}

/// Color for a node based on its symbol kind.
pub(crate) fn node_color(kind: SymbolKind) -> Color32 {
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

/// Color for an edge based on its kind.
pub(crate) fn edge_color(kind: EdgeKind) -> Color32 {
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
