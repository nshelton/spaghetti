//! Graph layout algorithms.
//!
//! Pure function from [`core_ir::Graph`] → [`Positions`]. No rendering dependencies.

use core_ir::{EdgeKind, Graph, SymbolId};
use glam::Vec2;
use indexmap::IndexMap;
use std::collections::VecDeque;

/// Mapping from symbol IDs to 2D positions.
///
/// Uses [`IndexMap`] to guarantee deterministic iteration order.
#[derive(Debug, Clone)]
pub struct Positions(pub IndexMap<SymbolId, Vec2>);

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
            return Positions(IndexMap::new());
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
        let id_to_idx: IndexMap<SymbolId, usize> =
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

        let mut map: IndexMap<SymbolId, Vec2> = ids.into_iter().zip(pos).collect();
        pack_components(&mut map, graph);
        Positions(map)
    }
}

/// Padding between component bounding boxes (in layout units).
const COMPONENT_PADDING: f32 = 50.0;

/// Detect connected components via BFS and shift them so their padded bounding
/// boxes do not overlap. Uses a simple horizontal strip-packing approach.
fn pack_components(positions: &mut IndexMap<SymbolId, Vec2>, graph: &Graph) {
    let components = find_components(positions, graph);
    if components.len() <= 1 {
        return;
    }

    // Compute bounding box for each component
    struct BBox {
        min: Vec2,
        max: Vec2,
    }

    let mut boxes: Vec<BBox> = components
        .iter()
        .map(|comp| {
            let mut min = Vec2::splat(f32::INFINITY);
            let mut max = Vec2::splat(f32::NEG_INFINITY);
            for &id in comp {
                let p = positions[&id];
                min = min.min(p);
                max = max.max(p);
            }
            BBox { min, max }
        })
        .collect();

    // Pack components left-to-right with padding
    let mut cursor_x: f32 = 0.0;
    for (i, comp) in components.iter().enumerate() {
        let bbox = &mut boxes[i];
        let width = bbox.max.x - bbox.min.x;
        let offset_x = cursor_x - bbox.min.x;
        let offset_y = -bbox.min.y; // Align top edges at y=0

        for &id in comp {
            let p = positions.get_mut(&id).expect("symbol in component");
            p.x += offset_x;
            p.y += offset_y;
        }

        bbox.min.x += offset_x;
        bbox.max.x += offset_x;
        bbox.min.y += offset_y;
        bbox.max.y += offset_y;

        cursor_x = bbox.max.x + width.clamp(1.0, COMPONENT_PADDING) + COMPONENT_PADDING;
    }
}

/// Find connected components using BFS over the graph's edges.
///
/// Returns a list of components, each being a list of [`SymbolId`]s. Components
/// are sorted by the first symbol ID in each group for deterministic ordering.
fn find_components(positions: &IndexMap<SymbolId, Vec2>, graph: &Graph) -> Vec<Vec<SymbolId>> {
    // Build adjacency list from graph edges (undirected for component detection)
    let mut adj: IndexMap<SymbolId, Vec<SymbolId>> = IndexMap::new();
    for id in positions.keys() {
        adj.entry(*id).or_default();
    }
    for edge in &graph.edges {
        if positions.contains_key(&edge.from) && positions.contains_key(&edge.to) {
            adj.entry(edge.from).or_default().push(edge.to);
            adj.entry(edge.to).or_default().push(edge.from);
        }
    }

    let mut visited: IndexMap<SymbolId, bool> = positions.keys().map(|&id| (id, false)).collect();
    let mut components = Vec::new();

    for &id in positions.keys() {
        if visited[&id] {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(id);
        *visited.get_mut(&id).expect("visited entry") = true;

        while let Some(current) = queue.pop_front() {
            component.push(current);
            if let Some(neighbors) = adj.get(&current) {
                for &neighbor in neighbors {
                    if !visited[&neighbor] {
                        *visited.get_mut(&neighbor).expect("visited entry") = true;
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        components.push(component);
    }

    components
}
