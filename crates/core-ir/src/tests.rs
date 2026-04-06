//! Tests for core-ir types and graph operations.

use super::*;

fn make_symbol(name: &str, kind: SymbolKind) -> Symbol {
    let id = SymbolId::from_parts(name, kind);
    Symbol {
        id,
        kind,
        name: name.split("::").last().unwrap_or(name).to_owned(),
        qualified_name: name.to_owned(),
        location: None,
        module: None,
        attrs: Default::default(),
    }
}

#[test]
fn test_symbol_id_determinism() {
    let a = SymbolId::from_parts("Foo::bar", SymbolKind::Method);
    let b = SymbolId::from_parts("Foo::bar", SymbolKind::Method);
    assert_eq!(a, b);

    // Different kind → different ID
    let c = SymbolId::from_parts("Foo::bar", SymbolKind::Field);
    assert_ne!(a, c);
}

#[test]
fn test_file_table_interning() {
    let mut ft = FileTable::default();
    let a = ft.intern("src/main.cpp");
    let b = ft.intern("src/foo.cpp");
    let a2 = ft.intern("src/main.cpp");
    assert_eq!(a, a2);
    assert_ne!(a, b);
    assert_eq!(ft.resolve(a), Some("src/main.cpp"));
    assert_eq!(ft.len(), 2);
}

#[test]
fn test_graph_add_and_neighbors() {
    let mut g = Graph::new();
    let shape = make_symbol("Shape", SymbolKind::Class);
    let circle = make_symbol("Circle", SymbolKind::Class);
    let area = make_symbol("Circle::area", SymbolKind::Method);

    let shape_id = g.add_symbol(shape);
    let circle_id = g.add_symbol(circle);
    let area_id = g.add_symbol(area);

    g.add_edge(Edge {
        from: circle_id,
        to: shape_id,
        kind: EdgeKind::Inherits,
        location: None,
    });
    g.add_edge(Edge {
        from: area_id,
        to: circle_id,
        kind: EdgeKind::Contains,
        location: None,
    });

    assert_eq!(g.symbol_count(), 3);
    assert_eq!(g.edge_count(), 2);

    // Neighbors with filter
    let inherits: Vec<_> = g.neighbors(circle_id, &[EdgeKind::Inherits]).collect();
    assert_eq!(inherits, vec![shape_id]);

    // Neighbors without filter (all)
    let all: Vec<_> = g.neighbors(circle_id, &[]).collect();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_merge_determinism() {
    let mut g1 = Graph::new();
    let mut g2 = Graph::new();

    let shape = make_symbol("Shape", SymbolKind::Class);
    let circle = make_symbol("Circle", SymbolKind::Class);

    let shape_id = g1.add_symbol(shape.clone());
    g2.add_symbol(circle.clone());

    g2.add_edge(Edge {
        from: circle.id,
        to: shape_id,
        kind: EdgeKind::Inherits,
        location: None,
    });

    g1.merge(g2);
    assert_eq!(g1.symbol_count(), 2);
    assert_eq!(g1.edge_count(), 1);

    // Merge again — same result (idempotent for symbols, edges append)
    let mut g3 = Graph::new();
    g3.add_symbol(circle);
    g1.merge(g3);
    assert_eq!(g1.symbol_count(), 2); // overwrite, not duplicate
}

#[test]
fn test_serde_roundtrip() {
    let mut g = Graph::new();
    let fid = g.files.intern("src/main.cpp");

    let sym = Symbol {
        id: SymbolId::from_parts("main", SymbolKind::Function),
        kind: SymbolKind::Function,
        name: "main".to_owned(),
        qualified_name: "main".to_owned(),
        location: Some(Location {
            file: fid,
            line: 1,
            col: 1,
        }),
        module: None,
        attrs: Default::default(),
    };
    g.add_symbol(sym);

    let json = g.to_json().expect("serialize");
    let g2 = Graph::from_json(&json).expect("deserialize");

    assert_eq!(g2.symbol_count(), 1);
    assert_eq!(g2.files.resolve(FileId(0)), Some("src/main.cpp"));
}
