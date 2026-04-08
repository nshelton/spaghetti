//! Central canvas panel: node and edge rendering, pan/zoom, selection.

use core_ir::{EdgeKind, SymbolId, SymbolKind};
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::app::{SpaghettiApp, NODE_HEIGHT, NODE_WIDTH};

impl SpaghettiApp {
    /// Draw the central canvas: nodes and edges.
    pub(crate) fn central_panel(&mut self, ui: &mut egui::Ui) {
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

            // Handle pan (drag)
            if response.dragged_by(egui::PointerButton::Primary)
                && self
                    .hit_test(response.interact_pointer_pos(), canvas_center)
                    .is_none()
            {
                let delta = response.drag_delta();
                self.camera.offset.x += delta.x / self.camera.zoom;
                self.camera.offset.y += delta.y / self.camera.zoom;
            }

            // Handle click (select)
            if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    self.selection = self.hit_test(Some(pointer), canvas_center);
                }
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
    pub(crate) fn hit_test(&self, pointer: Option<Pos2>, canvas_center: Pos2) -> Option<SymbolId> {
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
