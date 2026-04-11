//! Graph data and file-tree structure.

use core_ir::Graph;

use crate::file_tree::FileTree;

/// Immutable (after indexing) graph data and the derived file tree.
pub struct GraphState {
    /// The symbol/edge graph produced by the frontend.
    pub graph: Graph,
    /// Directory hierarchy built from symbol source locations.
    pub file_tree: FileTree,
}
