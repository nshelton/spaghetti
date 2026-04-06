//! Tests for core-ir types and graph operations.

use std::collections::{HashMap, HashSet};

use core_ir::*;
use smallvec::smallvec;

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
// neighbors() — 5-node fixture graph
//
// Fixture topology:
//
//   A --Calls--> B --Inherits--> C
//   |                            ^
//   +--Contains--> D             |
//                                |
//   E --Calls--> E  (self-loop)  |
//   E --Inherits--> C            |
//
// Node A: connected to B (Calls) and D (Contains)
// Node B: connected to A (Calls, inbound) and C (Inherits)
// Node C: connected to B (Inherits, inbound) and E (Inherits, inbound)
// Node D: connected to A (Contains, inbound) — leaf for outgoing
// Node E: self-loop (Calls) and inherits C
// ---------------------------------------------------------------------------

/// Build the standard 5-node fixture graph used by neighbors tests.
fn neighbors_fixture() -> (Graph, SymbolId, SymbolId, SymbolId, SymbolId, SymbolId) {
    let mut g = Graph::new();
    let a = g.add_symbol(make_symbol("A", SymbolKind::Class));
    let b = g.add_symbol(make_symbol("B", SymbolKind::Class));
    let c = g.add_symbol(make_symbol("C", SymbolKind::Class));
    let d = g.add_symbol(make_symbol("D", SymbolKind::Method));
    let e = g.add_symbol(make_symbol("E", SymbolKind::Class));

    g.add_edge(Edge {
        from: a,
        to: b,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: b,
        to: c,
        kind: EdgeKind::Inherits,
        location: None,
    });
    g.add_edge(Edge {
        from: a,
        to: d,
        kind: EdgeKind::Contains,
        location: None,
    });
    g.add_edge(Edge {
        from: e,
        to: e,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: e,
        to: c,
        kind: EdgeKind::Inherits,
        location: None,
    });

    (g, a, b, c, d, e)
}

#[test]
fn test_neighbors_all_no_filter() {
    let (g, a, b, _c, _d, _e) = neighbors_fixture();

    // A is connected to B (Calls) and D (Contains) — bidirectional traversal.
    let mut all_a: Vec<SymbolId> = g.neighbors(a, &[]).collect();
    all_a.sort_by_key(|id| id.0);
    let mut expected = vec![b, _d];
    expected.sort_by_key(|id| id.0);
    assert_eq!(all_a, expected, "A should see B and D with no filter");

    // B is connected to A (Calls, inbound) and C (Inherits, outbound).
    let mut all_b: Vec<SymbolId> = g.neighbors(b, &[]).collect();
    all_b.sort_by_key(|id| id.0);
    let mut expected_b = vec![a, _c];
    expected_b.sort_by_key(|id| id.0);
    assert_eq!(all_b, expected_b, "B should see A and C with no filter");
}

#[test]
fn test_neighbors_filter_single_kind() {
    let (g, a, b, _c, _d, _e) = neighbors_fixture();

    // A --Calls--> B, so filtering by Calls from A yields B.
    let calls: Vec<SymbolId> = g.neighbors(a, &[EdgeKind::Calls]).collect();
    assert_eq!(calls, vec![b]);

    // A --Contains--> D, so filtering by Contains from A yields D.
    let contains: Vec<SymbolId> = g.neighbors(a, &[EdgeKind::Contains]).collect();
    assert_eq!(contains, vec![_d]);
}

#[test]
fn test_neighbors_filter_multiple_kinds() {
    let (g, _a, _b, c, _d, e) = neighbors_fixture();

    // E has Calls(self) and Inherits(C). Filter by both kinds.
    let mut multi: Vec<SymbolId> = g
        .neighbors(e, &[EdgeKind::Calls, EdgeKind::Inherits])
        .collect();
    multi.sort_by_key(|id| id.0);
    // Self-loop yields E twice (from and to), plus C from Inherits.
    // e.from == e and e.to == e both match, so self-loop produces 2 entries for E.
    assert!(multi.contains(&e), "E should appear (self-loop via Calls)");
    assert!(multi.contains(&c), "C should appear (Inherits from E)");
}

#[test]
fn test_neighbors_leaf_node() {
    let (g, _a, _b, _c, d, _e) = neighbors_fixture();

    // D only has an inbound Contains edge from A — bidirectional means we see A.
    let leaf: Vec<SymbolId> = g.neighbors(d, &[]).collect();
    assert_eq!(leaf, vec![_a], "D should see A via inbound Contains edge");

    // But if we filter by Calls, D has none.
    let leaf_calls: Vec<SymbolId> = g.neighbors(d, &[EdgeKind::Calls]).collect();
    assert!(leaf_calls.is_empty(), "D has no Calls edges");
}

#[test]
fn test_neighbors_unknown_node() {
    let (g, _a, _b, _c, _d, _e) = neighbors_fixture();

    let unknown = SymbolId(999_999);
    let result: Vec<SymbolId> = g.neighbors(unknown, &[]).collect();
    assert!(
        result.is_empty(),
        "unknown node should yield empty iterator"
    );
}

#[test]
fn test_neighbors_self_loop() {
    let (g, _a, _b, _c, _d, e) = neighbors_fixture();

    // E --Calls--> E (self-loop). The implementation uses if/else-if, so
    // a self-loop edge yields the node exactly once (the `from` branch fires).
    let self_refs: Vec<SymbolId> = g.neighbors(e, &[EdgeKind::Calls]).collect();
    assert_eq!(
        self_refs,
        vec![e],
        "self-loop should yield the node once per self-referencing edge"
    );
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

// ---------------------------------------------------------------------------
// CAP-002: Graph serde roundtrip golden-file tests
// ---------------------------------------------------------------------------

/// Test 1: Construct graph in code → serialize → deserialize → assert equality.
#[test]
fn test_serde_roundtrip_equality() {
    let mut g = Graph::new();
    let fid = g.files.intern("src/lib.rs");

    let cls = Symbol {
        id: SymbolId::from_parts("Foo", SymbolKind::Class),
        kind: SymbolKind::Class,
        name: "Foo".to_owned(),
        qualified_name: "Foo".to_owned(),
        location: Some(Location {
            file: fid,
            line: 1,
            col: 1,
        }),
        module: Some("mymod".to_owned()),
        attrs: smallvec![Attr::Abstract, Attr::Virtual],
    };
    let method = Symbol {
        id: SymbolId::from_parts("Foo::bar", SymbolKind::Method),
        kind: SymbolKind::Method,
        name: "bar".to_owned(),
        qualified_name: "Foo::bar".to_owned(),
        location: Some(Location {
            file: fid,
            line: 5,
            col: 5,
        }),
        module: None,
        attrs: smallvec![Attr::Const],
    };

    let cls_id = g.add_symbol(cls);
    let method_id = g.add_symbol(method);
    g.add_edge(Edge {
        from: cls_id,
        to: method_id,
        kind: EdgeKind::Contains,
        location: None,
    });

    let json = g.to_json().expect("serialize");
    let deserialized = Graph::from_json(&json).expect("deserialize");
    assert_eq!(g, deserialized);
}

/// Test 2: Load golden file → deserialize → re-serialize → deserialize → assert stability.
#[test]
fn test_golden_file_roundtrip_stability() {
    let golden_json =
        std::fs::read_to_string("tests/fixtures/serde_golden.json").expect("read golden file");
    let g1 = Graph::from_json(&golden_json).expect("first deserialize");
    let reserialized = g1.to_json().expect("re-serialize");
    let g2 = Graph::from_json(&reserialized).expect("second deserialize");
    assert_eq!(g1, g2);
}

/// Test 3: Verify the golden file covers all SymbolKind and EdgeKind variants.
#[test]
fn test_golden_file_variant_coverage() {
    let golden_json =
        std::fs::read_to_string("tests/fixtures/serde_golden.json").expect("read golden file");
    let g = Graph::from_json(&golden_json).expect("deserialize golden");

    // Collect all SymbolKind variants present
    let symbol_kinds: std::collections::HashSet<_> = g
        .symbols
        .values()
        .map(|s| std::mem::discriminant(&s.kind))
        .collect();
    let all_symbol_kinds = [
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Field,
        SymbolKind::Namespace,
        SymbolKind::TemplateInstantiation,
        SymbolKind::TranslationUnit,
    ];
    for kind in &all_symbol_kinds {
        assert!(
            symbol_kinds.contains(&std::mem::discriminant(kind)),
            "golden file missing SymbolKind::{kind:?}"
        );
    }

    // Collect all EdgeKind variants present
    let edge_kinds: std::collections::HashSet<_> = g
        .edges
        .iter()
        .map(|e| std::mem::discriminant(&e.kind))
        .collect();
    let all_edge_kinds = [
        EdgeKind::Calls,
        EdgeKind::Inherits,
        EdgeKind::Contains,
        EdgeKind::ReadsField,
        EdgeKind::WritesField,
        EdgeKind::Includes,
        EdgeKind::Instantiates,
        EdgeKind::HasType,
        EdgeKind::Overrides,
    ];
    for kind in &all_edge_kinds {
        assert!(
            edge_kinds.contains(&std::mem::discriminant(kind)),
            "golden file missing EdgeKind::{kind:?}"
        );
    }
}

/// Test 4: Empty graph roundtrips correctly.
#[test]
fn test_empty_graph_roundtrip() {
    let g = Graph::new();
    let json = g.to_json().expect("serialize empty");
    let g2 = Graph::from_json(&json).expect("deserialize empty");
    assert_eq!(g, g2);
    assert_eq!(g2.symbol_count(), 0);
    assert_eq!(g2.edge_count(), 0);
}

/// Test 5: `examples/tiny-cpp/graph.json` roundtrips without data loss.
#[test]
fn test_tiny_cpp_graph_roundtrip() {
    let original_json = std::fs::read_to_string("../../examples/tiny-cpp/graph.json")
        .expect("read tiny-cpp graph.json");
    let g1 = Graph::from_json(&original_json).expect("first deserialize");
    let reserialized = g1.to_json().expect("re-serialize");
    let g2 = Graph::from_json(&reserialized).expect("second deserialize");
    assert_eq!(g1, g2);
    // Sanity check: the tiny-cpp graph has known counts
    assert_eq!(g1.symbol_count(), 7);
    assert_eq!(g1.edge_count(), 9);
}
