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
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

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

/// Per-edge-kind tuneable parameters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EdgeKindParams {
    /// Rest length of springs for this edge kind.
    pub target_distance: f32,
    /// Spring attraction coefficient for this edge kind.
    pub attraction: f32,
}

/// Tuneable constants for the force-directed simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForceParams {
    /// Coulomb-like repulsion strength (all pairs).
    pub repulsion: f32,
    /// Global spring attraction coefficient (fallback for edge kinds not in
    /// [`edge_params`](Self::edge_params)).
    pub attraction: f32,
    /// Velocity damping factor applied each step (lower = more damping).
    pub damping: f32,
    /// Maximum velocity magnitude per node per step. Prevents wild overshooting.
    pub max_velocity: f32,
    /// Global rest length of edge springs (fallback).
    pub ideal_length: f32,
    /// Minimum distance clamped to avoid division by near-zero.
    pub min_dist: f32,
    /// Cutoff distance for grid-based repulsion. Pairs farther apart than this
    /// skip the expensive per-pair calculation. Set to `f32::INFINITY` to
    /// disable the optimisation and fall back to all-pairs.
    pub repulsion_cutoff: f32,
    /// Gentle pull toward the centroid of all nodes. Keeps disconnected
    /// components from drifting to infinity. Set to `0.0` to disable.
    pub gravity: f32,
    /// Per-edge-kind overrides for attraction and target distance.
    pub edge_params: HashMap<EdgeKind, EdgeKindParams>,
}

impl Default for ForceParams {
    fn default() -> Self {
        let mut edge_params = HashMap::new();
        edge_params.insert(
            EdgeKind::Contains,
            EdgeKindParams {
                target_distance: 80.0,
                attraction: 0.015,
            },
        );
        edge_params.insert(
            EdgeKind::Calls,
            EdgeKindParams {
                target_distance: 150.0,
                attraction: 0.01,
            },
        );
        edge_params.insert(
            EdgeKind::Inherits,
            EdgeKindParams {
                target_distance: 200.0,
                attraction: 0.008,
            },
        );
        edge_params.insert(
            EdgeKind::Overrides,
            EdgeKindParams {
                target_distance: 120.0,
                attraction: 0.012,
            },
        );

        Self {
            repulsion: 5000.0,
            attraction: 0.01,
            damping: 0.75,
            ideal_length: 150.0,
            max_velocity: 50.0,
            min_dist: 1.0,
            repulsion_cutoff: 500.0,
            gravity: 0.5,
            edge_params,
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
    /// Total steps run so far, used for adaptive cooling.
    total_steps: u32,
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
            total_steps: 0,
        }
    }

    /// Run `n` simulation iterations.
    pub fn step(&mut self, n: u32) {
        self.step_inner(n, false);
    }

    /// Run simulation iterations for at most `budget` wall-clock time.
    ///
    /// Returns the number of iterations completed. Always runs at least one
    /// iteration so the layout makes progress even on very large graphs.
    pub fn step_budgeted(&mut self, budget: Duration) -> u32 {
        let deadline = Instant::now() + budget;
        let mut count = 0u32;
        loop {
            self.step_inner(1, false);
            count += 1;
            if Instant::now() >= deadline {
                break;
            }
        }
        count
    }

    /// Run up to `n` iterations, optionally stopping early when energy is low.
    fn step_inner(&mut self, n: u32, early_stop: bool) {
        /// Energy threshold below which batch layout stops early.
        const EARLY_STOP_ENERGY: f32 = 0.5;
        /// Threshold below which we use serial computation (rayon overhead not
        /// worth it for small graphs).
        const PARALLEL_THRESHOLD: usize = 500;

        let len = self.positions.len();
        if len == 0 {
            return;
        }
        let p = &self.params;

        for _ in 0..n {
            if early_stop && self.energy() < EARLY_STOP_ENERGY {
                break;
            }

            // --- Repulsive forces via grid-based cutoff ---
            let cutoff = p.repulsion_cutoff;
            let cutoff_sq = cutoff * cutoff;
            let inv_cutoff = 1.0 / cutoff;
            let repulsion = p.repulsion;
            let min_dist = p.min_dist;

            // Build spatial grid: assign each node to a cell.
            let cell_keys: Vec<(i32, i32)> = self
                .positions
                .iter()
                .map(|pos| {
                    let cx = (pos.x * inv_cutoff).floor() as i32;
                    let cy = (pos.y * inv_cutoff).floor() as i32;
                    (cx, cy)
                })
                .collect();

            let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::with_capacity(len / 4 + 1);
            for (i, &key) in cell_keys.iter().enumerate() {
                grid.entry(key).or_default().push(i);
            }

            // Compute repulsive forces per-node in parallel.
            // Each node accumulates its own force by scanning its 3×3
            // neighbourhood, avoiding the need for atomic writes.
            let positions_ref = &self.positions;
            let grid_ref = &grid;

            let repulsive_forces: Vec<Vec2> = if len >= PARALLEL_THRESHOLD {
                (0..len)
                    .into_par_iter()
                    .map(|i| {
                        compute_repulsion_for_node(
                            i,
                            positions_ref,
                            grid_ref,
                            &cell_keys,
                            cutoff_sq,
                            inv_cutoff,
                            repulsion,
                            min_dist,
                        )
                    })
                    .collect()
            } else {
                (0..len)
                    .map(|i| {
                        compute_repulsion_for_node(
                            i,
                            positions_ref,
                            grid_ref,
                            &cell_keys,
                            cutoff_sq,
                            inv_cutoff,
                            repulsion,
                            min_dist,
                        )
                    })
                    .collect()
            };

            let mut forces = repulsive_forces;

            // Attractive forces along visible edges only (per-edge-kind params)
            for &(from, to, kind) in &self.edge_pairs {
                if !self.visible_edge_kinds.contains(&kind) {
                    continue;
                }
                let (attr, rest_len) = if let Some(ep) = p.edge_params.get(&kind) {
                    (ep.attraction, ep.target_distance)
                } else {
                    (p.attraction, p.ideal_length)
                };
                let delta = self.positions[to] - self.positions[from];
                let dist = delta.length().max(p.min_dist);
                let force = delta.normalize_or_zero() * attr * (dist - rest_len);
                forces[from] += force;
                forces[to] -= force;
            }

            // Gravity: gentle pull toward the centroid
            if p.gravity > 0.0 {
                let centroid = self.positions.iter().copied().sum::<Vec2>() / len as f32;
                for (force, pos) in forces.iter_mut().zip(self.positions.iter()) {
                    *force += (centroid - *pos) * p.gravity;
                }
            }

            // Adaptive cooling: damping decreases over time so the layout
            // progressively freezes into place.
            let cooling = (1.0 - (self.total_steps as f32 / 300.0).min(0.95)).max(0.05);
            let effective_damping = p.damping * cooling;
            let max_vel = p.max_velocity * cooling;
            self.total_steps += 1;

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
                    *vel = (*vel + *force) * effective_damping;
                    // Clamp velocity to prevent overshooting.
                    let speed = vel.length();
                    if speed > max_vel {
                        *vel *= max_vel / speed;
                    }
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
    ///
    /// Partially resets the cooling schedule so neighbours can re-settle
    /// around the new position.
    pub fn unpin(&mut self, id: SymbolId) {
        self.pins.shift_remove(&id);
        // Roll back cooling partially so the layout can re-settle.
        self.total_steps = self.total_steps.saturating_sub(60);
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

    /// Shared reference to the current force parameters.
    pub fn params(&self) -> &ForceParams {
        &self.params
    }

    /// Mutable reference to the force parameters for live-tweaking.
    pub fn params_mut(&mut self) -> &mut ForceParams {
        &mut self.params
    }

    /// Partially reset the cooling schedule so parameter changes take visible
    /// effect. Call after modifying [`ForceParams`] via [`params_mut`](Self::params_mut).
    pub fn reheat(&mut self) {
        self.total_steps = self.total_steps.saturating_sub(100);
    }
}

/// Compute repulsive force on node `i` from its 3×3 grid neighbourhood.
///
/// This is a per-node computation suitable for parallel execution — each
/// call reads shared positions but writes only to its own output.
#[allow(clippy::too_many_arguments)]
fn compute_repulsion_for_node(
    i: usize,
    positions: &[Vec2],
    grid: &HashMap<(i32, i32), Vec<usize>>,
    cell_keys: &[(i32, i32)],
    cutoff_sq: f32,
    inv_cutoff: f32,
    repulsion: f32,
    min_dist: f32,
) -> Vec2 {
    let _ = inv_cutoff; // used for grid key computation at call site
    let pos_i = positions[i];
    let (cx, cy) = cell_keys[i];
    let mut force = Vec2::ZERO;

    // Scan 3×3 neighbourhood (including own cell)
    for dx in -1..=1i32 {
        for dy in -1..=1i32 {
            let nx = cx.wrapping_add(dx);
            let ny = cy.wrapping_add(dy);
            if let Some(cell) = grid.get(&(nx, ny)) {
                for &j in cell {
                    if j == i {
                        continue;
                    }
                    let delta = pos_i - positions[j];
                    let dist_sq = delta.length_squared();
                    if dist_sq > cutoff_sq || dist_sq < 1e-10 {
                        continue;
                    }
                    let dist = dist_sq.sqrt().max(min_dist);
                    force += delta.normalize_or_zero() * (repulsion / (dist * dist));
                }
            }
        }
    }
    force
}

/// Force-directed layout with grid-based repulsion approximation.
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

        let mut state = LayoutState::new(graph, self.seed, ForceParams::default());
        state.step_inner(self.iterations, true);

        let mut map = state.positions().0;
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
