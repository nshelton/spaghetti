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
use std::collections::{HashMap, HashSet, VecDeque};
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
    /// Strength of the location-affinity force that attracts nodes sharing
    /// filesystem directories. `0.0` disables the force.
    #[serde(default = "default_location_strength")]
    pub location_strength: f32,
    /// Per-depth-level decay for the location force. A value of `0.5` means
    /// parent-directory attraction is half of same-directory attraction.
    #[serde(default = "default_location_falloff")]
    pub location_falloff: f32,
    /// Strength of the containment force that pulls children of an expanded
    /// container toward their siblings' centroid. `0.0` disables.
    #[serde(default = "default_containment_strength")]
    pub containment_strength: f32,
}

fn default_location_strength() -> f32 {
    0.3
}

fn default_location_falloff() -> f32 {
    0.5
}

fn default_containment_strength() -> f32 {
    0.02
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
        edge_params.insert(
            EdgeKind::ReadsField,
            EdgeKindParams {
                target_distance: 100.0,
                attraction: 0.008,
            },
        );
        edge_params.insert(
            EdgeKind::WritesField,
            EdgeKindParams {
                target_distance: 100.0,
                attraction: 0.008,
            },
        );
        edge_params.insert(
            EdgeKind::Includes,
            EdgeKindParams {
                target_distance: 180.0,
                attraction: 0.006,
            },
        );
        edge_params.insert(
            EdgeKind::Instantiates,
            EdgeKindParams {
                target_distance: 160.0,
                attraction: 0.008,
            },
        );
        edge_params.insert(
            EdgeKind::HasType,
            EdgeKindParams {
                target_distance: 120.0,
                attraction: 0.006,
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
            location_strength: default_location_strength(),
            location_falloff: default_location_falloff(),
            containment_strength: default_containment_strength(),
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
    /// Per-node edge degree (number of edges touching this node).
    /// Used to normalize spring forces on high-degree hubs.
    degrees: Vec<f32>,
    /// Total steps run so far, used for adaptive cooling.
    total_steps: u32,
    /// Node indices that are hidden (excluded from all force computation).
    hidden: HashSet<usize>,
    /// Hierarchical directory groups for the location-affinity force.
    /// `dir_groups[depth][group_idx]` is a list of node indices sharing
    /// the same directory prefix at that depth.
    dir_groups: Vec<Vec<Vec<usize>>>,
    /// Maximum directory depth across all nodes (cached for force scaling).
    max_dir_depth: usize,
    /// Maps each node index to its parent container index (via Contains edges).
    parent_of: Vec<Option<usize>>,
    /// Maps each container node index to its children indices.
    children_of: Vec<Vec<usize>>,
    /// Set of node indices that are containers (have >=1 Contains-child).
    containers: HashSet<usize>,
    /// Which containers are currently expanded (children visible).
    expanded: HashSet<usize>,
    /// Nodes hidden by collapse (tracked separately from file-tree hidden).
    collapse_hidden: HashSet<usize>,
    /// Nodes hidden by external callers (file-tree visibility).
    external_hidden: HashSet<usize>,
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
            EdgeKind::ReadsField,
            EdgeKind::WritesField,
            EdgeKind::Includes,
            EdgeKind::Instantiates,
            EdgeKind::HasType,
        ];

        let velocities = vec![Vec2::ZERO; n];

        // Precompute per-node degree for force normalization.
        let mut degrees = vec![1.0f32; n];
        for &(from, to, _) in &edge_pairs {
            degrees[from] += 1.0;
            degrees[to] += 1.0;
        }

        // Build hierarchical directory groups from symbol file locations.
        let dir_groups = build_dir_groups(graph, &ids, &id_to_idx);
        let max_dir_depth = dir_groups.len().saturating_sub(1);

        // Build containment hierarchy from Contains edges.
        let mut parent_of = vec![None; n];
        let mut children_of = vec![Vec::new(); n];
        let mut containers = HashSet::new();
        for &(from, to, kind) in &edge_pairs {
            if kind == EdgeKind::Contains {
                parent_of[to] = Some(from);
                children_of[from].push(to);
                containers.insert(from);
            }
        }

        // Default: all containers collapsed. Hide their children.
        let expanded = HashSet::new();
        let mut collapse_hidden = HashSet::new();
        for &c in &containers {
            for &child in &children_of[c] {
                collapse_hidden.insert(child);
            }
        }

        // Combine collapse-hidden into the main hidden set.
        let hidden = collapse_hidden.clone();

        Self {
            ids,
            positions,
            velocities,
            edge_pairs,
            visible_edge_kinds,
            id_to_idx,
            pins: IndexMap::new(),
            params,
            degrees,
            total_steps: 0,
            hidden,
            dir_groups,
            max_dir_depth,
            parent_of,
            children_of,
            containers,
            expanded,
            collapse_hidden,
            external_hidden: HashSet::new(),
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

            // Build spatial grid: assign each node to a cell (skip hidden).
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
                if !self.hidden.contains(&i) {
                    grid.entry(key).or_default().push(i);
                }
            }

            // Compute repulsive forces per-node in parallel.
            // Each node accumulates its own force by scanning its 3×3
            // neighbourhood, avoiding the need for atomic writes.
            let positions_ref = &self.positions;
            let grid_ref = &grid;

            let hidden_ref = &self.hidden;
            let repulsive_forces: Vec<Vec2> = if len >= PARALLEL_THRESHOLD {
                (0..len)
                    .into_par_iter()
                    .map(|i| {
                        if hidden_ref.contains(&i) {
                            return Vec2::ZERO;
                        }
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
                        if hidden_ref.contains(&i) {
                            return Vec2::ZERO;
                        }
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

            // Attractive forces along visible edges only (per-edge-kind params).
            //
            // Linear spring, but each endpoint's force is divided by
            // sqrt(degree) so hub nodes (many connections) don't get
            // yanked across the canvas by the sum of all their edges.
            for &(from, to, kind) in &self.edge_pairs {
                if !self.visible_edge_kinds.contains(&kind) {
                    continue;
                }
                if self.hidden.contains(&from) || self.hidden.contains(&to) {
                    continue;
                }
                let (attr, rest_len) = if let Some(ep) = p.edge_params.get(&kind) {
                    (ep.attraction, ep.target_distance)
                } else {
                    (p.attraction, p.ideal_length)
                };
                let delta = self.positions[to] - self.positions[from];
                let dist = delta.length().max(p.min_dist);
                let displacement = attr * (dist - rest_len);
                let dir = delta.normalize_or_zero();
                // Scale by 1/sqrt(degree) at each endpoint so hubs stay calm.
                forces[from] += dir * displacement / self.degrees[from].sqrt();
                forces[to] -= dir * displacement / self.degrees[to].sqrt();
            }

            // Containment force: for expanded containers, pull children
            // toward their siblings' centroid so they stay clustered.
            if p.containment_strength > 0.0 {
                for &c in &self.containers {
                    if !self.expanded.contains(&c) {
                        continue;
                    }
                    let children = &self.children_of[c];
                    if children.len() < 2 {
                        continue;
                    }
                    // Compute centroid of visible children.
                    let mut centroid = Vec2::ZERO;
                    let mut count = 0u32;
                    for &child in children {
                        if !self.hidden.contains(&child) {
                            centroid += self.positions[child];
                            count += 1;
                        }
                    }
                    if count < 2 {
                        continue;
                    }
                    centroid /= count as f32;

                    for &child in children {
                        if !self.hidden.contains(&child) {
                            forces[child] +=
                                (centroid - self.positions[child]) * p.containment_strength;
                        }
                    }
                }
            }

            // Location-affinity force: pull nodes toward their directory
            // group centroids at each depth level.
            if p.location_strength > 0.0 && !self.dir_groups.is_empty() {
                let max_d = self.max_dir_depth;
                for (depth, groups_at_depth) in self.dir_groups.iter().enumerate() {
                    // Deeper = more specific = stronger. Scale so the deepest
                    // level gets full strength and shallower levels decay.
                    let level_scale = if max_d > 0 {
                        p.location_falloff.powi((max_d - depth) as i32)
                    } else {
                        1.0
                    };
                    let strength = p.location_strength * level_scale;
                    if strength < 1e-6 {
                        continue;
                    }

                    for group in groups_at_depth {
                        // Compute centroid of visible nodes in this group.
                        let mut centroid = Vec2::ZERO;
                        let mut count = 0u32;
                        for &idx in group {
                            if !self.hidden.contains(&idx) {
                                centroid += self.positions[idx];
                                count += 1;
                            }
                        }
                        if count < 2 {
                            continue;
                        }
                        centroid /= count as f32;

                        for &idx in group {
                            if !self.hidden.contains(&idx) {
                                forces[idx] += (centroid - self.positions[idx]) * strength;
                            }
                        }
                    }
                }
            }

            // Gravity: gentle pull toward the centroid (skip hidden)
            if p.gravity > 0.0 {
                let mut centroid = Vec2::ZERO;
                let mut visible_count = 0u32;
                for (i, pos) in self.positions.iter().enumerate() {
                    if !self.hidden.contains(&i) {
                        centroid += *pos;
                        visible_count += 1;
                    }
                }
                if visible_count > 0 {
                    centroid /= visible_count as f32;
                    for (i, (force, pos)) in
                        forces.iter_mut().zip(self.positions.iter()).enumerate()
                    {
                        if !self.hidden.contains(&i) {
                            *force += (centroid - *pos) * p.gravity;
                        }
                    }
                }
            }

            // Adaptive cooling: damping decreases over time so the layout
            // progressively freezes into place.
            let cooling = (1.0 - (self.total_steps as f32 / 300.0).min(0.95)).max(0.05);
            let effective_damping = p.damping * cooling;
            let max_vel = p.max_velocity * cooling;
            self.total_steps += 1;

            // Update velocities and positions (skip pinned and hidden nodes)
            for (i, (((id, pos), vel), force)) in self
                .ids
                .iter()
                .zip(self.positions.iter_mut())
                .zip(self.velocities.iter_mut())
                .zip(forces.iter())
                .enumerate()
            {
                if self.hidden.contains(&i) {
                    *vel = Vec2::ZERO;
                    continue;
                }
                if let Some(&pin_pos) = self.pins.get(id) {
                    *pos = pin_pos;
                    *vel = Vec2::ZERO;
                } else {
                    *vel = (*vel + *force) * effective_damping;
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

    /// Randomize all node positions and zero velocities, then reheat.
    /// Uses the current timestamp as a seed so each reset looks different.
    pub fn randomize(&mut self) {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345);
        let n = self.positions.len();
        for (i, pos) in self.positions.iter_mut().enumerate() {
            let hash = seed.wrapping_mul(i as u64 + 1).wrapping_add(0x9E37_79B9);
            let x = ((hash & 0xFFFF) as f32 / 65535.0 - 0.5) * 400.0;
            let y = (((hash >> 16) & 0xFFFF) as f32 / 65535.0 - 0.5) * 400.0;
            *pos = Vec2::new(x, y);
        }
        for v in &mut self.velocities {
            *v = Vec2::ZERO;
        }
        self.total_steps = 0;
        let _ = n;
    }

    /// Slightly perturb all node positions by a small random offset, then reheat.
    /// Unlike [`randomize`](Self::randomize), this preserves the overall layout
    /// shape — useful for nudging the simulation out of a local minimum.
    pub fn juggle(&mut self) {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345);
        let amount = 30.0_f32;
        for (i, pos) in self.positions.iter_mut().enumerate() {
            let hash = seed.wrapping_mul(i as u64 + 1).wrapping_add(0x9E37_79B9);
            let dx = ((hash & 0xFFFF) as f32 / 65535.0 - 0.5) * amount;
            let dy = (((hash >> 16) & 0xFFFF) as f32 / 65535.0 - 0.5) * amount;
            *pos += Vec2::new(dx, dy);
        }
        self.total_steps = self.total_steps.saturating_sub(100);
    }

    /// Set which symbols are hidden from the simulation (by external callers,
    /// e.g. file-tree visibility toggles).
    ///
    /// Hidden nodes are excluded from all force computations (repulsion,
    /// attraction, gravity, location affinity) and their positions/velocities
    /// are not updated. They still occupy their slot so indices remain stable.
    ///
    /// This merges with collapse-hidden state — nodes hidden by either
    /// mechanism are excluded.
    pub fn set_hidden(&mut self, ids: &[SymbolId]) {
        self.external_hidden.clear();
        for id in ids {
            if let Some(&idx) = self.id_to_idx.get(id) {
                self.external_hidden.insert(idx);
            }
        }
        self.rebuild_hidden();
        self.reheat();
    }

    /// Rebuild the effective hidden set from collapse + external sources.
    fn rebuild_hidden(&mut self) {
        self.hidden = &self.collapse_hidden | &self.external_hidden;
    }

    /// Collapse a container node: hide all its children.
    ///
    /// Children's positions are moved to the parent's position so they
    /// animate outward on subsequent expand.
    pub fn collapse(&mut self, id: SymbolId) {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return;
        };
        if !self.containers.contains(&idx) {
            return;
        }
        self.expanded.remove(&idx);
        let parent_pos = self.positions[idx];
        for &child in &self.children_of[idx] {
            self.collapse_hidden.insert(child);
            self.positions[child] = parent_pos;
            self.velocities[child] = Vec2::ZERO;
        }
        self.rebuild_hidden();
        self.reheat();
    }

    /// Expand a container node: show all its children.
    ///
    /// Children are scattered in a small radius around the parent so they
    /// animate into place.
    pub fn expand(&mut self, id: SymbolId) {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return;
        };
        if !self.containers.contains(&idx) {
            return;
        }
        self.expanded.insert(idx);
        let parent_pos = self.positions[idx];
        let children = self.children_of[idx].clone();
        for (i, &child) in children.iter().enumerate() {
            self.collapse_hidden.remove(&child);
            // Scatter children in a circle around the parent.
            let angle = (i as f32) * std::f32::consts::TAU / children.len().max(1) as f32;
            let offset = Vec2::new(angle.cos(), angle.sin()) * 50.0;
            self.positions[child] = parent_pos + offset;
            self.velocities[child] = Vec2::ZERO;
        }
        self.rebuild_hidden();
        self.reheat();
    }

    /// Toggle a container between collapsed and expanded.
    pub fn toggle_expand(&mut self, id: SymbolId) {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return;
        };
        if self.expanded.contains(&idx) {
            self.collapse(id);
        } else {
            self.expand(id);
        }
    }

    /// Check whether a container is currently expanded.
    pub fn is_expanded(&self, id: SymbolId) -> bool {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return false;
        };
        self.expanded.contains(&idx)
    }

    /// Check whether a symbol is a container (has children via Contains edges).
    pub fn is_container(&self, id: SymbolId) -> bool {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return false;
        };
        self.containers.contains(&idx)
    }

    /// Return the children of a container node.
    pub fn children_of(&self, id: SymbolId) -> Vec<SymbolId> {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return Vec::new();
        };
        self.children_of[idx].iter().map(|&i| self.ids[i]).collect()
    }

    /// Return the parent container of a node, if any.
    pub fn parent_of(&self, id: SymbolId) -> Option<SymbolId> {
        let &idx = self.id_to_idx.get(&id)?;
        self.parent_of[idx].map(|p| self.ids[p])
    }

    /// Collapse all container nodes.
    pub fn collapse_all(&mut self) {
        let container_ids: Vec<SymbolId> =
            self.containers.iter().map(|&idx| self.ids[idx]).collect();
        for id in container_ids {
            self.collapse(id);
        }
    }

    /// Expand all container nodes.
    pub fn expand_all(&mut self) {
        let container_ids: Vec<SymbolId> =
            self.containers.iter().map(|&idx| self.ids[idx]).collect();
        for id in container_ids {
            self.expand(id);
        }
    }

    /// Return all symbol IDs that are currently hidden due to collapsed containers.
    pub fn collapsed_hidden_ids(&self) -> Vec<SymbolId> {
        self.collapse_hidden
            .iter()
            .map(|&idx| self.ids[idx])
            .collect()
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

/// Build hierarchical directory groups from the graph's file table.
///
/// Returns `dir_groups[depth][group_idx]` = list of node indices sharing
/// the same directory prefix at that depth. For example, if nodes A and B
/// are in `src/shapes/` and node C is in `src/util/`, then at depth 0
/// (`src`) all three are grouped, and at depth 1 A,B are in one group
/// and C in another.
fn build_dir_groups(
    graph: &Graph,
    ids: &[SymbolId],
    id_to_idx: &IndexMap<SymbolId, usize>,
) -> Vec<Vec<Vec<usize>>> {
    // For each node, extract directory path components.
    let mut node_dir_components: Vec<Option<Vec<String>>> = vec![None; ids.len()];
    let mut max_depth: usize = 0;

    for (id, sym) in &graph.symbols {
        let Some(&idx) = id_to_idx.get(id) else {
            continue;
        };
        let Some(loc) = &sym.location else {
            continue;
        };
        let Some(path_str) = graph.files.resolve(loc.file) else {
            continue;
        };
        // Skip external (absolute) paths.
        if path_str.starts_with('/') {
            continue;
        }
        let path = std::path::Path::new(path_str);
        let components: Vec<String> = path
            .parent()
            .unwrap_or(std::path::Path::new(""))
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if !components.is_empty() {
            max_depth = max_depth.max(components.len());
            node_dir_components[idx] = Some(components);
        }
    }

    if max_depth == 0 {
        return Vec::new();
    }

    // Build groups at each depth level.
    let mut dir_groups: Vec<Vec<Vec<usize>>> = Vec::with_capacity(max_depth);

    for depth in 0..max_depth {
        let mut groups_map: HashMap<Vec<&str>, Vec<usize>> = HashMap::new();
        for (idx, comps) in node_dir_components.iter().enumerate() {
            if let Some(comps) = comps {
                if comps.len() > depth {
                    let prefix: Vec<&str> = comps[..=depth].iter().map(|s| s.as_str()).collect();
                    groups_map.entry(prefix).or_default().push(idx);
                }
            }
        }
        // Only keep groups with 2+ members (singletons don't need forces).
        let groups: Vec<Vec<usize>> = groups_map.into_values().filter(|g| g.len() >= 2).collect();
        dir_groups.push(groups);
    }

    dir_groups
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
            if let Some(p) = positions.get_mut(&id) {
                p.x += offset_x;
                p.y += offset_y;
            }
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
        if let Some(v) = visited.get_mut(&id) {
            *v = true;
        }

        while let Some(current) = queue.pop_front() {
            component.push(current);
            if let Some(neighbors) = adj.get(&current) {
                for &neighbor in neighbors {
                    if !visited[&neighbor] {
                        if let Some(v) = visited.get_mut(&neighbor) {
                            *v = true;
                        }
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        components.push(component);
    }

    components
}
