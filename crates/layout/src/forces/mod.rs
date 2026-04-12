//! Composable force pipeline for the force-directed layout simulation.
//!
//! Each force type implements the [`Force`] trait and accumulates
//! contributions into a shared `forces` buffer during [`Force::apply`].
//! [`ForceContext`] bundles the read-only simulation state that every force
//! needs each step — it is rebuilt cheaply on each tick from borrowed slices
//! and references, so forces always see the current positions, sizes,
//! visibility, and topology.
//!
//! Forces are progressively extracted from [`crate::LayoutState::step_inner`]
//! into this module. Each extraction step leaves the simulation's overall
//! behavior unchanged — the inline computation is replaced with a call into
//! the pipeline. See issue #33 for the full migration plan.

use core_ir::EdgeKind;
use glam::Vec2;
use std::any::Any;
use std::collections::HashSet;

pub mod containment;
pub mod gravity;
pub mod location_affinity;
pub mod repulsion;
pub mod spring_attraction;

pub use containment::Containment;
pub use gravity::Gravity;
pub use location_affinity::LocationAffinity;
pub use repulsion::Repulsion;
pub use spring_attraction::SpringAttraction;

/// Threshold (in nodes) above which forces may parallelise their inner
/// loops under rayon. For smaller graphs, the overhead of spawning work
/// onto the thread pool outweighs the savings, so forces fall back to a
/// serial implementation.
pub(crate) const PARALLEL_THRESHOLD: usize = 500;

/// Read-only snapshot of simulation state shared with every [`Force`] during
/// a single simulation step.
///
/// A `ForceContext` is reconstructed at the start of each step from borrowed
/// slices and references owned by [`crate::LayoutState`], so it is cheap to
/// build and forces always observe the current state.
///
/// Forces must not attempt to mutate any of these fields; mutation happens
/// through the `forces` buffer passed to [`Force::apply`] and is integrated
/// into positions/velocities by [`crate::LayoutState`] after all forces have
/// run.
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
    /// and participates in force computation this step. Hidden nodes
    /// (collapsed descendants, file-tree hidden) are `false`.
    pub active: &'a [bool],
    /// Edge topology as `(from_idx, to_idx, kind)` triples.
    pub edge_pairs: &'a [(usize, usize, EdgeKind)],
    /// Edge kinds currently toggled on in the UI. Forces that spring along
    /// edges should skip pairs whose kind is not in this list.
    pub visible_edge_kinds: &'a [EdgeKind],
    /// Container-hierarchy children lookup: `children_of[i]` is the list of
    /// node indices directly contained by node `i` (empty if `i` is not a
    /// container).
    pub children_of: &'a [Vec<usize>],
    /// Set of node indices that are containers (have ≥1 `Contains` child).
    pub containers: &'a HashSet<usize>,
    /// Set of container node indices currently expanded (children visible).
    pub expanded: &'a HashSet<usize>,
    /// Containers that live at the top level of the hierarchy (namespaces,
    /// translation units). These typically receive a reduced containment
    /// strength.
    pub toplevel_containers: &'a HashSet<usize>,
    /// Total number of nodes in the simulation (length of `positions`,
    /// `sizes`, `degrees`, `active`).
    pub node_count: usize,
}

/// A composable force in the layout simulation.
///
/// Forces are held by [`crate::LayoutState`] as a `Vec<Box<dyn Force>>` and
/// run in order each step. Every enabled force gets a mutable view of the
/// shared `forces` accumulator and a read-only [`ForceContext`]; after all
/// forces run, the accumulated contributions are integrated into velocities
/// and positions.
///
/// Implementors must be `Send + Sync` so the pipeline can eventually fan out
/// force application across rayon threads. Within a single `apply` call,
/// forces read only the shared context and write only into their slice of
/// the `forces` buffer — no interior mutability required.
///
/// The `Any` super-trait allows [`crate::LayoutState`] to downcast boxed
/// forces back to their concrete type. This is used internally to sync
/// parameters from the legacy [`crate::ForceParams`] struct, and will later
/// back the `force::<T>()` / `force_mut::<T>()` typed accessors used by the
/// viz crate (see issue #33, Phase 3).
pub trait Force: Any + Send + Sync {
    /// Short human-readable identifier, used for UI headers and debug logs.
    fn name(&self) -> &str;

    /// Whether this force is currently active. A disabled force is skipped
    /// entirely by the pipeline and contributes nothing to `forces`.
    fn enabled(&self) -> bool;

    /// Toggle this force on or off.
    fn set_enabled(&mut self, enabled: bool);

    /// Accumulate this force's contribution into `forces[i]` for each active
    /// node `i`. Implementors must:
    ///
    /// - Only write to indices where `ctx.active[i]` is `true`.
    /// - Add to `forces[i]` rather than overwrite — multiple forces stack.
    /// - Avoid reading or writing outside `0..ctx.node_count`.
    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]);

    /// Upcast to `&dyn Any` so callers can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Upcast to `&mut dyn Any` so callers can downcast to the concrete type.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
