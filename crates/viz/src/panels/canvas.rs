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
                self.render.camera.apply_zoom(zoom_factor);
            }

            // --- Drag / pan / click interaction ---

            // Drag started: determine whether we are dragging a node or panning.
            if response.drag_started_by(egui::PointerButton::Primary) {
                if let Some(hit) =
                    self.hit_test_compound(response.interact_pointer_pos(), canvas_center)
                {
                    self.interaction.dragging = Some(hit);
                    self.interaction.selection = Some(hit);
                    if let Some(&world) = self.simulation.positions.0.get(&hit) {
                        self.simulation.layout_state.pin(hit, world);
                    }
                }
            }

            // Ongoing drag.
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(dragged_id) = self.interaction.dragging {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let world = self.render.camera.screen_to_world(pointer, canvas_center);
                        self.simulation.layout_state.set_position(dragged_id, world);
                    }
                } else {
                    let delta = response.drag_delta();
                    self.render.camera.offset.x += delta.x / self.render.camera.zoom;
                    self.render.camera.offset.y += delta.y / self.render.camera.zoom;
                }
            }

            // Drag released: unpin the node.
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let Some(dragged_id) = self.interaction.dragging.take() {
                    self.simulation.layout_state.unpin(dragged_id);
                }
            }

            // Handle click (select) — only when not dragging.
            if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    self.interaction.selection =
                        self.hit_test_compound(Some(pointer), canvas_center);
                }
            }

            // Handle double-click: toggle expand/collapse on container nodes.
            if response.double_clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if let Some(hit) = self.hit_test_compound(Some(pointer), canvas_center) {
                        if self.simulation.layout_state.is_container(hit) {
                            self.simulation.layout_state.toggle_expand(hit);
                            self.filters
                                .sync_hidden_symbols(&self.graph, &mut self.simulation);
                        }
                    }
                }
            }

            // Press P to toggle pause on the layout simulation.
            if ui.input(|i| i.key_pressed(egui::Key::P)) && !ui.memory(|m| m.focused().is_some()) {
                self.simulation.paused = !self.simulation.paused;
            }

            // --- Run incremental simulation ---
            let active_kinds = self.filters.edge_filter.active_kinds();
            self.simulation
                .layout_state
                .set_visible_edge_kinds(&active_kinds);
            if !self.simulation.paused {
                self.simulation
                    .layout_state
                    .step_budgeted(std::time::Duration::from_millis(8));
            }
            self.simulation.positions = self.simulation.layout_state.positions();

            // Auto-fit camera once the layout settles or after a timeout.
            let energy = self.simulation.layout_state.energy();
            self.interaction.frame_count = self.interaction.frame_count.saturating_add(1);
            let auto_fit_timeout = self.interaction.frame_count >= 120;
            if !self.interaction.auto_fitted && (energy < ENERGY_THRESHOLD || auto_fit_timeout) {
                self.render.camera.fit_to_bounds(
                    &self.simulation.positions,
                    &self.simulation.node_sizes,
                    response.rect.size(),
                );
                self.interaction.auto_fitted = true;
            }

            // Press F to re-frame the view.
            if ui.input(|i| i.key_pressed(egui::Key::F)) && !ui.memory(|m| m.focused().is_some()) {
                self.render.camera.fit_to_bounds(
                    &self.simulation.positions,
                    &self.simulation.node_sizes,
                    response.rect.size(),
                );
            }

            // Press R to randomize the layout.
            if ui.input(|i| i.key_pressed(egui::Key::R)) && !ui.memory(|m| m.focused().is_some()) {
                self.simulation.layout_state.randomize();
            }

            // Press J to juggle (slightly perturb) the layout.
            if ui.input(|i| i.key_pressed(egui::Key::J)) && !ui.memory(|m| m.focused().is_some()) {
                self.simulation.layout_state.juggle();
            }

            // Request repaint while the layout is still settling.
            if energy > ENERGY_THRESHOLD || self.interaction.dragging.is_some() {
                ui.ctx().request_repaint();
            }

            // Viewport culling
            let margin = NODE_WIDTH;
            let (vis_min, vis_max) = self.render.camera.visible_world_rect(response.rect);
            let vis_min = GVec2::new(vis_min.x - margin, vis_min.y - margin);
            let vis_max = GVec2::new(vis_max.x + margin, vis_max.y + margin);

            // LOD thresholds based on zoom level.
            let circle_mode = self.render.render.circle_mode;
            let circle_radius = self.render.render.circle_radius;
            let draw_labels = !circle_mode && self.render.camera.zoom >= 0.4;
            let draw_rects = !circle_mode && self.render.camera.zoom >= 0.15;

            // --- Build edge rerouting map for collapsed containers ---
            let reroute = self.build_edge_reroute_map();

            // --- 1. Draw expanded container backgrounds ---
            self.simulation.container_rects.clear();
            for (id, sym) in &self.graph.graph.symbols {
                if !self.simulation.layout_state.is_container(*id)
                    || !self.simulation.layout_state.is_expanded(*id)
                {
                    continue;
                }
                if self.filters.hidden_symbols.contains(id) {
                    continue;
                }
                let children = self.simulation.layout_state.children_of(*id);
                if children.is_empty() {
                    continue;
                }

                let mut min_w = GVec2::splat(f32::INFINITY);
                let mut max_w = GVec2::splat(f32::NEG_INFINITY);
                let mut has_visible = false;
                for &child_id in &children {
                    if self.filters.hidden_symbols.contains(&child_id) {
                        continue;
                    }
                    if let Some(&pos) = self.simulation.positions.0.get(&child_id) {
                        min_w = min_w.min(pos - GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                        max_w = max_w.max(pos + GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                        has_visible = true;
                    }
                }
                if !has_visible {
                    continue;
                }

                if let Some(&parent_pos) = self.simulation.positions.0.get(id) {
                    min_w = min_w.min(parent_pos - GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                    max_w = max_w.max(parent_pos + GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
                }

                let title_height = 20.0;
                min_w -= GVec2::new(CONTAINER_PADDING, CONTAINER_PADDING + title_height);
                max_w += GVec2::new(CONTAINER_PADDING, CONTAINER_PADDING);

                if max_w.x < vis_min.x
                    || min_w.x > vis_max.x
                    || max_w.y < vis_min.y
                    || min_w.y > vis_max.y
                {
                    continue;
                }

                let screen_min = self.render.camera.world_to_screen(min_w, canvas_center);
                let screen_max = self.render.camera.world_to_screen(max_w, canvas_center);
                let container_rect = Rect::from_min_max(screen_min, screen_max);

                self.simulation.container_rects.push((*id, container_rect));

                let base_color = self.render.render.node_color(sym.kind);
                let bg = with_alpha(base_color, 0.15);
                painter.rect_filled(container_rect, 8.0, bg);
                painter.rect_stroke(
                    container_rect,
                    8.0,
                    Stroke::new(1.0, with_alpha(base_color, 0.4)),
                    StrokeKind::Outside,
                );

                if draw_labels {
                    let title_pos =
                        egui::Pos2::new(container_rect.left() + 6.0, container_rect.top() + 2.0);
                    let font = egui::FontId::proportional(13.0 * self.render.camera.zoom);
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

            for edge in &self.graph.graph.edges {
                if !active_kinds.contains(&edge.kind) {
                    continue;
                }

                let from_id = reroute.get(&edge.from).copied().unwrap_or(edge.from);
                let to_id = reroute.get(&edge.to).copied().unwrap_or(edge.to);

                if from_id == to_id {
                    continue;
                }

                if self.filters.hidden_symbols.contains(&from_id)
                    || self.filters.hidden_symbols.contains(&to_id)
                {
                    continue;
                }

                let from_visible_kind = self
                    .graph
                    .graph
                    .symbols
                    .get(&edge.from)
                    .is_some_and(|s| self.filters.node_filter.is_enabled(s.kind));
                let to_visible_kind = self
                    .graph
                    .graph
                    .symbols
                    .get(&edge.to)
                    .is_some_and(|s| self.filters.node_filter.is_enabled(s.kind));
                if !from_visible_kind || !to_visible_kind {
                    continue;
                }

                let is_rerouted =
                    reroute.contains_key(&edge.from) || reroute.contains_key(&edge.to);
                if is_rerouted {
                    let kind_disc = edge.kind as u8;
                    if !drawn_aggregated.insert((from_id, to_id, kind_disc)) {
                        continue;
                    }
                }

                let from_pos = self.simulation.positions.0.get(&from_id);
                let to_pos = self.simulation.positions.0.get(&to_id);
                if let (Some(&from), Some(&to)) = (from_pos, to_pos) {
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

                    let screen_from = self.render.camera.world_to_screen(from, canvas_center);
                    let screen_to = self.render.camera.world_to_screen(to, canvas_center);

                    let color = with_alpha(
                        self.render.render.edge_color(edge.kind),
                        self.render.render.edge_opacity,
                    );
                    let thickness = if is_rerouted { 2.5 } else { 1.5 };
                    painter.line_segment([screen_from, screen_to], Stroke::new(thickness, color));
                }
            }

            // --- 3. Draw nodes ---
            let nodes_with_edges: Option<std::collections::HashSet<core_ir::SymbolId>> =
                if self.filters.hide_edgeless {
                    let mut set = std::collections::HashSet::new();
                    for edge in &self.graph.graph.edges {
                        if !active_kinds.contains(&edge.kind) {
                            continue;
                        }
                        let from_ok = !self.filters.hidden_symbols.contains(&edge.from)
                            && self
                                .graph
                                .graph
                                .symbols
                                .get(&edge.from)
                                .is_some_and(|s| self.filters.node_filter.is_enabled(s.kind));
                        let to_ok = !self.filters.hidden_symbols.contains(&edge.to)
                            && self
                                .graph
                                .graph
                                .symbols
                                .get(&edge.to)
                                .is_some_and(|s| self.filters.node_filter.is_enabled(s.kind));
                        if from_ok && to_ok {
                            set.insert(edge.from);
                            set.insert(edge.to);
                        }
                    }
                    Some(set)
                } else {
                    None
                };
            let node_size = Vec2::new(
                NODE_WIDTH * self.render.camera.zoom,
                NODE_HEIGHT * self.render.camera.zoom,
            );
            let container_node_size = node_size * COLLAPSED_CONTAINER_SCALE;

            for (id, sym) in &self.graph.graph.symbols {
                if self.filters.hidden_symbols.contains(id) {
                    continue;
                }
                if !self.filters.node_filter.is_enabled(sym.kind) {
                    continue;
                }
                if let Some(ref with_edges) = nodes_with_edges {
                    if !with_edges.contains(id) {
                        continue;
                    }
                }
                if let Some(&world_pos) = self.simulation.positions.0.get(id) {
                    if world_pos.x < vis_min.x
                        || world_pos.x > vis_max.x
                        || world_pos.y < vis_min.y
                        || world_pos.y > vis_max.y
                    {
                        continue;
                    }

                    let screen_pos = self.render.camera.world_to_screen(world_pos, canvas_center);
                    let base = with_alpha(
                        self.render.render.node_color(sym.kind),
                        self.render.render.node_opacity,
                    );

                    let is_container = self.simulation.layout_state.is_container(*id);
                    let is_collapsed =
                        is_container && !self.simulation.layout_state.is_expanded(*id);

                    if draw_rects {
                        let size = if is_collapsed {
                            container_node_size
                        } else {
                            node_size
                        };
                        let rect = Rect::from_center_size(screen_pos, size);

                        let bg = if self.interaction.selection == Some(*id) {
                            brighten(base, 2.0)
                        } else {
                            base
                        };
                        painter.rect_filled(rect, 4.0, bg);

                        let border_width = if is_collapsed { 2.0 } else { 1.0 };
                        painter.rect_stroke(
                            rect,
                            4.0,
                            Stroke::new(border_width, Color32::from_gray(60)),
                            StrokeKind::Outside,
                        );

                        if draw_labels {
                            let font = egui::FontId::proportional(12.0 * self.render.camera.zoom);
                            let label = if is_collapsed {
                                let n = self.simulation.layout_state.children_of(*id).len();
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
                        let color = if self.interaction.selection == Some(*id) {
                            brighten(base, 2.0)
                        } else {
                            base
                        };
                        let r = if circle_mode {
                            let base_r = circle_radius * self.render.camera.zoom;
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
            self.render.fps.tick();
            paint_fps_overlay(ui, response.rect, self.render.fps.fps());
        });
    }

    /// Build a map from child symbol ID -> collapsed parent symbol ID
    /// for edge rerouting.
    fn build_edge_reroute_map(&self) -> HashMap<SymbolId, SymbolId> {
        let mut reroute = HashMap::new();
        for (id, _sym) in &self.graph.graph.symbols {
            if self.simulation.layout_state.is_container(*id)
                && !self.simulation.layout_state.is_expanded(*id)
            {
                for child_id in self.simulation.layout_state.children_of(*id) {
                    reroute.insert(child_id, *id);
                }
            }
        }
        reroute
    }

    /// Hit-test that accounts for expanded container backgrounds.
    fn hit_test_compound(
        &self,
        pointer: Option<egui::Pos2>,
        canvas_center: egui::Pos2,
    ) -> Option<SymbolId> {
        let radius = if self.render.render.circle_mode {
            Some(self.render.render.circle_radius)
        } else {
            None
        };
        if let Some(hit) = crate::camera::hit_test(
            &self.render.camera,
            &self.simulation.positions,
            &self.simulation.node_sizes,
            pointer,
            canvas_center,
            radius,
        ) {
            if !self.filters.hidden_symbols.contains(&hit) {
                return Some(hit);
            }
        }

        let pointer = pointer?;
        for &(id, rect) in self.simulation.container_rects.iter().rev() {
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
