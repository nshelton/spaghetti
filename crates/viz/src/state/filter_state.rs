//! Visibility filtering: edge kinds, node kinds, hidden symbols, edgeless filter.

use std::collections::{HashMap, HashSet};

use core_ir::{EdgeKind, SymbolId, SymbolKind};

use super::graph_state::GraphState;
use super::simulation::SimulationState;

/// All symbol kinds in the system.
pub const ALL_SYMBOL_KINDS: [SymbolKind; 8] = [
    SymbolKind::Class,
    SymbolKind::Struct,
    SymbolKind::Function,
    SymbolKind::Method,
    SymbolKind::Field,
    SymbolKind::Namespace,
    SymbolKind::TemplateInstantiation,
    SymbolKind::TranslationUnit,
];

/// All edge kinds in the system.
pub const ALL_EDGE_KINDS: [EdgeKind; 9] = [
    EdgeKind::Calls,
    EdgeKind::Inherits,
    EdgeKind::Contains,
    EdgeKind::Overrides,
    EdgeKind::ReadsField,
    EdgeKind::WritesField,
    EdgeKind::Includes,
    EdgeKind::Instantiates,
    EdgeKind::HasType,
];

/// Edge kind filter state — tracks which edge kinds are visible.
pub struct EdgeKindFilter {
    enabled: HashSet<EdgeKind>,
}

impl Default for EdgeKindFilter {
    fn default() -> Self {
        Self {
            enabled: ALL_EDGE_KINDS.iter().copied().collect(),
        }
    }
}

impl EdgeKindFilter {
    /// Restore from saved edge filter map. Missing keys default to enabled.
    pub fn from_saved(saved: &HashMap<String, bool>) -> Self {
        let mut enabled = HashSet::new();
        for &kind in &ALL_EDGE_KINDS {
            let key = format!("{kind:?}");
            let is_enabled = saved.get(&key).copied().unwrap_or(true);
            if is_enabled {
                enabled.insert(kind);
            }
        }
        Self { enabled }
    }

    /// Export current state as a string-keyed map for serialization.
    pub fn to_saved(&self) -> HashMap<String, bool> {
        ALL_EDGE_KINDS
            .iter()
            .map(|&kind| (format!("{kind:?}"), self.enabled.contains(&kind)))
            .collect()
    }

    /// Returns the list of currently active edge kinds.
    pub fn active_kinds(&self) -> Vec<EdgeKind> {
        self.enabled.iter().copied().collect()
    }

    /// Check whether a specific edge kind is enabled.
    pub fn is_enabled(&self, kind: EdgeKind) -> bool {
        self.enabled.contains(&kind)
    }

    /// Toggle a specific edge kind on or off.
    pub fn toggle(&mut self, kind: EdgeKind) {
        if self.enabled.contains(&kind) {
            self.enabled.remove(&kind);
        } else {
            self.enabled.insert(kind);
        }
    }
}

/// Symbol kind filter state — tracks which node kinds are visible.
pub struct SymbolKindFilter {
    enabled: HashSet<SymbolKind>,
}

impl Default for SymbolKindFilter {
    fn default() -> Self {
        Self {
            enabled: ALL_SYMBOL_KINDS.iter().copied().collect(),
        }
    }
}

impl SymbolKindFilter {
    /// Restore from saved node filter map. Missing keys default to enabled.
    pub fn from_saved(saved: &HashMap<String, bool>) -> Self {
        let mut enabled = HashSet::new();
        for &kind in &ALL_SYMBOL_KINDS {
            let key = format!("{kind:?}");
            let is_enabled = saved.get(&key).copied().unwrap_or(true);
            if is_enabled {
                enabled.insert(kind);
            }
        }
        Self { enabled }
    }

    /// Export current state as a string-keyed map for serialization.
    pub fn to_saved(&self) -> HashMap<String, bool> {
        ALL_SYMBOL_KINDS
            .iter()
            .map(|&kind| (format!("{kind:?}"), self.enabled.contains(&kind)))
            .collect()
    }

    /// Check whether a specific symbol kind is enabled.
    pub fn is_enabled(&self, kind: SymbolKind) -> bool {
        self.enabled.contains(&kind)
    }

    /// Toggle a specific symbol kind on or off.
    pub fn toggle(&mut self, kind: SymbolKind) {
        if self.enabled.contains(&kind) {
            self.enabled.remove(&kind);
        } else {
            self.enabled.insert(kind);
        }
    }
}

/// Combined visibility/filter state.
#[derive(Default)]
pub struct FilterState {
    /// Which edge kinds are currently visible.
    pub edge_filter: EdgeKindFilter,
    /// Which symbol kinds are currently visible.
    pub node_filter: SymbolKindFilter,
    /// Symbols currently hidden by file-tree visibility toggles + collapse.
    pub hidden_symbols: HashSet<SymbolId>,
    /// Whether to hide nodes that have no visible edges.
    pub hide_edgeless: bool,
    /// Saved directory visibility (applied after indexing completes).
    pub pending_dir_visibility: HashMap<String, bool>,
}

impl FilterState {
    /// Recompute the combined hidden set (file-tree visibility + node-kind
    /// filter + edgeless filter) and push it to the layout engine.
    pub fn sync_hidden_to_layout(&self, graph: &GraphState, sim: &mut SimulationState) {
        let mut hidden: Vec<SymbolId> = self.hidden_symbols.iter().copied().collect();

        // Collect nodes that have at least one *visible* edge (if the edgeless
        // filter is on). An edge is visible when its kind is enabled and both
        // endpoints have visible node kinds and aren't file-tree-hidden.
        let nodes_with_edges: Option<HashSet<SymbolId>> = if self.hide_edgeless {
            let active_kinds = self.edge_filter.active_kinds();
            let mut set = HashSet::new();
            for edge in &graph.graph.edges {
                if !active_kinds.contains(&edge.kind) {
                    continue;
                }
                let from_ok = !self.hidden_symbols.contains(&edge.from)
                    && graph
                        .graph
                        .symbols
                        .get(&edge.from)
                        .is_some_and(|s| self.node_filter.is_enabled(s.kind));
                let to_ok = !self.hidden_symbols.contains(&edge.to)
                    && graph
                        .graph
                        .symbols
                        .get(&edge.to)
                        .is_some_and(|s| self.node_filter.is_enabled(s.kind));
                if from_ok && to_ok {
                    set.insert(edge.from);
                    set.insert(edge.to);
                }
            }
            Some(set)
        } else {
            None
        };

        for (id, sym) in &graph.graph.symbols {
            if self.hidden_symbols.contains(id) {
                continue;
            }
            if !self.node_filter.is_enabled(sym.kind) {
                hidden.push(*id);
                continue;
            }
            if let Some(ref with_edges) = nodes_with_edges {
                if !with_edges.contains(id) {
                    hidden.push(*id);
                }
            }
        }
        sim.layout_state.set_hidden(&hidden);
    }

    /// Recompute the effective hidden-symbols set from file-tree visibility.
    /// Collapse no longer hides children (they stay visible inside the box).
    pub fn sync_hidden_symbols(&mut self, graph: &GraphState, sim: &mut SimulationState) {
        let file_hidden = graph.file_tree.hidden_symbols();
        self.hidden_symbols = file_hidden.clone();
        let hidden_vec: Vec<_> = file_hidden.into_iter().collect();
        sim.layout_state.set_hidden(&hidden_vec);
        sim.update_node_sizes(&graph.graph);
    }
}
