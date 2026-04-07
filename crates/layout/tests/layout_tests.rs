//! Tests for layout algorithms.

use core_ir::{Edge, EdgeKind, Graph, Symbol, SymbolId, SymbolKind};
use glam::Vec2;
use layout::{ForceDirected, Layout};

fn make_symbol(name: &str, kind: SymbolKind) -> Symbol {
    Symbol {
        id: SymbolId::from_parts(name, kind),
        kind,
        name: name.to_owned(),
        qualified_name: name.to_owned(),
        location: None,
        module: None,
        attrs: Default::default(),
    }
}

fn three_node_graph() -> Graph {
    let mut g = Graph::new();
    let a = make_symbol("A", SymbolKind::Class);
    let b = make_symbol("B", SymbolKind::Class);
    let c = make_symbol("C", SymbolKind::Class);
    let a_id = a.id;
    let b_id = b.id;
    g.add_symbol(a);
    g.add_symbol(b);
    g.add_symbol(c);
    g.add_edge(Edge {
        from: a_id,
        to: b_id,
        kind: EdgeKind::Inherits,
        location: None,
    });
    g
}

/// Run-to-run consistency: 10 iterations with identical inputs must produce
/// bitwise-identical f32 positions every time.
#[test]
fn test_run_to_run_consistency() {
    let g = three_node_graph();
    let layout = ForceDirected {
        seed: 42,
        iterations: 100,
    };

    let reference = layout.compute(&g);

    for run in 0..10 {
        let result = layout.compute(&g);
        for (id, pos1) in &reference.0 {
            let pos2 = result.0.get(id).expect("missing symbol");
            assert_eq!(
                pos1.x.to_bits(),
                pos2.x.to_bits(),
                "run {run}: x differs for {id:?}"
            );
            assert_eq!(
                pos1.y.to_bits(),
                pos2.y.to_bits(),
                "run {run}: y differs for {id:?}"
            );
        }
    }
}

/// Golden-file validation: positions must match the committed reference values
/// at the bit level.
#[test]
fn test_golden_file() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/layout_golden.json"))
            .expect("failed to parse golden file");

    let g = three_node_graph();
    let seed = golden["seed"].as_u64().unwrap();
    let iterations = golden["iterations"].as_u64().unwrap() as u32;
    let layout = ForceDirected { seed, iterations };
    let positions = layout.compute(&g);

    let expected = golden["expected_positions"].as_object().unwrap();

    for (id_str, bits_val) in expected {
        let id = SymbolId(id_str.parse::<u64>().unwrap());
        let pos = positions
            .0
            .get(&id)
            .unwrap_or_else(|| panic!("missing symbol {id_str} in computed positions"));

        let expected_bits = bits_val.as_array().unwrap();
        let expected_x_bits = expected_bits[0].as_u64().unwrap() as u32;
        let expected_y_bits = expected_bits[1].as_u64().unwrap() as u32;

        assert_eq!(
            pos.x.to_bits(),
            expected_x_bits,
            "golden x mismatch for symbol {id_str}: got {} (bits {}), expected bits {expected_x_bits}",
            pos.x,
            pos.x.to_bits()
        );
        assert_eq!(
            pos.y.to_bits(),
            expected_y_bits,
            "golden y mismatch for symbol {id_str}: got {} (bits {}), expected bits {expected_y_bits}",
            pos.y,
            pos.y.to_bits()
        );
    }
}

/// Different seeds must produce different layouts.
#[test]
fn test_seed_variation() {
    let g = three_node_graph();
    let layout_a = ForceDirected {
        seed: 1,
        iterations: 100,
    };
    let layout_b = ForceDirected {
        seed: 999,
        iterations: 100,
    };

    let pos_a = layout_a.compute(&g);
    let pos_b = layout_b.compute(&g);

    // Collect positions as vectors for comparison
    let vals_a: Vec<Vec2> = pos_a.0.values().copied().collect();
    let vals_b: Vec<Vec2> = pos_b.0.values().copied().collect();

    // At least one position must differ
    let any_differ = vals_a
        .iter()
        .zip(&vals_b)
        .any(|(a, b)| a.x.to_bits() != b.x.to_bits() || a.y.to_bits() != b.y.to_bits());

    assert!(any_differ, "different seeds produced identical layouts");
}

/// Single-node graph must not produce NaN or infinity values.
#[test]
fn test_single_node_no_nan() {
    let mut g = Graph::new();
    g.add_symbol(make_symbol("Alone", SymbolKind::Class));

    let layout = ForceDirected {
        seed: 42,
        iterations: 100,
    };
    let positions = layout.compute(&g);

    assert_eq!(positions.0.len(), 1);
    for (id, pos) in &positions.0 {
        assert!(pos.x.is_finite(), "NaN/Inf x for {id:?}: {}", pos.x);
        assert!(pos.y.is_finite(), "NaN/Inf y for {id:?}: {}", pos.y);
    }
}

#[test]
fn test_non_overlapping() {
    let g = three_node_graph();
    let layout = ForceDirected {
        seed: 42,
        iterations: 200,
    };
    let positions = layout.compute(&g);
    let pts: Vec<Vec2> = positions.0.values().copied().collect();

    // All pairs should be separated by at least some minimum distance
    for i in 0..pts.len() {
        for j in (i + 1)..pts.len() {
            let dist = (pts[i] - pts[j]).length();
            assert!(dist > 10.0, "nodes {i} and {j} are too close: {dist} apart");
        }
    }
}

#[test]
fn test_empty_graph() {
    let g = Graph::new();
    let layout = ForceDirected::default();
    let positions = layout.compute(&g);
    assert!(positions.0.is_empty());
}

// ---------------------------------------------------------------------------
// CAP-006: Disconnected component tests
// ---------------------------------------------------------------------------

/// Build two disconnected 3-node triangles.
fn disconnected_graph() -> Graph {
    let mut g = Graph::new();

    let a1 = make_symbol("A1", SymbolKind::Class);
    let b1 = make_symbol("B1", SymbolKind::Class);
    let c1 = make_symbol("C1", SymbolKind::Class);
    let a1_id = a1.id;
    let b1_id = b1.id;
    let c1_id = c1.id;
    g.add_symbol(a1);
    g.add_symbol(b1);
    g.add_symbol(c1);
    g.add_edge(Edge {
        from: a1_id,
        to: b1_id,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: b1_id,
        to: c1_id,
        kind: EdgeKind::Calls,
        location: None,
    });

    let a2 = make_symbol("A2", SymbolKind::Class);
    let b2 = make_symbol("B2", SymbolKind::Class);
    let c2 = make_symbol("C2", SymbolKind::Class);
    let a2_id = a2.id;
    let b2_id = b2.id;
    let c2_id = c2.id;
    g.add_symbol(a2);
    g.add_symbol(b2);
    g.add_symbol(c2);
    g.add_edge(Edge {
        from: a2_id,
        to: b2_id,
        kind: EdgeKind::Calls,
        location: None,
    });
    g.add_edge(Edge {
        from: b2_id,
        to: c2_id,
        kind: EdgeKind::Calls,
        location: None,
    });

    g
}

/// All positions must be finite (no NaN or Infinity) after layout.
#[test]
fn test_disconnected_finite_positions() {
    let g = disconnected_graph();
    let layout = ForceDirected {
        seed: 42,
        iterations: 200,
    };
    let positions = layout.compute(&g);

    assert_eq!(positions.0.len(), 6);
    for (id, pos) in &positions.0 {
        assert!(pos.x.is_finite(), "NaN/Inf x for {id:?}");
        assert!(pos.y.is_finite(), "NaN/Inf y for {id:?}");
    }
}

/// All positions in a 6-node disconnected graph must stay within [-10000, 10000].
#[test]
fn test_disconnected_bounded() {
    let g = disconnected_graph();
    let layout = ForceDirected {
        seed: 42,
        iterations: 200,
    };
    let positions = layout.compute(&g);

    for (id, pos) in &positions.0 {
        assert!(
            pos.x.abs() <= 10000.0,
            "x out of bounds for {id:?}: {}",
            pos.x
        );
        assert!(
            pos.y.abs() <= 10000.0,
            "y out of bounds for {id:?}: {}",
            pos.y
        );
    }
}

/// Bounding boxes of the two disconnected components (with 50px padding) must
/// not intersect.
#[test]
fn test_disconnected_non_overlapping_components() {
    let g = disconnected_graph();
    let layout = ForceDirected {
        seed: 42,
        iterations: 200,
    };
    let positions = layout.compute(&g);

    // Identify component membership by name prefix
    let comp1: Vec<Vec2> = positions
        .0
        .iter()
        .filter(|(&id, _)| g.symbols.get(&id).is_some_and(|s| s.name.ends_with('1')))
        .map(|(_, &p)| p)
        .collect();

    let comp2: Vec<Vec2> = positions
        .0
        .iter()
        .filter(|(&id, _)| g.symbols.get(&id).is_some_and(|s| s.name.ends_with('2')))
        .map(|(_, &p)| p)
        .collect();

    assert_eq!(comp1.len(), 3);
    assert_eq!(comp2.len(), 3);

    let bbox = |pts: &[Vec2]| -> (Vec2, Vec2) {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for &p in pts {
            min = min.min(p);
            max = max.max(p);
        }
        (min, max)
    };

    let (min1, max1) = bbox(&comp1);
    let (min2, max2) = bbox(&comp2);

    let padding = 50.0;
    // Check axis-aligned bounding box non-overlap with padding
    let overlap_x =
        (min1.x - padding) < (max2.x + padding) && (max1.x + padding) > (min2.x - padding);
    let overlap_y =
        (min1.y - padding) < (max2.y + padding) && (max1.y + padding) > (min2.y - padding);

    assert!(
        !(overlap_x && overlap_y),
        "component bounding boxes overlap with {padding}px padding.\n\
         comp1: ({}, {}) -> ({}, {})\n\
         comp2: ({}, {}) -> ({}, {})",
        min1.x,
        min1.y,
        max1.x,
        max1.y,
        min2.x,
        min2.y,
        max2.x,
        max2.y,
    );
}

/// A single isolated node added alongside a connected component must not
/// overlap with it.
#[test]
fn test_disconnected_isolated_node() {
    let mut g = disconnected_graph();
    let lonely = make_symbol("Lonely", SymbolKind::Class);
    g.add_symbol(lonely);

    let layout = ForceDirected {
        seed: 42,
        iterations: 200,
    };
    let positions = layout.compute(&g);

    assert_eq!(positions.0.len(), 7);

    // All positions must be finite
    for (id, pos) in &positions.0 {
        assert!(pos.x.is_finite(), "NaN/Inf x for {id:?}");
        assert!(pos.y.is_finite(), "NaN/Inf y for {id:?}");
    }

    // The lonely node must not sit inside either component's bounding box
    let lonely_id = SymbolId::from_parts("Lonely", SymbolKind::Class);
    let lonely_pos = positions.0.get(&lonely_id).expect("lonely node present");

    let comp_ids: Vec<Vec<Vec2>> = vec![
        positions
            .0
            .iter()
            .filter(|(&id, _)| g.symbols.get(&id).is_some_and(|s| s.name.ends_with('1')))
            .map(|(_, &p)| p)
            .collect(),
        positions
            .0
            .iter()
            .filter(|(&id, _)| g.symbols.get(&id).is_some_and(|s| s.name.ends_with('2')))
            .map(|(_, &p)| p)
            .collect(),
    ];

    let padding = 50.0;
    for (i, comp) in comp_ids.iter().enumerate() {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for &p in comp {
            min = min.min(p);
            max = max.max(p);
        }

        let inside_x = lonely_pos.x >= (min.x - padding) && lonely_pos.x <= (max.x + padding);
        let inside_y = lonely_pos.y >= (min.y - padding) && lonely_pos.y <= (max.y + padding);

        assert!(
            !(inside_x && inside_y),
            "lonely node at ({}, {}) is inside component {i}'s padded bbox: ({}, {}) -> ({}, {})",
            lonely_pos.x,
            lonely_pos.y,
            min.x - padding,
            min.y - padding,
            max.x + padding,
            max.y + padding,
        );
    }
}
