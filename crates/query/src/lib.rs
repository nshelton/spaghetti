//! Graph query functions for spaghetti.
//!
//! Simple queries over [`core_ir::Graph`] — subgraph extraction, name search,
//! and caller lookup. Designed to be callable from the viz UI and (future) MCP server.

use std::collections::{HashSet, VecDeque};

use core_ir::{EdgeKind, Graph, SymbolId};

/// Extract a subgraph rooted at `root`, traversing up to `depth` hops along
/// edges whose kind is in `kinds`. If `kinds` is empty, all edge kinds match.
///
/// # Edge cases
///
/// * `depth = 0` — returns only the root node with no edges.
/// * `depth = 1` — returns the root plus its immediate neighbors via matching
///   edge kinds.
/// * Empty `kinds` slice — follows all edge kinds (no filtering).
/// * Root not in graph — returns an empty graph (no panic).
/// * Traversal is **bidirectional**: an edge `A → B` lets BFS reach `A` from
///   `B` and vice-versa. A leaf node at depth 1 will therefore include its
///   predecessor.
/// * The returned graph contains only edges where **both** endpoints are in the
///   visited set **and** the edge kind matches the filter.
pub fn subgraph_around(g: &Graph, root: SymbolId, depth: u32, kinds: &[EdgeKind]) -> Graph {
    let mut visited: HashSet<SymbolId> = HashSet::new();
    let mut queue: VecDeque<(SymbolId, u32)> = VecDeque::new();
    queue.push_back((root, 0));
    visited.insert(root);

    while let Some((current, d)) = queue.pop_front() {
        if d >= depth {
            continue;
        }
        for neighbor in g.neighbors(current, kinds) {
            if visited.insert(neighbor) {
                queue.push_back((neighbor, d + 1));
            }
        }
    }

    let mut result = Graph::new();
    result.files = g.files.clone();

    for &id in &visited {
        if let Some(sym) = g.symbols.get(&id) {
            result.add_symbol(sym.clone());
        }
    }

    for edge in &g.edges {
        if visited.contains(&edge.from)
            && visited.contains(&edge.to)
            && (kinds.is_empty() || kinds.contains(&edge.kind))
        {
            result.add_edge(edge.clone());
        }
    }

    result
}

/// Find symbols whose `name` or `qualified_name` contains `pattern`
/// (case-insensitive).
pub fn find_by_name(g: &Graph, pattern: &str) -> Vec<SymbolId> {
    let pattern_lower = pattern.to_lowercase();
    g.symbols
        .values()
        .filter(|sym| {
            sym.name.to_lowercase().contains(&pattern_lower)
                || sym.qualified_name.to_lowercase().contains(&pattern_lower)
        })
        .map(|sym| sym.id)
        .collect()
}

/// Find all symbols that have a `Calls` edge pointing **to** `id`.
///
/// # Direction semantics
///
/// Only **incoming** `Calls` edges are considered: an edge `A → B` with kind
/// `Calls` means A is a caller of B. This function returns the `from` side of
/// such edges. Outgoing edges (callees of `id`) are excluded.
///
/// Non-call edge kinds (`Inherits`, `Contains`, etc.) are ignored entirely.
///
/// If `id` does not exist in the graph or nothing calls it, an empty `Vec` is
/// returned — the function never panics on an unknown ID.
///
/// If `id` calls itself (self-loop), it appears in the result.
pub fn callers_of(g: &Graph, id: SymbolId) -> Vec<SymbolId> {
    g.edges
        .iter()
        .filter(|e| e.to == id && e.kind == EdgeKind::Calls)
        .map(|e| e.from)
        .collect()
}
