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
