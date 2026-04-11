//! Layout simulation state: force-directed engine, positions, pause control.

use core_ir::{Graph, SymbolId, SymbolKind};
use glam::Vec2 as GVec2;
use layout::{LayoutState, Positions};

use crate::camera::{NodeSizes, NODE_HEIGHT, NODE_WIDTH};

/// Size of collapsed container box (must match layout crate's COLLAPSED_HALF_SIZE * 2).
const COLLAPSED_BOX_SIZE: GVec2 = GVec2::new(160.0, 100.0);

/// Padding for top-level containers (namespaces, translation units).
const TOPLEVEL_CONTAINER_PADDING: f32 = 50.0;

/// Padding for class-level containers.
const CLASS_CONTAINER_PADDING: f32 = 15.0;

/// Extra vertical space for the container title bar.
const TITLE_HEIGHT: f32 = 20.0;

/// Force-directed layout simulation state.
pub struct SimulationState {
    /// The incremental force-directed layout engine.
    pub layout_state: LayoutState,
    /// Current node positions (snapshot from layout_state each frame).
    pub positions: Positions,
    /// Per-node sizes for hit-testing and auto-fit (empty = use defaults).
    pub node_sizes: NodeSizes,
    /// Whether the layout simulation is paused.
    pub paused: bool,
    /// Screen-space rects of expanded containers (computed each frame for
    /// hit-testing). Pairs of (container SymbolId, screen Rect).
    pub container_rects: Vec<(SymbolId, egui::Rect)>,
}

impl SimulationState {
    /// Recompute per-node sizes based on collapse state and push them to
    /// the layout engine for size-aware container overlap resolution.
    ///
    /// Expanded containers get their real bounding box (descendant positions
    /// and padding), matching what the canvas renders. This ensures the overlap
    /// resolution knows the true extent of each container.
    pub fn update_node_sizes(&mut self, graph: &Graph) {
        let default_size = GVec2::new(NODE_WIDTH, NODE_HEIGHT);
        let half_node = GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0);
        let collapsed_size = COLLAPSED_BOX_SIZE;

        let positions = self.layout_state.positions();

        let mut sizes: Vec<(SymbolId, GVec2)> = Vec::new();
        for (&id, sym) in &graph.symbols {
            if !self.layout_state.is_container(id) {
                sizes.push((id, default_size));
                continue;
            }
            if !self.layout_state.is_expanded(id) {
                sizes.push((id, collapsed_size));
                continue;
            }
            // Expanded container: compute real bbox from descendant positions.
            let descendants = self.layout_state.all_descendants(id);
            let parent_pos = positions.0.get(&id).copied().unwrap_or(GVec2::ZERO);
            let mut mn = parent_pos - half_node;
            let mut mx = parent_pos + half_node;
            for &desc_id in &descendants {
                if let Some(&pos) = positions.0.get(&desc_id) {
                    mn = mn.min(pos - half_node);
                    mx = mx.max(pos + half_node);
                }
            }
            let pad = match sym.kind {
                SymbolKind::Namespace | SymbolKind::TranslationUnit => TOPLEVEL_CONTAINER_PADDING,
                _ => CLASS_CONTAINER_PADDING,
            };
            mn -= GVec2::new(pad, pad + TITLE_HEIGHT);
            mx += GVec2::new(pad, pad);
            let size = mx - mn;
            sizes.push((id, size));
        }
        self.layout_state.set_sizes(&sizes);
    }
}
