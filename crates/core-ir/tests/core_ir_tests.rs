//! Tests for core-ir types and graph operations.

use std::collections::{HashMap, HashSet};

use core_ir::*;

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
fn test_symbol_id_determinism_1000_iterations() {
    let reference = SymbolId::from_parts("Foo::bar", SymbolKind::Method);
    for _ in 0..1000 {
        assert_eq!(
            SymbolId::from_parts("Foo::bar", SymbolKind::Method),
            reference
        );
    }
}

#[test]
fn test_symbol_id_golden_file() {
    let golden_json =
        std::fs::read_to_string("tests/fixtures/symbol_id_golden.json").expect("golden file");
    let golden: HashMap<String, u64> =
        serde_json::from_str(&golden_json).expect("parse golden JSON");

    let test_name = "TestSymbol";
    let kinds: &[(&str, SymbolKind)] = &[
        ("Class", SymbolKind::Class),
        ("Struct", SymbolKind::Struct),
        ("Function", SymbolKind::Function),
        ("Method", SymbolKind::Method),
        ("Field", SymbolKind::Field),
        ("Namespace", SymbolKind::Namespace),
        ("TemplateInstantiation", SymbolKind::TemplateInstantiation),
        ("TranslationUnit", SymbolKind::TranslationUnit),
    ];

    for (kind_name, kind) in kinds {
        let key = format!("{test_name}|{kind_name}");
        let expected = golden
            .get(&key)
            .unwrap_or_else(|| panic!("missing golden entry for {key}"));
        let actual = SymbolId::from_parts(test_name, *kind);
        assert_eq!(
            actual.0, *expected,
            "golden mismatch for {key}: got {}, expected {expected}",
            actual.0
        );
    }
}

#[test]
fn test_symbol_id_no_collisions_tiny_cpp() {
    let json =
        std::fs::read_to_string("../../examples/tiny-cpp/graph.json").expect("tiny-cpp graph.json");
    let graph = Graph::from_json(&json).expect("parse graph");

    let ids: Vec<SymbolId> = graph.symbols.keys().copied().collect();
    let unique: HashSet<SymbolId> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "collision detected among {} symbols",
        ids.len()
    );
}

#[test]
fn test_symbol_id_whitespace_normalization() {
    let canonical = SymbolId::from_parts("Foo::bar", SymbolKind::Method);

    // Leading/trailing whitespace is trimmed
    assert_eq!(
        SymbolId::from_parts("  Foo::bar  ", SymbolKind::Method),
        canonical
    );
    assert_eq!(
        SymbolId::from_parts("\tFoo::bar\n", SymbolKind::Method),
        canonical
    );

    // Interior whitespace variations collapse to the same result
    let spaced = SymbolId::from_parts("Foo :: bar", SymbolKind::Method);
    assert_eq!(
        SymbolId::from_parts("Foo ::  bar", SymbolKind::Method),
        spaced
    );
    assert_eq!(
        SymbolId::from_parts("Foo  ::  bar", SymbolKind::Method),
        spaced
    );

    // But different names still differ
    assert_ne!(
        SymbolId::from_parts("Foo::baz", SymbolKind::Method),
        canonical
    );
}

#[test]
fn test_symbol_id_different_kinds_differ() {
    let name = "SomeName";
    let all_kinds = [
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Field,
        SymbolKind::Namespace,
        SymbolKind::TemplateInstantiation,
        SymbolKind::TranslationUnit,
    ];
    let ids: Vec<u64> = all_kinds
        .iter()
        .map(|k| SymbolId::from_parts(name, *k).0)
        .collect();
    let unique: HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "different kinds must produce different IDs for the same name"
    );
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

// ---------------------------------------------------------------------------
// Graph::merge() tests — CAP-003
// ---------------------------------------------------------------------------

fn make_symbol_with_location(name: &str, kind: SymbolKind, file: FileId) -> Symbol {
    let id = SymbolId::from_parts(name, kind);
    Symbol {
        id,
        kind,
        name: name.split("::").last().unwrap_or(name).to_owned(),
        qualified_name: name.to_owned(),
        location: Some(Location {
            file,
            line: 1,
            col: 1,
        }),
        module: None,
        attrs: Default::default(),
    }
}

fn make_symbol_with_attrs(name: &str, kind: SymbolKind, attrs: &[Attr]) -> Symbol {
    let id = SymbolId::from_parts(name, kind);
    Symbol {
        id,
        kind,
        name: name.split("::").last().unwrap_or(name).to_owned(),
        qualified_name: name.to_owned(),
        location: None,
        module: None,
        attrs: attrs.iter().cloned().collect(),
    }
}

/// Non-conflicting merge: disjoint symbols and edges are combined.
#[test]
fn test_merge_non_conflicting() {
    let mut g1 = Graph::new();
    let mut g2 = Graph::new();

    let shape = make_symbol("Shape", SymbolKind::Class);
    let circle = make_symbol("Circle", SymbolKind::Class);
    let square = make_symbol("Square", SymbolKind::Class);

    let shape_id = g1.add_symbol(shape);
    let circle_id = g1.add_symbol(circle);
    g1.add_edge(Edge {
        from: circle_id,
        to: shape_id,
        kind: EdgeKind::Inherits,
        location: None,
    });

    let shape2 = make_symbol("Shape", SymbolKind::Class);
    let square_id = g2.add_symbol(square);
    g2.add_symbol(shape2);
    g2.add_edge(Edge {
        from: square_id,
        to: shape_id,
        kind: EdgeKind::Inherits,
        location: None,
    });

    g1.merge(g2);
    assert_eq!(g1.symbol_count(), 3, "should have Shape, Circle, Square");
    assert_eq!(g1.edge_count(), 2, "should have both inheritance edges");
}

/// Symbol conflict: last wins — incoming symbol replaces existing.
#[test]
fn test_merge_symbol_conflict_last_wins() {
    let mut g1 = Graph::new();
    let mut g2 = Graph::new();

    let shape_v1 = make_symbol_with_attrs("Shape", SymbolKind::Class, &[Attr::Abstract]);
    let shape_v2 = make_symbol_with_attrs("Shape", SymbolKind::Class, &[Attr::Virtual]);

    assert_eq!(
        shape_v1.id, shape_v2.id,
        "same qualified_name+kind → same ID"
    );

    g1.add_symbol(shape_v1);
    g2.add_symbol(shape_v2.clone());

    g1.merge(g2);
    assert_eq!(g1.symbol_count(), 1);

    let merged = g1.symbols.get(&shape_v2.id).expect("symbol present");
    assert_eq!(
        merged.attrs.as_slice(),
        &[Attr::Virtual],
        "last-wins: incoming attrs replace existing"
    );
}

/// Edge deduplication: duplicate (from, to, kind) edges are not added twice.
#[test]
fn test_merge_edge_deduplication() {
    let mut g1 = Graph::new();
    let mut g2 = Graph::new();

    let circle = make_symbol("Circle", SymbolKind::Class);
    let shape = make_symbol("Shape", SymbolKind::Class);

    g1.add_symbol(circle.clone());
    g1.add_symbol(shape.clone());
    g1.add_edge(Edge {
        from: circle.id,
        to: shape.id,
        kind: EdgeKind::Inherits,
        location: None,
    });

    // g2 has the same structural edge (different location).
    g2.add_edge(Edge {
        from: circle.id,
        to: shape.id,
        kind: EdgeKind::Inherits,
        location: Some(Location {
            file: FileId(0),
            line: 42,
            col: 1,
        }),
    });

    g1.merge(g2);
    assert_eq!(
        g1.edge_count(),
        1,
        "duplicate edge (same from/to/kind) should be deduplicated"
    );

    // A different kind between the same nodes is NOT a duplicate.
    let mut g3 = Graph::new();
    g3.add_edge(Edge {
        from: circle.id,
        to: shape.id,
        kind: EdgeKind::Contains,
        location: None,
    });
    g1.merge(g3);
    assert_eq!(
        g1.edge_count(),
        2,
        "different EdgeKind should be kept as a separate edge"
    );
}

/// FileTable remapping: file IDs from the incoming graph are rewritten to
/// match the target graph's interning table.
#[test]
fn test_merge_file_table_remapping() {
    let mut g1 = Graph::new();
    let mut g2 = Graph::new();

    let f1 = g1.files.intern("src/main.cpp");
    let _f2 = g1.files.intern("src/shape.cpp");

    // g2 interns different files — IDs will differ from g1.
    let g2_f0 = g2.files.intern("src/circle.cpp"); // FileId(0) in g2
    let g2_f1 = g2.files.intern("src/main.cpp"); // FileId(1) in g2, but FileId(0) in g1

    let circle = make_symbol_with_location("Circle", SymbolKind::Class, g2_f0);
    let main_fn = make_symbol_with_location("main", SymbolKind::Function, g2_f1);
    g2.add_symbol(circle.clone());
    g2.add_symbol(main_fn.clone());
    g2.add_edge(Edge {
        from: main_fn.id,
        to: circle.id,
        kind: EdgeKind::Calls,
        location: Some(Location {
            file: g2_f1,
            line: 10,
            col: 5,
        }),
    });

    g1.merge(g2);

    // "src/circle.cpp" should have been interned into g1's table.
    let circle_sym = g1.symbols.get(&circle.id).expect("circle present");
    let circle_file = circle_sym.location.as_ref().expect("has location").file;
    assert_eq!(
        g1.files.resolve(circle_file),
        Some("src/circle.cpp"),
        "circle file path remapped correctly"
    );

    // "src/main.cpp" should resolve to the SAME FileId that g1 already had.
    let main_sym = g1.symbols.get(&main_fn.id).expect("main present");
    let main_file = main_sym.location.as_ref().expect("has location").file;
    assert_eq!(main_file, f1, "shared path reuses existing FileId");

    // Edge location should also be remapped.
    let edge = &g1.edges[0];
    let edge_file = edge.location.as_ref().expect("has location").file;
    assert_eq!(edge_file, f1, "edge location file remapped to g1's FileId");
}

/// Merge order consistency: A.merge(B) and B.merge(A) produce the same
/// symbols (last-wins means the second argument's data takes precedence).
#[test]
fn test_merge_order_consistency() {
    let shape_v1 = make_symbol_with_attrs("Shape", SymbolKind::Class, &[Attr::Abstract]);
    let shape_v2 = make_symbol_with_attrs("Shape", SymbolKind::Class, &[Attr::Virtual]);
    let circle = make_symbol("Circle", SymbolKind::Class);

    // A.merge(B): B's Shape wins
    let mut ga = Graph::new();
    ga.add_symbol(shape_v1.clone());
    ga.add_symbol(circle.clone());
    let mut gb = Graph::new();
    gb.add_symbol(shape_v2.clone());
    ga.merge(gb);

    let ab_shape = ga.symbols.get(&shape_v1.id).unwrap();
    assert_eq!(
        ab_shape.attrs.as_slice(),
        &[Attr::Virtual],
        "B wins in A.merge(B)"
    );

    // B.merge(A): A's Shape wins
    let mut gb2 = Graph::new();
    gb2.add_symbol(shape_v2.clone());
    let mut ga2 = Graph::new();
    ga2.add_symbol(shape_v1.clone());
    ga2.add_symbol(circle.clone());
    gb2.merge(ga2);

    let ba_shape = gb2.symbols.get(&shape_v1.id).unwrap();
    assert_eq!(
        ba_shape.attrs.as_slice(),
        &[Attr::Abstract],
        "A wins in B.merge(A)"
    );

    // Both contain the same set of symbol IDs.
    let keys_ab: HashSet<_> = ga.symbols.keys().collect();
    let keys_ba: HashSet<_> = gb2.symbols.keys().collect();
    assert_eq!(keys_ab, keys_ba, "same symbol IDs regardless of order");
}

/// Idempotency: merging a graph into itself does not duplicate symbols or
/// edges.
#[test]
fn test_merge_idempotency() {
    let mut g = Graph::new();
    let shape = make_symbol("Shape", SymbolKind::Class);
    let circle = make_symbol("Circle", SymbolKind::Class);

    let shape_id = g.add_symbol(shape);
    let circle_id = g.add_symbol(circle);
    g.add_edge(Edge {
        from: circle_id,
        to: shape_id,
        kind: EdgeKind::Inherits,
        location: None,
    });

    let clone = g.clone();
    g.merge(clone);

    assert_eq!(g.symbol_count(), 2, "symbols not duplicated");
    assert_eq!(g.edge_count(), 1, "edges not duplicated");
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
