//! Graph layout algorithms.
//!
//! Pure function from [`core_ir::Graph`] → [`Positions`]. No rendering dependencies.

use std::collections::HashMap;

use core_ir::{EdgeKind, Graph, SymbolId};
use glam::Vec2;

/// Mapping from symbol IDs to 2D positions.
#[derive(Debug, Clone)]
pub struct Positions(pub HashMap<SymbolId, Vec2>);

/// A layout algorithm that computes positions for graph nodes.
pub trait Layout {
    /// Compute positions for all symbols in the graph.
    fn compute(&self, graph: &Graph) -> Positions;
}

/// Force-directed layout using a simplified Barnes-Hut approach.
///
/// Deterministic given a fixed seed and iteration count.
pub struct ForceDirected {
    /// Random seed for initial placement.
    pub seed: u64,
    /// Number of simulation iterations.
    pub iterations: u32,
}

impl Default for ForceDirected {
    fn default() -> Self {
        Self {
            seed: 42,
            iterations: 200,
        }
    }
}

impl Layout for ForceDirected {
    fn compute(&self, graph: &Graph) -> Positions {
        let ids: Vec<SymbolId> = graph.symbols.keys().copied().collect();
        let n = ids.len();
        if n == 0 {
            return Positions(HashMap::new());
        }

        // Deterministic initial positions using a simple hash-based scatter
        let mut pos: Vec<Vec2> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let hash = self.seed.wrapping_mul(id.0).wrapping_add(i as u64);
                let x = ((hash & 0xFFFF) as f32 / 65535.0 - 0.5) * 400.0;
                let y = (((hash >> 16) & 0xFFFF) as f32 / 65535.0 - 0.5) * 400.0;
                Vec2::new(x, y)
            })
            .collect();

        // Build edge index (as pairs of position indices)
        let id_to_idx: HashMap<SymbolId, usize> =
            ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let edge_pairs: Vec<(usize, usize)> = graph
            .edges
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    EdgeKind::Calls | EdgeKind::Inherits | EdgeKind::Contains | EdgeKind::Overrides
                )
            })
            .filter_map(|e| {
                let from = id_to_idx.get(&e.from)?;
                let to = id_to_idx.get(&e.to)?;
                Some((*from, *to))
            })
            .collect();

        // Force-directed simulation
        let repulsion = 5000.0_f32;
        let attraction = 0.01_f32;
        let damping = 0.9_f32;
        let ideal_length = 150.0_f32;
        let min_dist = 1.0_f32;

        let mut velocities = vec![Vec2::ZERO; n];

        for _ in 0..self.iterations {
            let mut forces = vec![Vec2::ZERO; n];

            // Repulsive forces (all pairs — simplified, no Barnes-Hut quadtree for v0)
            // TODO: Barnes-Hut octree for O(n log n) instead of O(n^2)
            for i in 0..n {
                for j in (i + 1)..n {
                    let delta = pos[i] - pos[j];
                    let dist = delta.length().max(min_dist);
                    let force = delta.normalize_or_zero() * (repulsion / (dist * dist));
                    forces[i] += force;
                    forces[j] -= force;
                }
            }

            // Attractive forces along edges
            for &(from, to) in &edge_pairs {
                let delta = pos[to] - pos[from];
                let dist = delta.length().max(min_dist);
                let force = delta.normalize_or_zero() * attraction * (dist - ideal_length);
                forces[from] += force;
                forces[to] -= force;
            }

            // Update velocities and positions
            for i in 0..n {
                velocities[i] = (velocities[i] + forces[i]) * damping;
                pos[i] += velocities[i];
            }
        }

        let map: HashMap<SymbolId, Vec2> = ids.into_iter().zip(pos).collect();
        Positions(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_ir::{Edge, EdgeKind, Symbol, SymbolId, SymbolKind};

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
}
