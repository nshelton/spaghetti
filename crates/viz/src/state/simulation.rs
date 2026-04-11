//! Layout simulation state: force-directed engine, positions, pause control.

use core_ir::{Graph, SymbolId};
use glam::Vec2 as GVec2;
use layout::{LayoutState, Positions};

use crate::camera::{NodeSizes, NODE_HEIGHT, NODE_WIDTH};

/// Size of collapsed container box (must match layout crate's COLLAPSED_HALF_SIZE * 2).
const COLLAPSED_BOX_SIZE: GVec2 = GVec2::new(160.0, 100.0);

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
    /// the layout engine for size-aware repulsion.
    pub fn update_node_sizes(&mut self, graph: &Graph) {
        let default_size = GVec2::new(NODE_WIDTH, NODE_HEIGHT);
        let collapsed_size = COLLAPSED_BOX_SIZE;

        let mut sizes: Vec<(SymbolId, GVec2)> = Vec::new();
        for &id in graph.symbols.keys() {
            let is_collapsed =
                self.layout_state.is_container(id) && !self.layout_state.is_expanded(id);
            let size = if is_collapsed {
                collapsed_size
            } else {
                default_size
            };
            sizes.push((id, size));
        }
        self.layout_state.set_sizes(&sizes);
    }
}
