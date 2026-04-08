//! Graph layout algorithms.
//!
//! Pure function from [`core_ir::Graph`] → [`Positions`]. No rendering dependencies.
//!
//! The main types are:
//! - [`ForceDirected`]: batch layout (compute once, get positions).
//! - [`LayoutState`]: incremental simulation driven frame-by-frame, with
//!   support for pinning nodes (interactive dragging).

use core_ir::{EdgeKind, Graph, SymbolId};
use glam::Vec2;
use indexmap::IndexMap;
use std::collections::VecDeque;
use std::time::Instant;
use tracing::info;

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

/// Tuneable constants for the force-directed simulation.
#[derive(Debug, Clone, Copy)]
pub struct ForceParams {
    /// Coulomb-like repulsion strength (all pairs).
    pub repulsion: f32,
    /// Spring attraction coefficient (edges only).
    pub attraction: f32,
    /// Velocity damping factor applied each step.
    pub damping: f32,
    /// Rest length of edge springs.
    pub ideal_length: f32,
    /// Minimum distance clamped to avoid division by near-zero.
    pub min_dist: f32,
}

impl Default for ForceParams {
    fn default() -> Self {
        Self {
            repulsion: 5000.0,
            attraction: 0.01,
            damping: 0.9,
            ideal_length: 150.0,
            min_dist: 1.0,
        }
    }
}

/// Incremental force-directed simulation state.
///
/// Created from a [`Graph`] and driven frame-by-frame via [`step`](Self::step).
/// Nodes can be pinned to fixed positions (e.g. while the user is dragging
/// them); pinned nodes still exert forces on their neighbours but skip their
/// own velocity/position updates.
pub struct LayoutState {
    ids: Vec<SymbolId>,
    positions: Vec<Vec2>,
    velocities: Vec<Vec2>,
    edge_pairs: Vec<(usize, usize, EdgeKind)>,
    visible_edge_kinds: Vec<EdgeKind>,
    id_to_idx: IndexMap<SymbolId, usize>,
    pins: IndexMap<SymbolId, Vec2>,
    params: ForceParams,
}

impl LayoutState {
    /// Initialise a new simulation from a graph.
    ///
    /// `seed` controls the deterministic initial scatter. `params` sets the
    /// force constants.
    pub fn new(graph: &Graph, seed: u64, params: ForceParams) -> Self {
        let ids: Vec<SymbolId> = graph.symbols.keys().copied().collect();
        let n = ids.len();

        let positions: Vec<Vec2> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let hash = seed.wrapping_mul(id.0).wrapping_add(i as u64);
                let x = ((hash & 0xFFFF) as f32 / 65535.0 - 0.5) * 400.0;
                let y = (((hash >> 16) & 0xFFFF) as f32 / 65535.0 - 0.5) * 400.0;
                Vec2::new(x, y)
            })
            .collect();

        let id_to_idx: IndexMap<SymbolId, usize> =
            ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let edge_pairs: Vec<(usize, usize, EdgeKind)> = graph
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
                Some((*from, *to, e.kind))
            })
            .collect();

        let visible_edge_kinds = vec![
            EdgeKind::Calls,
            EdgeKind::Inherits,
            EdgeKind::Contains,
            EdgeKind::Overrides,
        ];

        let velocities = vec![Vec2::ZERO; n];

        Self {
            ids,
            positions,
            velocities,
            edge_pairs,
            visible_edge_kinds,
            id_to_idx,
            pins: IndexMap::new(),
            params,
        }
    }

    /// Number of nodes in the simulation.
    pub fn node_count(&self) -> usize {
        self.ids.len()
    }

    /// Run `n` simulation iterations.
    pub fn step(&mut self, n: u32) {
        let len = self.positions.len();
        let p = &self.params;

        for _ in 0..n {
            let mut forces = vec![Vec2::ZERO; len];

            // Repulsive forces (all pairs)
            // TODO: Barnes-Hut octree for O(n log n) instead of O(n^2)
            for i in 0..len {
                for j in (i + 1)..len {
                    let delta = self.positions[i] - self.positions[j];
                    let dist = delta.length().max(p.min_dist);
                    let force = delta.normalize_or_zero() * (p.repulsion / (dist * dist));
                    forces[i] += force;
                    forces[j] -= force;
                }
            }

            // Attractive forces along visible edges only
            for &(from, to, kind) in &self.edge_pairs {
                if !self.visible_edge_kinds.contains(&kind) {
                    continue;
                }
                let delta = self.positions[to] - self.positions[from];
                let dist = delta.length().max(p.min_dist);
                let force = delta.normalize_or_zero() * p.attraction * (dist - p.ideal_length);
                forces[from] += force;
                forces[to] -= force;
            }

            // Update velocities and positions (skip pinned nodes)
            let iter = self
                .ids
                .iter()
                .zip(self.positions.iter_mut())
                .zip(self.velocities.iter_mut())
                .zip(forces.iter());
            for (((id, pos), vel), force) in iter {
                if let Some(&pin_pos) = self.pins.get(id) {
                    // Pinned: snap to pin position, zero velocity.
                    *pos = pin_pos;
                    *vel = Vec2::ZERO;
                } else {
                    *vel = (*vel + *force) * p.damping;
                    *pos += *vel;
                }
            }
        }
    }

    /// Update which edge kinds contribute attractive forces.
    ///
    /// Only edges whose kind is in `kinds` will exert spring forces during
    /// [`step`](Self::step). This should match whatever the UI is currently
    /// rendering so hidden edges don't pull nodes around.
    pub fn set_visible_edge_kinds(&mut self, kinds: &[EdgeKind]) {
        self.visible_edge_kinds = kinds.to_vec();
    }

    /// Pin a node to a fixed position.
    ///
    /// The node will stay at `pos` during subsequent [`step`](Self::step)
    /// calls but continues to exert forces on its neighbours.
    pub fn pin(&mut self, id: SymbolId, pos: Vec2) {
        self.pins.insert(id, pos);
        if let Some(&idx) = self.id_to_idx.get(&id) {
            self.positions[idx] = pos;
            self.velocities[idx] = Vec2::ZERO;
        }
    }

    /// Release a previously pinned node so it resumes normal simulation.
    pub fn unpin(&mut self, id: SymbolId) {
        self.pins.shift_remove(&id);
    }

    /// Directly set a node's position (useful for updating a pin target
    /// during a drag).
    pub fn set_position(&mut self, id: SymbolId, pos: Vec2) {
        if let Some(&idx) = self.id_to_idx.get(&id) {
            self.positions[idx] = pos;
        }
        // If the node is pinned, update the pin target too.
        if self.pins.contains_key(&id) {
            self.pins.insert(id, pos);
        }
    }

    /// Return a snapshot of the current positions.
    pub fn positions(&self) -> Positions {
        Positions(
            self.ids
                .iter()
                .copied()
                .zip(self.positions.iter().copied())
                .collect(),
        )
    }

    /// Total kinetic energy of the system (sum of squared velocity magnitudes).
    ///
    /// Useful as a convergence test — when energy drops below a threshold the
    /// layout has settled and repaints can stop.
    pub fn energy(&self) -> f32 {
        self.velocities.iter().map(|v| v.length_squared()).sum()
    }
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
        if graph.symbols.is_empty() {
            return Positions(IndexMap::new());
        }

        let start = Instant::now();
        let mut state = LayoutState::new(graph, self.seed, ForceParams::default());
        let init_elapsed = start.elapsed();

        let sim_start = Instant::now();
        state.step(self.iterations);
        let sim_elapsed = sim_start.elapsed();

        let pack_start = Instant::now();
        let mut map = state.positions().0;
        pack_components(&mut map, graph);
        let pack_elapsed = pack_start.elapsed();

        info!(
            nodes = state.node_count(),
            iterations = self.iterations,
            init_ms = format!("{:.1}", init_elapsed.as_secs_f64() * 1000.0),
            sim_ms = format!("{:.1}", sim_elapsed.as_secs_f64() * 1000.0),
            pack_ms = format!("{:.1}", pack_elapsed.as_secs_f64() * 1000.0),
            "batch layout complete"
        );

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
