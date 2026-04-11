//! Central canvas panel: node and edge rendering, pan/zoom, dragging, selection.

use egui::{Color32, Rect, Stroke, StrokeKind, Vec2};
use glam::Vec2 as GVec2;

use crate::app::{SpaghettiApp, ENERGY_THRESHOLD};
use crate::camera::{NODE_HEIGHT, NODE_WIDTH};
use crate::fps::paint_fps_overlay;

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
            // Use a time budget so large graphs don't block the frame.
            let active_kinds = self.edge_filter.active_kinds();
            self.layout_state.set_visible_edge_kinds(&active_kinds);
            self.layout_state
                .step_budgeted(std::time::Duration::from_millis(8));
            self.positions = self.layout_state.positions();

            // Auto-fit camera once the layout settles or after a timeout.
            let energy = self.layout_state.energy();
            self.frame_count = self.frame_count.saturating_add(1);
            let auto_fit_timeout = self.frame_count >= 120; // ~2s at 60fps
            if !self.auto_fitted && (energy < ENERGY_THRESHOLD || auto_fit_timeout) {
                self.camera
                    .fit_to_bounds(&self.positions, response.rect.size());
                self.auto_fitted = true;
            }

            // Press F to re-frame the view.
            if ui.input(|i| i.key_pressed(egui::Key::F)) && !ui.memory(|m| m.focused().is_some()) {
                self.camera
                    .fit_to_bounds(&self.positions, response.rect.size());
            }

            // Request repaint while the layout is still settling.
            if energy > ENERGY_THRESHOLD || self.dragging.is_some() {
                ui.ctx().request_repaint();
            }

            // Viewport culling: compute the visible world-space rectangle
            // with a margin so partially-visible nodes/edges aren't clipped.
            let margin = NODE_WIDTH; // world-space margin
            let (vis_min, vis_max) = self.camera.visible_world_rect(response.rect);
            let vis_min = GVec2::new(vis_min.x - margin, vis_min.y - margin);
            let vis_max = GVec2::new(vis_max.x + margin, vis_max.y + margin);

            // LOD thresholds based on zoom level.
            let circle_mode = self.render.circle_mode;
            let circle_radius = self.render.circle_radius;
            let draw_labels = !circle_mode && self.camera.zoom >= 0.4;
            let draw_rects = !circle_mode && self.camera.zoom >= 0.15;

            // Draw edges (skip if both endpoints are outside the viewport)
            for edge in &self.graph.edges {
                if !active_kinds.contains(&edge.kind) {
                    continue;
                }
                if self.hidden_symbols.contains(&edge.from)
                    || self.hidden_symbols.contains(&edge.to)
                {
                    continue;
                }
                let from_pos = self.positions.0.get(&edge.from);
                let to_pos = self.positions.0.get(&edge.to);
                if let (Some(&from), Some(&to)) = (from_pos, to_pos) {
                    // Cull: skip edge if both endpoints are outside the viewport.
                    let from_visible = from.x >= vis_min.x
                        && from.x <= vis_max.x
                        && from.y >= vis_min.y
                        && from.y <= vis_max.y;
                    let to_visible = to.x >= vis_min.x
                        && to.x <= vis_max.x
                        && to.y >= vis_min.y
                        && to.y <= vis_max.y;
                    if !from_visible && !to_visible {
                        continue;
                    }

                    let screen_from = self.camera.world_to_screen(from, canvas_center);
                    let screen_to = self.camera.world_to_screen(to, canvas_center);

                    let color =
                        with_alpha(self.render.edge_color(edge.kind), self.render.edge_opacity);
                    painter.line_segment([screen_from, screen_to], Stroke::new(1.5, color));
                }
            }

            // Draw nodes
            let node_size = Vec2::new(
                NODE_WIDTH * self.camera.zoom,
                NODE_HEIGHT * self.camera.zoom,
            );
            for (id, sym) in &self.graph.symbols {
                if self.hidden_symbols.contains(id) {
                    continue;
                }
                if let Some(&world_pos) = self.positions.0.get(id) {
                    // Viewport culling: skip nodes entirely outside the visible area.
                    if world_pos.x < vis_min.x
                        || world_pos.x > vis_max.x
                        || world_pos.y < vis_min.y
                        || world_pos.y > vis_max.y
                    {
                        continue;
                    }

                    let screen_pos = self.camera.world_to_screen(world_pos, canvas_center);
                    let base =
                        with_alpha(self.render.node_color(sym.kind), self.render.node_opacity);

                    if draw_rects {
                        let rect = Rect::from_center_size(screen_pos, node_size);

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

                        // Label (only when zoomed in enough)
                        if draw_labels {
                            let font = egui::FontId::proportional(12.0 * self.camera.zoom);
                            painter.text(
                                screen_pos,
                                egui::Align2::CENTER_CENTER,
                                &sym.name,
                                font,
                                Color32::WHITE,
                            );
                        }
                    } else {
                        // Circle mode or very low zoom: draw a filled circle.
                        let color = if self.selection == Some(*id) {
                            brighten(base, 2.0)
                        } else {
                            base
                        };
                        let r = if circle_mode {
                            circle_radius * self.camera.zoom
                        } else {
                            2.0
                        };
                        painter.circle_filled(screen_pos, r, color);
                    }
                }
            }

            // FPS overlay
            self.fps.tick();
            paint_fps_overlay(ui, response.rect, self.fps.fps());
        });
    }
}

/// Apply an opacity factor (0.0–1.0) to a color's alpha channel.
fn with_alpha(color: Color32, opacity: f32) -> Color32 {
    let a = (opacity.clamp(0.0, 1.0) * 255.0) as u8;
    Color32::from_rgba_premultiplied(
        (color.r() as u16 * a as u16 / 255) as u8,
        (color.g() as u16 * a as u16 / 255) as u8,
        (color.b() as u16 * a as u16 / 255) as u8,
        a,
    )
}

/// Brighten a color by a multiplier, clamping to 255.
fn brighten(color: Color32, factor: f32) -> Color32 {
    Color32::from_rgb(
        (color.r() as f32 * factor).min(255.0) as u8,
        (color.g() as f32 * factor).min(255.0) as u8,
        (color.b() as f32 * factor).min(255.0) as u8,
    )
}
