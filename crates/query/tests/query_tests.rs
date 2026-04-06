//! Tests for query functions.

use core_ir::{Edge, EdgeKind, Graph, Symbol, SymbolId, SymbolKind};
use query::{callers_of, find_by_name, subgraph_around};

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

// ---------------------------------------------------------------------------
// CAP-007: 7-node reference graph for subgraph_around tests
// ---------------------------------------------------------------------------
//
//   A --Calls--> B --Calls--> C --Calls--> D
//   A --Inherits--> E
//   B --Contains--> F
//   C --Inherits--> G
//

fn cap007_graph() -> (
    Graph,
    SymbolId,
    SymbolId,
    SymbolId,
    SymbolId,
    SymbolId,
    SymbolId,
    SymbolId,
) {
    let mut g = Graph::new();
    let a = make_symbol("A", SymbolKind::Class);
    let b = make_symbol("B", SymbolKind::Class);
    let c = make_symbol("C", SymbolKind::Class);
    let d = make_symbol("D", SymbolKind::Class);
    let e = make_symbol("E", SymbolKind::Class);
    let f = make_symbol("F", SymbolKind::Class);
    let node_g = make_symbol("G", SymbolKind::Class);

    let (a_id, b_id, c_id, d_id, e_id, f_id, g_id) =
        (a.id, b.id, c.id, d.id, e.id, f.id, node_g.id);

    g.add_symbol(a);
    g.add_symbol(b);
    g.add_symbol(c);
    g.add_symbol(d);
    g.add_symbol(e);
    g.add_symbol(f);
    g.add_symbol(node_g);

    g.add_edge(Edge {
        from: a_id,
        to: b_id,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: b_id,
        to: c_id,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: c_id,
        to: d_id,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: a_id,
        to: e_id,
        kind: EdgeKind::Inherits,
        location: None,
    });
    g.add_edge(Edge {
        from: b_id,
        to: f_id,
        kind: EdgeKind::Contains,
        location: None,
    });
    g.add_edge(Edge {
        from: c_id,
        to: g_id,
        kind: EdgeKind::Inherits,
        location: None,
    });

    (g, a_id, b_id, c_id, d_id, e_id, f_id, g_id)
}

/// CAP-007 test 1: depth 0 returns only root, no edges.
#[test]
fn test_cap007_depth_zero() {
    let (g, a_id, ..) = cap007_graph();
    let sub = subgraph_around(&g, a_id, 0, &[]);
    assert_eq!(sub.symbol_count(), 1);
    assert!(sub.symbols.contains_key(&a_id));
    assert_eq!(sub.edge_count(), 0);
}

/// CAP-007 test 2: depth 1 from A returns {A, B, E} with edges {A→B, A→E}.
#[test]
fn test_cap007_depth_one() {
    let (g, a_id, b_id, _, _, e_id, ..) = cap007_graph();
    let sub = subgraph_around(&g, a_id, 1, &[]);
    assert_eq!(sub.symbol_count(), 3);
    assert!(sub.symbols.contains_key(&a_id));
    assert!(sub.symbols.contains_key(&b_id));
    assert!(sub.symbols.contains_key(&e_id));
    assert_eq!(sub.edge_count(), 2);
}

/// CAP-007 test 3: depth 2 from A returns {A, B, E, C, F} with correct edges.
#[test]
fn test_cap007_depth_two() {
    let (g, a_id, b_id, c_id, _, e_id, f_id, _) = cap007_graph();
    let sub = subgraph_around(&g, a_id, 2, &[]);
    assert_eq!(sub.symbol_count(), 5);
    assert!(sub.symbols.contains_key(&a_id));
    assert!(sub.symbols.contains_key(&b_id));
    assert!(sub.symbols.contains_key(&c_id));
    assert!(sub.symbols.contains_key(&e_id));
    assert!(sub.symbols.contains_key(&f_id));
    // Edges: A→B (Calls), A→E (Inherits), B→C (Calls), B→F (Contains)
    assert_eq!(sub.edge_count(), 4);
}

/// CAP-007 test 4: depth 2 from A with Calls filter returns {A, B, C}, only Calls edges.
#[test]
fn test_cap007_kind_filter_calls() {
    let (g, a_id, b_id, c_id, ..) = cap007_graph();
    let sub = subgraph_around(&g, a_id, 2, &[EdgeKind::Calls]);
    assert_eq!(sub.symbol_count(), 3);
    assert!(sub.symbols.contains_key(&a_id));
    assert!(sub.symbols.contains_key(&b_id));
    assert!(sub.symbols.contains_key(&c_id));
    assert_eq!(sub.edge_count(), 2);
    for edge in &sub.edges {
        assert_eq!(edge.kind, EdgeKind::Calls);
    }
}

/// CAP-007 test 5: root not in graph returns empty graph, no panic.
#[test]
fn test_cap007_root_not_found() {
    let (g, ..) = cap007_graph();
    let missing = SymbolId::from_parts("DoesNotExist", SymbolKind::Class);
    let sub = subgraph_around(&g, missing, 3, &[]);
    assert_eq!(sub.symbol_count(), 0);
    assert_eq!(sub.edge_count(), 0);
}

/// CAP-007 test 6: leaf node D at depth 1 reaches C (bidirectional traversal).
#[test]
fn test_cap007_leaf_as_root() {
    let (g, _, _, c_id, d_id, ..) = cap007_graph();
    let sub = subgraph_around(&g, d_id, 1, &[]);
    // D's only neighbor is C (via the C→D Calls edge, traversed bidirectionally).
    assert_eq!(sub.symbol_count(), 2);
    assert!(sub.symbols.contains_key(&d_id));
    assert!(sub.symbols.contains_key(&c_id));
    assert_eq!(sub.edge_count(), 1);
}

/// CAP-007 test 7: edge integrity — no edge references a symbol outside the subgraph.
#[test]
fn test_cap007_edge_integrity() {
    let (g, a_id, ..) = cap007_graph();
    // Test at several depths to be thorough.
    for depth in 0..=4 {
        let sub = subgraph_around(&g, a_id, depth, &[]);
        for edge in &sub.edges {
            assert!(
                sub.symbols.contains_key(&edge.from),
                "depth {depth}: edge.from {:?} not in subgraph symbols",
                edge.from
            );
            assert!(
                sub.symbols.contains_key(&edge.to),
                "depth {depth}: edge.to {:?} not in subgraph symbols",
                edge.to
            );
        }
    }
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
