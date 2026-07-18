//! Graph query functions for spaghetti.
//!
//! Simple queries over [`core_ir::Graph`] — subgraph extraction, name search,
//! and caller lookup. Designed to be callable from the viz UI and (future) MCP server.

use std::collections::{HashMap, HashSet, VecDeque};

use core_ir::{Edge, EdgeKind, Graph, Symbol, SymbolId};

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

/// Duplicate shared field types into per-container clones ("split shared
/// types" mode).
///
/// Shared leaf types (`vec3`, `string`, …) referenced by fields of many
/// structs become high-fan-in hub nodes that wreck force layouts. This
/// transform splits them: for every type `T` that receives
/// [`EdgeKind::HasType`] edges from fields of **two or more** distinct
/// containers, each container `C` gets its own clone of `T`:
///
/// * The clone's [`SymbolId`] derives from `"<T>@<C>"` (qualified names) —
///   deterministic across runs and collision-free, since `@` cannot appear
///   in a C++ qualified name.
/// * The clone inherits `C`'s location, so file-tree filtering and
///   directory-affinity forces keep it with the container, not with the
///   type's definition file.
/// * `HasType` edges from `C`'s fields are retargeted to the clone, and a
///   `Contains` edge `C → clone` nests the clone inside the container.
///
/// The original type node keeps its definition subgraph (members,
/// inheritance, …) but loses the retargeted fan-in. Types referenced by a
/// single container, and `HasType` edges from symbols with no container,
/// pass through unchanged.
pub fn split_shared_types(g: &Graph) -> Graph {
    // symbol → its container, from Contains edges (first parent wins).
    let mut container_of: HashMap<SymbolId, SymbolId> = HashMap::new();
    for e in &g.edges {
        if e.kind == EdgeKind::Contains {
            container_of.entry(e.to).or_insert(e.from);
        }
    }

    // type → distinct containers whose fields reference it via HasType.
    let mut containers_by_type: HashMap<SymbolId, HashSet<SymbolId>> = HashMap::new();
    for e in &g.edges {
        if e.kind == EdgeKind::HasType {
            if let Some(&c) = container_of.get(&e.from) {
                containers_by_type.entry(e.to).or_default().insert(c);
            }
        }
    }

    let mut result = Graph::new();
    result.files = g.files.clone();
    for sym in g.symbols.values() {
        result.add_symbol(sym.clone());
    }

    // Create clones in edge order so output is deterministic.
    let mut clones: HashMap<(SymbolId, SymbolId), SymbolId> = HashMap::new();
    for e in &g.edges {
        if e.kind != EdgeKind::HasType {
            continue;
        }
        let Some(&c) = container_of.get(&e.from) else {
            continue;
        };
        if containers_by_type.get(&e.to).is_none_or(|s| s.len() < 2) {
            continue;
        }
        if clones.contains_key(&(e.to, c)) {
            continue;
        }
        let (Some(ty), Some(container)) = (g.symbols.get(&e.to), g.symbols.get(&c)) else {
            continue;
        };
        let qualified = format!("{}@{}", ty.qualified_name, container.qualified_name);
        let clone_id = SymbolId::from_parts(&qualified, ty.kind);
        result.add_symbol(Symbol {
            id: clone_id,
            kind: ty.kind,
            name: ty.name.clone(),
            qualified_name: qualified,
            location: container.location,
            module: ty.module.clone(),
            attrs: ty.attrs.clone(),
        });
        result.add_edge(Edge {
            from: c,
            to: clone_id,
            kind: EdgeKind::Contains,
            location: None,
        });
        clones.insert((e.to, c), clone_id);
    }

    // Copy edges, retargeting split HasType fan-in to the clones.
    for e in &g.edges {
        let mut edge = e.clone();
        if edge.kind == EdgeKind::HasType {
            if let Some(&c) = container_of.get(&edge.from) {
                if let Some(&clone_id) = clones.get(&(edge.to, c)) {
                    edge.to = clone_id;
                }
            }
        }
        result.add_edge(edge);
    }

    result
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
