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

#[test]
fn test_determinism() {
    let g = three_node_graph();
    let layout = ForceDirected {
        seed: 42,
        iterations: 100,
    };
    let p1 = layout.compute(&g);
    let p2 = layout.compute(&g);

    for (id, pos1) in &p1.0 {
        let pos2 = p2.0.get(id).expect("missing symbol in second run");
        assert!(
            (*pos1 - *pos2).length() < f32::EPSILON,
            "positions differ for {id:?}: {pos1} vs {pos2}"
        );
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
