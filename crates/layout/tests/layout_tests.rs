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
