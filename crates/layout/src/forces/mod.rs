//! Composable force pipeline for the force-directed layout simulation.
//!
//! Each force type implements the [`Force`] trait and accumulates
//! contributions into a shared `forces` buffer during [`Force::apply`].
//! [`ForceContext`] bundles the read-only simulation state every force
//! needs each step — it is rebuilt cheaply on each tick from borrowed
//! slices and references, so forces always see the current positions,
//! sizes, visibility, and topology.

use core_ir::EdgeKind;
use glam::Vec2;
use std::any::Any;
use std::collections::HashSet;

pub mod container_repulsion;
pub mod containment;
pub mod gravity;
pub mod location_affinity;
pub mod repulsion;
pub mod spring_attraction;

pub use container_repulsion::ContainerRepulsion;
pub use containment::Containment;
pub use gravity::Gravity;
pub use location_affinity::LocationAffinity;
pub use repulsion::Repulsion;
pub use spring_attraction::SpringAttraction;

/// Threshold (in nodes) above which forces may parallelise their inner
/// loops under rayon. Below this, the thread-pool overhead outweighs
/// the savings and forces fall back to serial execution.
pub(crate) const PARALLEL_THRESHOLD: usize = 500;

/// Read-only snapshot of simulation state shared with every [`Force`]
/// during a single simulation step.
#[non_exhaustive]
pub struct ForceContext<'a> {
    /// Current world-space positions, indexed by node index.
    pub positions: &'a [Vec2],
    /// Per-node bounding-box sizes in world units. Used by size-aware
    /// repulsion (edge-to-edge rather than center-to-center).
    pub sizes: &'a [Vec2],
    /// Per-node edge degree, used to normalize spring forces on hub nodes.
    pub degrees: &'a [f32],
    /// Per-node active flag. `active[i] == true` means node `i` is visible
    /// and participates in force computation this step.
    pub active: &'a [bool],
    /// Edge topology as `(from_idx, to_idx, kind)` triples.
    pub edge_pairs: &'a [(usize, usize, EdgeKind)],
    /// Edge kinds currently toggled on in the UI.
    pub visible_edge_kinds: &'a [EdgeKind],
    /// `children_of[i]` is the list of node indices directly contained by
    /// node `i` (empty if `i` is not a container).
    pub children_of: &'a [Vec<usize>],
    /// Set of node indices that are containers (have ≥1 `Contains` child).
    pub containers: &'a HashSet<usize>,
    /// Container indices currently expanded (children visible).
    pub expanded: &'a HashSet<usize>,
    /// Containers at the top of the hierarchy (namespaces, translation
    /// units). These typically get a reduced containment strength.
    pub toplevel_containers: &'a HashSet<usize>,
    /// Total number of nodes (length of `positions`, `sizes`, `degrees`,
    /// `active`).
    pub node_count: usize,
}

/// A composable force in the layout simulation.
///
/// Forces are held by [`crate::LayoutState`] as `Vec<Box<dyn Force>>` and
/// run in order each step. Each enabled force accumulates contributions
/// into a shared `forces` buffer, which is integrated into velocities
/// and positions after all forces have run.
///
/// Implementors must be `Send + Sync` so the pipeline can fan out across
/// rayon threads. The `Any` super-trait lets callers downcast boxed
/// forces back to their concrete type to read or mutate tunable fields.
pub trait Force: Any + Send + Sync {
    /// Whether this force is currently active. A disabled force is
    /// skipped by the pipeline and contributes nothing.
    fn enabled(&self) -> bool;

    /// Toggle this force on or off.
    fn set_enabled(&mut self, enabled: bool);

    /// Accumulate this force's contribution into `forces[i]` for each
    /// active node `i`. Implementors must only write to indices where
    /// `ctx.active[i]` is `true`, add to `forces[i]` rather than
    /// overwrite, and avoid reading or writing outside
    /// `0..ctx.node_count`.
    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]);

    /// Upcast to `&dyn Any` for downcasting to the concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Upcast to `&mut dyn Any` for downcasting to the concrete type.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[cfg(test)]
pub(crate) mod test_utils {
    //! Shared constructors for force unit tests.
    //!
    //! Most force tests only care about a few slices (positions, active,
    //! perhaps edges or children). [`TestCtx`] owns everything a
    //! [`ForceContext`] needs to borrow so tests can build one in a few
    //! lines instead of repeating the full field list.
    use super::*;

    /// Owned backing storage for a [`ForceContext`]. Populate the fields
    /// you care about, leave the rest at their defaults, then call
    /// [`TestCtx::view`] to get a borrowed `ForceContext`.
    pub struct TestCtx {
        pub positions: Vec<Vec2>,
        pub active: Vec<bool>,
        pub sizes: Vec<Vec2>,
        pub degrees: Vec<f32>,
        pub edge_pairs: Vec<(usize, usize, EdgeKind)>,
        pub visible_edge_kinds: Vec<EdgeKind>,
        pub children_of: Vec<Vec<usize>>,
        pub containers: HashSet<usize>,
        pub expanded: HashSet<usize>,
        pub toplevel_containers: HashSet<usize>,
    }

    impl TestCtx {
        /// New test context for `positions` with every node active.
        /// All other fields start empty; callers set what they need.
        pub fn new(positions: Vec<Vec2>) -> Self {
            let n = positions.len();
            Self {
                positions,
                active: vec![true; n],
                sizes: vec![Vec2::ZERO; n],
                degrees: vec![1.0; n],
                edge_pairs: Vec::new(),
                visible_edge_kinds: Vec::new(),
                children_of: vec![Vec::new(); n],
                containers: HashSet::new(),
                expanded: HashSet::new(),
                toplevel_containers: HashSet::new(),
            }
        }

        /// Borrow all fields as a [`ForceContext`].
        pub fn view(&self) -> ForceContext<'_> {
            ForceContext {
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
                node_count: self.positions.len(),
            }
        }
    }
}
