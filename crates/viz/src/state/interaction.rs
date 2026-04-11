//! User interaction state: selection, drag, search.

use core_ir::SymbolId;

/// Transient interaction state shared across panels.
#[derive(Default)]
pub struct InteractionState {
    /// Currently selected node (shown in details panel).
    pub selection: Option<SymbolId>,
    /// Node currently being dragged on the canvas.
    pub dragging: Option<SymbolId>,
    /// Search text entered in the left panel.
    pub search: String,
    /// Whether the initial auto-fit has been performed.
    pub auto_fitted: bool,
    /// Frame counter used to trigger auto-fit after a timeout.
    pub frame_count: u32,
}
