//! Layout simulation state: force-directed engine, positions, pause control.

use core_ir::SymbolId;
use layout::{LayoutState, Positions};

use crate::camera::NodeSizes;

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
