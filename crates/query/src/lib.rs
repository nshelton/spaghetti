//! Graph query functions for spaghetti.
//!
//! Simple queries over [`core_ir::Graph`] — subgraph extraction, name search,
//! and caller lookup. Designed to be callable from the viz UI and (future) MCP server.

use std::collections::{HashSet, VecDeque};

use core_ir::{EdgeKind, Graph, SymbolId};

/// Extract a subgraph rooted at `root`, traversing up to `depth` hops along
/// edges whose kind is in `kinds`. If `kinds` is empty, all edge kinds match.
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

/// Find all symbols that have a `Calls` edge pointing to `id`.
pub fn callers_of(g: &Graph, id: SymbolId) -> Vec<SymbolId> {
    g.edges
        .iter()
        .filter(|e| e.to == id && e.kind == EdgeKind::Calls)
        .map(|e| e.from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_ir::{Edge, Symbol, SymbolKind};

    fn make_symbol(name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            id: SymbolId::from_parts(name, kind),
            kind,
            name: name.split("::").last().unwrap_or(name).to_owned(),
            qualified_name: name.to_owned(),
            location: None,
            module: None,
            attrs: Default::default(),
        }
    }

    fn test_graph() -> Graph {
        let mut g = Graph::new();
        let main_fn = make_symbol("main", SymbolKind::Function);
        let shape = make_symbol("Shape", SymbolKind::Class);
        let circle = make_symbol("Circle", SymbolKind::Class);
        let area = make_symbol("Circle::area", SymbolKind::Method);

        let main_id = main_fn.id;
        let shape_id = shape.id;
        let circle_id = circle.id;
        let area_id = area.id;

        g.add_symbol(main_fn);
        g.add_symbol(shape);
        g.add_symbol(circle);
        g.add_symbol(area);

        g.add_edge(Edge {
            from: circle_id,
            to: shape_id,
            kind: EdgeKind::Inherits,
            location: None,
        });
        g.add_edge(Edge {
            from: main_id,
            to: area_id,
            kind: EdgeKind::Calls,
            location: None,
        });
        g.add_edge(Edge {
            from: area_id,
            to: circle_id,
            kind: EdgeKind::Contains,
            location: None,
        });
        g
    }

    #[test]
    fn test_subgraph_around() {
        let g = test_graph();
        let circle_id = SymbolId::from_parts("Circle", SymbolKind::Class);

        // Depth 1, all kinds — should get Circle + its direct neighbors
        let sub = subgraph_around(&g, circle_id, 1, &[]);
        assert!(sub.symbols.contains_key(&circle_id));
        // Shape (via Inherits) and area (via Contains) should be included
        assert_eq!(sub.symbol_count(), 3);
    }

    #[test]
    fn test_subgraph_depth_zero() {
        let g = test_graph();
        let circle_id = SymbolId::from_parts("Circle", SymbolKind::Class);

        let sub = subgraph_around(&g, circle_id, 0, &[]);
        assert_eq!(sub.symbol_count(), 1);
        assert_eq!(sub.edge_count(), 0);
    }

    #[test]
    fn test_find_by_name() {
        let g = test_graph();
        let results = find_by_name(&g, "circle");
        // Should match "Circle" and "Circle::area"
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_callers_of() {
        let g = test_graph();
        let area_id = SymbolId::from_parts("Circle::area", SymbolKind::Method);
        let callers = callers_of(&g, area_id);
        assert_eq!(callers.len(), 1);
        assert_eq!(
            callers[0],
            SymbolId::from_parts("main", SymbolKind::Function)
        );
    }

    #[test]
    fn test_callers_of_none() {
        let g = test_graph();
        let shape_id = SymbolId::from_parts("Shape", SymbolKind::Class);
        let callers = callers_of(&g, shape_id);
        assert!(callers.is_empty());
    }
}
