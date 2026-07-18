//! Graph data and file-tree structure.

use core_ir::Graph;

use crate::file_tree::FileTree;

/// Immutable (after indexing) graph data and the derived file tree.
pub struct GraphState {
    /// The graph currently being displayed (transformed when split mode is on).
    pub graph: Graph,
    /// The untransformed graph as produced by the frontends.
    pub base: Graph,
    /// Whether shared field types are duplicated per containing struct.
    pub split_shared_types: bool,
    /// Directory hierarchy built from symbol source locations.
    pub file_tree: FileTree,
}

impl GraphState {
    /// Recompute the displayed graph from the base graph and the split mode.
    pub fn rebuild_display(&mut self) {
        self.graph = if self.split_shared_types {
            query::split_shared_types(&self.base)
        } else {
            self.base.clone()
        };
    }
}
