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
