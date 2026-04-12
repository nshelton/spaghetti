//! Graph layout algorithms.
//!
//! Pure function from [`core_ir::Graph`] → [`Positions`]. No rendering dependencies.
//!
//! The main types are:
//! - [`ForceDirected`]: batch layout (compute once, get positions).
//! - [`LayoutState`]: incremental simulation driven frame-by-frame, with
//!   support for pinning nodes (interactive dragging).

pub mod forces;

use core_ir::{EdgeKind, Graph, SymbolId, SymbolKind};
use glam::Vec2;
use indexmap::IndexMap;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// Half-size of collapsed container box for position clamping.
/// Children are constrained within this box around the container center.
const COLLAPSED_HALF_SIZE: Vec2 = Vec2::new(80.0, 50.0);

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
    /// Whether repulsion forces are enabled.
    #[serde(default = "default_true")]
    pub repulsion_enabled: bool,
    /// Whether edge spring attraction forces are enabled.
    #[serde(default = "default_true")]
    pub attraction_enabled: bool,
    /// Whether gravity (pull toward centroid) is enabled.
    #[serde(default = "default_true")]
    pub gravity_enabled: bool,
    /// Whether location-affinity forces are enabled.
    #[serde(default = "default_true")]
    pub location_enabled: bool,
    /// Whether containment forces are enabled.
    #[serde(default = "default_true")]
    pub containment_enabled: bool,
    /// Whether container overlap (gap-based) repulsion is enabled.
    #[serde(default = "default_true")]
    pub container_repulsion_enabled: bool,
    /// Strength of gap-based repulsion between container nodes.
    #[serde(default = "default_container_repulsion")]
    pub container_repulsion: f32,
}

fn default_container_repulsion() -> f32 {
    300.0
}

fn default_true() -> bool {
    true
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
            repulsion_enabled: true,
            attraction_enabled: true,
            gravity_enabled: true,
            location_enabled: true,
            containment_enabled: true,
            container_repulsion_enabled: true,
            container_repulsion: default_container_repulsion(),
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
    /// Per-node visibility flag. `active[i] == true` means node `i`
    /// participates in the simulation this step. Rebuilt from
    /// [`Self::collapse_hidden`] and [`Self::external_hidden`] by
    /// [`Self::rebuild_active`].
    active: Vec<bool>,
    /// Maps each node index to its parent container index (via Contains edges).
    parent_of: Vec<Option<usize>>,
    /// Maps each container node index to its children indices.
    children_of: Vec<Vec<usize>>,
    /// Set of node indices that are containers (have >=1 Contains-child).
    containers: HashSet<usize>,
    /// Containers that are top-level (Namespace or TranslationUnit).
    /// These get reduced containment strength.
    toplevel_containers: HashSet<usize>,
    /// Which containers are currently expanded (children visible).
    expanded: HashSet<usize>,
    /// Per-node collapse-hidden flag. Nodes marked `true` are currently
    /// hidden by a collapsed ancestor container (tracked separately from
    /// file-tree hidden).
    collapse_hidden: Vec<bool>,
    /// Per-node external-hidden flag. Nodes marked `true` are currently
    /// hidden by an external caller (e.g. the file-tree visibility panel).
    external_hidden: Vec<bool>,
    /// Per-node bounding box sizes in world units. Used for size-aware
    /// repulsion (edge-to-edge distance instead of center-to-center).
    sizes: Vec<Vec2>,
    /// Composable force pipeline run each step. Every enabled force
    /// accumulates contributions into a shared buffer that is then
    /// integrated into velocities and positions.
    forces: Vec<Box<dyn forces::Force>>,
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

        // Identify top-level containers (Namespace, TranslationUnit).
        let mut toplevel_containers = HashSet::new();
        for &c in &containers {
            let sym_id = ids[c];
            if let Some(sym) = graph.symbols.get(&sym_id) {
                if matches!(
                    sym.kind,
                    SymbolKind::Namespace | SymbolKind::TranslationUnit
                ) {
                    toplevel_containers.insert(c);
                }
            }
        }

        // Build sibling groups: containers grouped by their shared parent.
        // Top-level containers (no parent) form one group; children of each
        // container that are themselves containers form another group.
        let sibling_groups = build_sibling_groups(&containers, &parent_of);

        // Default: all containers collapsed. Children remain in the
        // simulation (visible, participate in forces) but are clamped
        // inside the collapsed container box each step.
        let expanded = HashSet::new();

        // Visibility bookkeeping: both "hidden" sources start empty, so
        // every node is active at construction time.
        let collapse_hidden = vec![false; n];
        let external_hidden = vec![false; n];
        let active = vec![true; n];

        let force_pipeline: Vec<Box<dyn forces::Force>> = vec![
            Box::new(forces::Repulsion::new(
                params.repulsion,
                params.repulsion_cutoff,
                params.min_dist,
                params.repulsion_enabled,
            )),
            Box::new(forces::SpringAttraction::new(
                params.attraction,
                params.ideal_length,
                params.min_dist,
                params.edge_params.clone(),
                params.attraction_enabled,
            )),
            Box::new(forces::Containment::new(
                params.containment_strength,
                params.containment_enabled,
            )),
            Box::new(forces::ContainerRepulsion::new(
                params.container_repulsion,
                sibling_groups,
                params.container_repulsion_enabled,
            )),
            Box::new(forces::LocationAffinity::new(
                params.location_strength,
                params.location_falloff,
                dir_groups,
                max_dir_depth,
                params.location_enabled,
            )),
            Box::new(forces::Gravity::new(params.gravity, params.gravity_enabled)),
        ];

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
            active,
            parent_of,
            children_of,
            containers,
            toplevel_containers,
            expanded,
            collapse_hidden,
            external_hidden,
            sizes: vec![Vec2::new(120.0, 30.0); n],
            forces: force_pipeline,
        }
    }

    /// Borrow the pipeline force of type `T`, if present. Each force
    /// type is unique in the pipeline, so the result effectively
    /// identifies "the" force of that type.
    pub fn force<T: forces::Force>(&self) -> Option<&T> {
        self.forces
            .iter()
            .find_map(|f| f.as_any().downcast_ref::<T>())
    }

    /// Mutably borrow the pipeline force of type `T`, if present.
    pub fn force_mut<T: forces::Force>(&mut self) -> Option<&mut T> {
        self.forces
            .iter_mut()
            .find_map(|f| f.as_any_mut().downcast_mut::<T>())
    }

    /// Copy parameter values from [`ForceParams`] into each pipeline
    /// force. Runs once per `step_inner` call so UI-driven param edits
    /// take effect immediately without plumbing a setter per field.
    fn sync_forces_from_params(&mut self) {
        let p = self.params.clone();
        for force in self.forces.iter_mut() {
            let any = force.as_any_mut();
            if let Some(g) = any.downcast_mut::<forces::Gravity>() {
                g.strength = p.gravity;
                g.enabled = p.gravity_enabled;
                continue;
            }
            if let Some(r) = any.downcast_mut::<forces::Repulsion>() {
                r.strength = p.repulsion;
                r.cutoff = p.repulsion_cutoff;
                r.min_dist = p.min_dist;
                r.enabled = p.repulsion_enabled;
                continue;
            }
            if let Some(s) = any.downcast_mut::<forces::SpringAttraction>() {
                s.global_attraction = p.attraction;
                s.global_ideal_length = p.ideal_length;
                s.min_dist = p.min_dist;
                s.enabled = p.attraction_enabled;
                s.edge_params.clone_from(&p.edge_params);
                continue;
            }
            if let Some(l) = any.downcast_mut::<forces::LocationAffinity>() {
                l.strength = p.location_strength;
                l.falloff = p.location_falloff;
                l.enabled = p.location_enabled;
                continue;
            }
            if let Some(c) = any.downcast_mut::<forces::Containment>() {
                c.strength = p.containment_strength;
                c.enabled = p.containment_enabled;
                continue;
            }
            if let Some(cr) = any.downcast_mut::<forces::ContainerRepulsion>() {
                cr.strength = p.container_repulsion;
                cr.enabled = p.container_repulsion_enabled;
                continue;
            }
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

        let len = self.positions.len();
        if len == 0 {
            return;
        }

        self.sync_forces_from_params();

        for _ in 0..n {
            if early_stop && self.energy() < EARLY_STOP_ENERGY {
                break;
            }

            // Accumulate all forces into a fresh buffer.
            let mut forces = vec![Vec2::ZERO; len];
            let ctx = forces::ForceContext {
                positions: &self.positions,
                sizes: &self.sizes,
                degrees: &self.degrees,
                active: &self.active,
                edge_pairs: &self.edge_pairs,
                visible_edge_kinds: &self.visible_edge_kinds,
                children_of: &self.children_of,
                containers: &self.containers,
                expanded: &self.expanded,
                toplevel_containers: &self.toplevel_containers,
                node_count: len,
            };
            for force in &self.forces {
                if force.enabled() {
                    force.apply(&ctx, &mut forces);
                }
            }

            // Integrate velocity/position, then clamp descendants of
            // collapsed containers inside their parent's bounding box.
            self.integrate(&forces);
            self.clamp_collapsed();
        }
    }

    /// Update velocities and positions from an accumulated force buffer.
    ///
    /// Applies adaptive cooling (damping decays over time), clamps per-node
    /// force and velocity magnitudes, skips hidden nodes, and holds pinned
    /// nodes at their fixed positions. Increments `total_steps`.
    ///
    /// Above [`forces::PARALLEL_THRESHOLD`] nodes the per-node update runs
    /// under rayon: each iteration writes exactly one position/velocity
    /// slot, so the work parallelises without contention.
    fn integrate(&mut self, forces: &[Vec2]) {
        let cooling = (1.0 - (self.total_steps as f32 / 300.0).min(0.95)).max(0.05);
        let effective_damping = self.params.damping * cooling;
        let max_vel = self.params.max_velocity * cooling;
        let max_force = self.params.max_velocity * 2.0;
        self.total_steps += 1;

        let n = self.positions.len();
        if n == 0 {
            return;
        }

        // Split `self` into disjoint field borrows so the parallel loop
        // can hold mutable borrows on positions/velocities while sharing
        // `active`, `ids`, and `pins` across worker threads.
        let positions = &mut self.positions;
        let velocities = &mut self.velocities;
        let active: &[bool] = &self.active;
        let ids: &[SymbolId] = &self.ids;
        let pins = &self.pins;

        // Per-node integration step, shared between the serial and
        // parallel paths so the math stays identical.
        let integrate_one = |i: usize, pos: &mut Vec2, vel: &mut Vec2, force: Vec2| {
            if !active[i] {
                *vel = Vec2::ZERO;
                return;
            }
            if let Some(&pin_pos) = pins.get(&ids[i]) {
                *pos = pin_pos;
                *vel = Vec2::ZERO;
                return;
            }
            // Clamp force magnitude to prevent explosive acceleration
            // when nodes are very close or container overlaps are large.
            let mut f = force;
            let f_mag = f.length();
            if f_mag > max_force {
                f *= max_force / f_mag;
            }
            *vel = (*vel + f) * effective_damping;
            let speed = vel.length();
            if speed > max_vel {
                *vel *= max_vel / speed;
            }
            *pos += *vel;
        };

        if n >= forces::PARALLEL_THRESHOLD {
            positions
                .par_iter_mut()
                .zip(velocities.par_iter_mut())
                .zip(forces.par_iter())
                .enumerate()
                .for_each(|(i, ((pos, vel), force))| {
                    integrate_one(i, pos, vel, *force);
                });
        } else {
            for (i, ((pos, vel), force)) in positions
                .iter_mut()
                .zip(velocities.iter_mut())
                .zip(forces.iter())
                .enumerate()
            {
                integrate_one(i, pos, vel, *force);
            }
        }
    }

    /// Clamp every descendant of a collapsed (non-expanded) container
    /// inside a fixed-size box around the container's current position.
    /// Expanded containers let their descendants move freely.
    fn clamp_collapsed(&mut self) {
        for &c in &self.containers {
            if self.expanded.contains(&c) || !self.active[c] {
                continue;
            }
            let center = self.positions[c];
            for &d in &self.all_descendants_idx(c) {
                if !self.active[d] {
                    continue;
                }
                let pos = &mut self.positions[d];
                pos.x = pos.x.clamp(
                    center.x - COLLAPSED_HALF_SIZE.x,
                    center.x + COLLAPSED_HALF_SIZE.x,
                );
                pos.y = pos.y.clamp(
                    center.y - COLLAPSED_HALF_SIZE.y,
                    center.y + COLLAPSED_HALF_SIZE.y,
                );
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
        for slot in self.external_hidden.iter_mut() {
            *slot = false;
        }
        for id in ids {
            if let Some(&idx) = self.id_to_idx.get(id) {
                self.external_hidden[idx] = true;
            }
        }
        self.rebuild_active();
        self.reheat();
    }

    /// Rebuild the [`Self::active`] flag from the collapse + external
    /// hidden sources. A node is active iff neither mechanism has marked
    /// it hidden.
    fn rebuild_active(&mut self) {
        for (i, active) in self.active.iter_mut().enumerate() {
            *active = !self.collapse_hidden[i] && !self.external_hidden[i];
        }
    }

    /// Collapse a container node: all descendants stay in the simulation
    /// but are clamped inside a fixed-size box around the parent each step.
    pub fn collapse(&mut self, id: SymbolId) {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return;
        };
        if !self.containers.contains(&idx) {
            return;
        }
        self.expanded.remove(&idx);
        // Gather all descendants close to parent so they animate into the box.
        let parent_pos = self.positions[idx];
        let descendants = self.all_descendants_idx(idx);
        for (i, &d) in descendants.iter().enumerate() {
            let angle = (i as f32) * std::f32::consts::TAU / descendants.len().max(1) as f32;
            let offset = Vec2::new(angle.cos(), angle.sin()) * 20.0;
            self.positions[d] = parent_pos + offset;
            self.velocities[d] = Vec2::ZERO;
        }
        self.reheat();
    }

    /// Expand a container node: all descendants are scattered outward and
    /// the dynamic bounding-box rendering takes over.
    pub fn expand(&mut self, id: SymbolId) {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return;
        };
        if !self.containers.contains(&idx) {
            return;
        }
        self.expanded.insert(idx);
        let parent_pos = self.positions[idx];
        let descendants = self.all_descendants_idx(idx);
        for (i, &d) in descendants.iter().enumerate() {
            let angle = (i as f32) * std::f32::consts::TAU / descendants.len().max(1) as f32;
            let offset = Vec2::new(angle.cos(), angle.sin()) * 50.0;
            self.positions[d] = parent_pos + offset;
            self.velocities[d] = Vec2::ZERO;
        }
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

    /// Return the direct children of a container node.
    pub fn children_of(&self, id: SymbolId) -> Vec<SymbolId> {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return Vec::new();
        };
        self.children_of[idx].iter().map(|&i| self.ids[i]).collect()
    }

    /// Return ALL descendants of a container (children, grandchildren, etc.).
    pub fn all_descendants(&self, id: SymbolId) -> Vec<SymbolId> {
        let Some(&idx) = self.id_to_idx.get(&id) else {
            return Vec::new();
        };
        self.all_descendants_idx(idx)
            .iter()
            .map(|&i| self.ids[i])
            .collect()
    }

    /// Internal: collect all descendant indices recursively.
    fn all_descendants_idx(&self, idx: usize) -> Vec<usize> {
        let mut result = Vec::new();
        let mut stack = self.children_of[idx].clone();
        while let Some(child) = stack.pop() {
            result.push(child);
            stack.extend_from_slice(&self.children_of[child]);
        }
        result
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

    /// Update per-node bounding box sizes for size-aware repulsion.
    pub fn set_sizes(&mut self, sizes: &[(SymbolId, Vec2)]) {
        for &(id, size) in sizes {
            if let Some(&idx) = self.id_to_idx.get(&id) {
                self.sizes[idx] = size;
            }
        }
    }

    /// Return all symbol IDs that are currently hidden due to collapsed containers.
    ///
    /// With the new collapse model (children stay visible, clamped inside
    /// a fixed box), this always returns an empty list.
    pub fn collapsed_hidden_ids(&self) -> Vec<SymbolId> {
        Vec::new()
    }
}

/// Build sibling groups: containers grouped by their shared parent.
/// Top-level containers (`parent_of[c] == None`) form one group; children of
/// each container that are themselves containers form another group.
fn build_sibling_groups(
    containers: &HashSet<usize>,
    parent_of: &[Option<usize>],
) -> Vec<Vec<usize>> {
    let mut by_parent: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for &c in containers {
        by_parent.entry(parent_of[c]).or_default().push(c);
    }
    // Only keep groups with at least 2 siblings (single containers can't repel).
    by_parent.into_values().filter(|g| g.len() >= 2).collect()
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
