//! Tests for query functions.

use core_ir::{Edge, EdgeKind, Graph, Location, Symbol, SymbolId, SymbolKind};
use query::{callers_of, find_by_name, split_shared_types, subgraph_around};

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

// ---------------------------------------------------------------------------
// CAP-008: callers_of directional correctness tests
// ---------------------------------------------------------------------------

/// Helper to build a graph for CAP-008 tests.
///
/// ```text
///   caller1 --Calls--> target --Calls--> callee1
///   caller2 --Calls--> target
///   unrelated --Inherits--> target
///   target --Calls--> target  (self-call)
/// ```
fn cap008_graph() -> (Graph, SymbolId, SymbolId, SymbolId, SymbolId, SymbolId) {
    let mut g = Graph::new();
    let caller1 = make_symbol("caller1", SymbolKind::Function);
    let caller2 = make_symbol("caller2", SymbolKind::Function);
    let target = make_symbol("target", SymbolKind::Method);
    let callee1 = make_symbol("callee1", SymbolKind::Function);
    let unrelated = make_symbol("unrelated", SymbolKind::Class);

    let (c1, c2, t, ce1, u) = (caller1.id, caller2.id, target.id, callee1.id, unrelated.id);

    g.add_symbol(caller1);
    g.add_symbol(caller2);
    g.add_symbol(target);
    g.add_symbol(callee1);
    g.add_symbol(unrelated);

    // Incoming Calls edges to target
    g.add_edge(Edge {
        from: c1,
        to: t,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: c2,
        to: t,
        kind: EdgeKind::Calls,
        location: None,
    });
    // Outgoing Calls edge from target (callee, should NOT appear)
    g.add_edge(Edge {
        from: t,
        to: ce1,
        kind: EdgeKind::Calls,
        location: None,
    });
    // Non-call edge pointing to target (should NOT appear)
    g.add_edge(Edge {
        from: u,
        to: t,
        kind: EdgeKind::Inherits,
        location: None,
    });
    // Self-call
    g.add_edge(Edge {
        from: t,
        to: t,
        kind: EdgeKind::Calls,
        location: None,
    });

    (g, c1, c2, t, ce1, u)
}

/// CAP-008 test 1: multiple symbols calling one target are all returned.
#[test]
fn test_cap008_basic_callers() {
    let (g, c1, c2, t, ..) = cap008_graph();
    let callers = callers_of(&g, t);
    assert!(callers.contains(&c1), "caller1 should be a caller");
    assert!(callers.contains(&c2), "caller2 should be a caller");
}

/// CAP-008 test 2: callees (outgoing edges) are excluded.
#[test]
fn test_cap008_exclude_callees() {
    let (g, _, _, t, ce1, _) = cap008_graph();
    let callers = callers_of(&g, t);
    assert!(
        !callers.contains(&ce1),
        "callee1 must not appear — it is called BY target, not a caller OF target"
    );
}

/// CAP-008 test 3: returns empty when nothing calls the target.
#[test]
fn test_cap008_empty_callers() {
    let (g, _, _, _, ce1, _) = cap008_graph();
    // callee1 has no incoming Calls edges in this graph
    let callers = callers_of(&g, ce1);
    // Only target calls callee1, so callers should be [target]
    // Actually test a node with truly zero callers — caller1 has none.
    let (g2, c1, ..) = cap008_graph();
    let callers2 = callers_of(&g2, c1);
    assert!(callers2.is_empty(), "caller1 has no incoming Calls edges");
    let _ = (g, callers);
}

/// CAP-008 test 4: non-call edges (Inherits, Contains, etc.) are ignored.
#[test]
fn test_cap008_edge_type_filtering() {
    let (g, _, _, t, _, u) = cap008_graph();
    let callers = callers_of(&g, t);
    assert!(
        !callers.contains(&u),
        "unrelated has an Inherits edge to target, not Calls — must be excluded"
    );
}

/// CAP-008 test 5: unknown ID returns empty vec without panicking.
#[test]
fn test_cap008_unknown_id() {
    let (g, ..) = cap008_graph();
    let missing = SymbolId::from_parts("does_not_exist", SymbolKind::Function);
    let callers = callers_of(&g, missing);
    assert!(callers.is_empty(), "unknown ID must return empty results");
}

/// CAP-008 test 6: self-call — target calls itself, so it should appear as its own caller.
#[test]
fn test_cap008_self_call() {
    let (g, _, _, t, ..) = cap008_graph();
    let callers = callers_of(&g, t);
    assert!(
        callers.contains(&t),
        "target calls itself — it should appear in its own callers list"
    );
}

// ---------------------------------------------------------------------------
// split_shared_types
// ---------------------------------------------------------------------------

/// Foo { vec3 a; vec3 b; }  Bar { vec3 c; }  Baz { Unique u; }
/// plus a container-less field `loose` of type vec3.
fn split_test_graph() -> Graph {
    let mut g = Graph::new();
    let foo_file = g.files.intern("src/foo.h");
    let math_file = g.files.intern("src/math.h");

    let mut foo = make_symbol("Foo", SymbolKind::Struct);
    foo.location = Some(Location {
        file: foo_file,
        line: 1,
        col: 1,
    });
    let mut vec3 = make_symbol("vec3", SymbolKind::Struct);
    vec3.location = Some(Location {
        file: math_file,
        line: 1,
        col: 1,
    });
    let bar = make_symbol("Bar", SymbolKind::Struct);
    let baz = make_symbol("Baz", SymbolKind::Struct);
    let unique = make_symbol("Unique", SymbolKind::Struct);
    let fa = make_symbol("Foo::a", SymbolKind::Field);
    let fb = make_symbol("Foo::b", SymbolKind::Field);
    let bc = make_symbol("Bar::c", SymbolKind::Field);
    let bu = make_symbol("Baz::u", SymbolKind::Field);
    let loose = make_symbol("loose", SymbolKind::Field);

    let contains = [
        (foo.id, fa.id),
        (foo.id, fb.id),
        (bar.id, bc.id),
        (baz.id, bu.id),
    ];
    let has_type = [
        (fa.id, vec3.id),
        (fb.id, vec3.id),
        (bc.id, vec3.id),
        (bu.id, unique.id),
        (loose.id, vec3.id),
    ];

    for s in [foo, bar, baz, vec3, unique, fa, fb, bc, bu, loose] {
        g.add_symbol(s);
    }
    for (from, to) in contains {
        g.add_edge(Edge {
            from,
            to,
            kind: EdgeKind::Contains,
            location: None,
        });
    }
    for (from, to) in has_type {
        g.add_edge(Edge {
            from,
            to,
            kind: EdgeKind::HasType,
            location: None,
        });
    }
    g
}

#[test]
fn test_split_creates_clone_per_container() {
    let g = split_test_graph();
    let out = split_shared_types(&g);

    // vec3 is referenced from Foo and Bar → one clone each, with
    // deterministic IDs derived from "type@container".
    let foo_clone = SymbolId::from_parts("vec3@Foo", SymbolKind::Struct);
    let bar_clone = SymbolId::from_parts("vec3@Bar", SymbolKind::Struct);
    assert!(out.symbols.contains_key(&foo_clone));
    assert!(out.symbols.contains_key(&bar_clone));
    assert_eq!(out.symbol_count(), g.symbol_count() + 2);

    // Clones keep the display name and inherit the container's location.
    let clone = &out.symbols[&foo_clone];
    let foo = &out.symbols[&SymbolId::from_parts("Foo", SymbolKind::Struct)];
    assert_eq!(clone.name, "vec3");
    assert_eq!(clone.location, foo.location);

    // Each clone is nested in its container via Contains.
    let foo_id = SymbolId::from_parts("Foo", SymbolKind::Struct);
    assert!(out
        .edges
        .iter()
        .any(|e| e.from == foo_id && e.to == foo_clone && e.kind == EdgeKind::Contains));
}

#[test]
fn test_split_retargets_fan_in() {
    let g = split_test_graph();
    let out = split_shared_types(&g);

    let vec3_id = SymbolId::from_parts("vec3", SymbolKind::Struct);
    let foo_clone = SymbolId::from_parts("vec3@Foo", SymbolKind::Struct);
    let fa = SymbolId::from_parts("Foo::a", SymbolKind::Field);
    let fb = SymbolId::from_parts("Foo::b", SymbolKind::Field);

    // Both Foo fields point at the same per-Foo clone.
    for field in [fa, fb] {
        assert!(out
            .edges
            .iter()
            .any(|e| e.from == field && e.to == foo_clone && e.kind == EdgeKind::HasType));
    }

    // The container-less field keeps its edge to the original; that is the
    // only HasType fan-in the original retains.
    let loose = SymbolId::from_parts("loose", SymbolKind::Field);
    let into_original: Vec<_> = out
        .edges
        .iter()
        .filter(|e| e.to == vec3_id && e.kind == EdgeKind::HasType)
        .collect();
    assert_eq!(into_original.len(), 1);
    assert_eq!(into_original[0].from, loose);

    // The original type node itself survives.
    assert!(out.symbols.contains_key(&vec3_id));
}

#[test]
fn test_split_leaves_single_container_types_alone() {
    let g = split_test_graph();
    let out = split_shared_types(&g);

    // Unique is only referenced from Baz → not split, edge untouched.
    let unique_id = SymbolId::from_parts("Unique", SymbolKind::Struct);
    let bu = SymbolId::from_parts("Baz::u", SymbolKind::Field);
    assert!(out
        .edges
        .iter()
        .any(|e| e.from == bu && e.to == unique_id && e.kind == EdgeKind::HasType));
    assert!(!out
        .symbols
        .contains_key(&SymbolId::from_parts("Unique@Baz", SymbolKind::Struct)));
}

#[test]
fn test_split_is_deterministic() {
    let g = split_test_graph();
    assert_eq!(split_shared_types(&g), split_shared_types(&g));
}
