//! Central canvas panel: node and edge rendering, pan/zoom, dragging, selection.

use std::collections::{HashMap, HashSet};

use egui::{Color32, Rect, Stroke, StrokeKind, Vec2};
use glam::Vec2 as GVec2;

use core_ir::SymbolId;

use crate::app::{SpaghettiApp, ENERGY_THRESHOLD};
use crate::camera::{NODE_HEIGHT, NODE_WIDTH};
use crate::fps::paint_fps_overlay;

/// Scale factor for collapsed container nodes (slightly larger than normal).
const COLLAPSED_CONTAINER_SCALE: f32 = 1.3;
/// World-space padding around children for expanded container backgrounds.
const CONTAINER_PADDING: f32 = 30.0;

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
                if let Some(hit) =
                    self.hit_test_compound(response.interact_pointer_pos(), canvas_center)
                {
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
                    self.selection = self.hit_test_compound(Some(pointer), canvas_center);
                }
            }

            // Handle double-click: toggle expand/collapse on container nodes.
            if response.double_clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if let Some(hit) = self.hit_test_compound(Some(pointer), canvas_center) {
                        if self.layout_state.is_container(hit) {
                            self.layout_state.toggle_expand(hit);
                            self.sync_hidden_symbols();
                        }
                    }
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

            // Press R to randomize the layout.
            if ui.input(|i| i.key_pressed(egui::Key::R)) && !ui.memory(|m| m.focused().is_some()) {
                self.layout_state.randomize();
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

            // --- Build edge rerouting map for collapsed containers ---
            let reroute = self.build_edge_reroute_map();

            // --- 1. Draw expanded container backgrounds ---
            // Store container screen rects for hit-testing.
            self.container_rects.clear();
            for (id, sym) in &self.graph.symbols {
                if !self.layout_state.is_container(*id) || !self.layout_state.is_expanded(*id) {
                    continue;
                }
                if self.hidden_symbols.contains(id) {
                    continue;
                }
                let children = self.layout_state.children_of(*id);
                if children.is_empty() {
                    continue;
                }

                // Compute world-space bounding box of visible children.
                let mut min_w = GVec2::splat(f32::INFINITY);
                let mut max_w = GVec2::splat(f32::NEG_INFINITY);
                let mut has_visible = false;
                for &child_id in &children {
                    if self.hidden_symbols.contains(&child_id) {
                        continue;
                    }
                    if let Some(&pos) = self.positions.0.get(&child_id) {
                        min_w = min_w.min(pos - GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                        max_w = max_w.max(pos + GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                        has_visible = true;
                    }
                }
                if !has_visible {
                    continue;
                }

                // Also include the container node itself in the bounds.
                if let Some(&parent_pos) = self.positions.0.get(id) {
                    min_w = min_w.min(parent_pos - GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                    max_w = max_w.max(parent_pos + GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                }

                // Add padding and leave space for the title at the top.
                let title_height = 20.0; // world units for the title bar
                min_w -= GVec2::new(CONTAINER_PADDING, CONTAINER_PADDING + title_height);
                max_w += GVec2::new(CONTAINER_PADDING, CONTAINER_PADDING);

                // Viewport culling for the container.
                if max_w.x < vis_min.x
                    || min_w.x > vis_max.x
                    || max_w.y < vis_min.y
                    || min_w.y > vis_max.y
                {
                    continue;
                }

                let screen_min = self.camera.world_to_screen(min_w, canvas_center);
                let screen_max = self.camera.world_to_screen(max_w, canvas_center);
                let container_rect = Rect::from_min_max(screen_min, screen_max);

                // Store for hit-testing.
                self.container_rects.push((*id, container_rect));

                // Draw background.
                let base_color = self.render.node_color(sym.kind);
                let bg = with_alpha(base_color, 0.15);
                painter.rect_filled(container_rect, 8.0, bg);
                painter.rect_stroke(
                    container_rect,
                    8.0,
                    Stroke::new(1.0, with_alpha(base_color, 0.4)),
                    StrokeKind::Outside,
                );

                // Draw container title at the top-left.
                if draw_labels {
                    let title_pos =
                        egui::Pos2::new(container_rect.left() + 6.0, container_rect.top() + 2.0);
                    let font = egui::FontId::proportional(13.0 * self.camera.zoom);
                    painter.text(
                        title_pos,
                        egui::Align2::LEFT_TOP,
                        &sym.name,
                        font,
                        with_alpha(Color32::WHITE, 0.8),
                    );
                }
            }

            // --- 2. Draw edges (with rerouting for collapsed containers) ---
            let mut drawn_aggregated: HashSet<(SymbolId, SymbolId, u8)> = HashSet::new();

            for edge in &self.graph.edges {
                if !active_kinds.contains(&edge.kind) {
                    continue;
                }

                // Reroute endpoints through collapsed containers.
                let from_id = reroute.get(&edge.from).copied().unwrap_or(edge.from);
                let to_id = reroute.get(&edge.to).copied().unwrap_or(edge.to);

                // Skip internal edges (both endpoints reroute to same container).
                if from_id == to_id {
                    continue;
                }

                // Skip if either effective endpoint is hidden.
                if self.hidden_symbols.contains(&from_id) || self.hidden_symbols.contains(&to_id) {
                    continue;
                }

                // Dedup aggregated edges: if this is a rerouted edge, only draw
                // one line per (from_container, to_container, kind) triple.
                let is_rerouted =
                    reroute.contains_key(&edge.from) || reroute.contains_key(&edge.to);
                if is_rerouted {
                    let kind_disc = edge.kind as u8;
                    if !drawn_aggregated.insert((from_id, to_id, kind_disc)) {
                        continue;
                    }
                }

                let from_pos = self.positions.0.get(&from_id);
                let to_pos = self.positions.0.get(&to_id);
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
                    let thickness = if is_rerouted { 2.5 } else { 1.5 };
                    painter.line_segment([screen_from, screen_to], Stroke::new(thickness, color));
                }
            }

            // --- 3. Draw nodes ---
            let node_size = Vec2::new(
                NODE_WIDTH * self.camera.zoom,
                NODE_HEIGHT * self.camera.zoom,
            );
            let container_node_size = node_size * COLLAPSED_CONTAINER_SCALE;

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

                    let is_container = self.layout_state.is_container(*id);
                    let is_collapsed = is_container && !self.layout_state.is_expanded(*id);

                    if draw_rects {
                        let size = if is_collapsed {
                            container_node_size
                        } else {
                            node_size
                        };
                        let rect = Rect::from_center_size(screen_pos, size);

                        let bg = if self.selection == Some(*id) {
                            brighten(base, 2.0)
                        } else {
                            base
                        };
                        painter.rect_filled(rect, 4.0, bg);

                        // Collapsed containers get a thicker border.
                        let border_width = if is_collapsed { 2.0 } else { 1.0 };
                        painter.rect_stroke(
                            rect,
                            4.0,
                            Stroke::new(border_width, Color32::from_gray(60)),
                            StrokeKind::Outside,
                        );

                        // Label (only when zoomed in enough)
                        if draw_labels {
                            let font = egui::FontId::proportional(12.0 * self.camera.zoom);
                            let label = if is_collapsed {
                                let n = self.layout_state.children_of(*id).len();
                                format!("{} (+{})", sym.name, n)
                            } else {
                                sym.name.clone()
                            };
                            painter.text(
                                screen_pos,
                                egui::Align2::CENTER_CENTER,
                                &label,
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
                            let base_r = circle_radius * self.camera.zoom;
                            if is_collapsed {
                                base_r * COLLAPSED_CONTAINER_SCALE
                            } else {
                                base_r
                            }
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

    /// Build a map from child symbol ID -> collapsed parent symbol ID
    /// for edge rerouting.
    fn build_edge_reroute_map(&self) -> HashMap<SymbolId, SymbolId> {
        let mut reroute = HashMap::new();
        for (id, _sym) in &self.graph.symbols {
            if self.layout_state.is_container(*id) && !self.layout_state.is_expanded(*id) {
                for child_id in self.layout_state.children_of(*id) {
                    reroute.insert(child_id, *id);
                }
            }
        }
        reroute
    }

    /// Hit-test that accounts for expanded container backgrounds.
    ///
    /// Checks children first (they're on top), then regular nodes,
    /// then expanded container backgrounds.
    fn hit_test_compound(
        &self,
        pointer: Option<egui::Pos2>,
        canvas_center: egui::Pos2,
    ) -> Option<SymbolId> {
        // First try the normal node hit-test (children and regular nodes).
        let radius = if self.render.circle_mode {
            Some(self.render.circle_radius)
        } else {
            None
        };
        if let Some(hit) = crate::camera::hit_test(
            &self.camera,
            &self.positions,
            pointer,
            canvas_center,
            radius,
        ) {
            // Only return if the node is visible (not hidden by collapse/file-tree).
            if !self.hidden_symbols.contains(&hit) {
                return Some(hit);
            }
        }

        // Then check expanded container background rects.
        let pointer = pointer?;
        for &(id, rect) in self.container_rects.iter().rev() {
            if rect.contains(pointer) {
                return Some(id);
            }
        }

        None
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
