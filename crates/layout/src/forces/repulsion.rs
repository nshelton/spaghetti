//! Grid-based Coulomb repulsion between visible nodes.
//!
//! Every active node pushes every other active node away with a force
//! that falls off as `strength / dist²`. Instead of an O(N²) all-pairs
//! loop, a spatial grid with cell size equal to `cutoff` restricts the
//! per-node work to a 3×3 neighbourhood. Pairs farther than `cutoff`
//! contribute nothing anyway, so the grid makes the cost linear in
//! practice.
//!
//! On large graphs (≥ [`PARALLEL_THRESHOLD`] nodes) the per-node inner
//! loop runs under rayon with `par_iter_mut` directly over the output
//! slice — each node writes one slot, so there are no write conflicts.

use glam::Vec2;
use rayon::prelude::*;
use std::any::Any;

use super::grid::SpatialGrid;
use super::{Force, ForceContext, PARALLEL_THRESHOLD};

/// Grid-based point-Coulomb repulsion.
pub struct Repulsion {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Coulomb-style coefficient applied per interacting pair.
    pub strength: f32,
    /// Maximum distance considered for repulsion. Pairs farther than
    /// this are skipped. Also sets the spatial grid cell size.
    pub cutoff: f32,
    /// Floor for pairwise distance to avoid `1 / dist²` blow-ups when
    /// two nodes are nearly coincident.
    pub min_dist: f32,
}

impl Repulsion {
    /// Create a new repulsion force with the given parameters.
    pub fn new(strength: f32, cutoff: f32, min_dist: f32, enabled: bool) -> Self {
        Self {
            enabled,
            strength,
            cutoff,
            min_dist,
        }
    }
}

impl Force for Repulsion {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        if self.strength == 0.0 || self.cutoff <= 0.0 {
            return;
        }

        let cutoff = self.cutoff;
        let cutoff_sq = cutoff * cutoff;
        let strength = self.strength;
        let min_dist = self.min_dist;
        let len = ctx.node_count;

        // Bucket active nodes into a spatial grid keyed by `cutoff`.
        let grid = SpatialGrid::build(
            cutoff,
            ctx.positions.iter().enumerate().filter_map(|(i, &pos)| {
                if ctx.active[i] {
                    Some((i, pos))
                } else {
                    None
                }
            }),
        );

        let positions = ctx.positions;
        let active = ctx.active;
        let grid_ref = &grid;

        // Per-node: each iteration writes a single output slot, so the
        // work parallelises without contention.
        if len >= PARALLEL_THRESHOLD {
            forces
                .par_iter_mut()
                .enumerate()
                .take(len)
                .for_each(|(i, force)| {
                    if !active[i] {
                        return;
                    }
                    *force += compute_repulsion_for_node(
                        i, positions, grid_ref, cutoff_sq, strength, min_dist,
                    );
                });
        } else {
            for (i, force) in forces.iter_mut().enumerate().take(len) {
                if !active[i] {
                    continue;
                }
                *force += compute_repulsion_for_node(
                    i, positions, grid_ref, cutoff_sq, strength, min_dist,
                );
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Point-Coulomb repulsive force on node `i` from the 3×3 cell
/// neighbourhood of its current position. Center-to-center distance,
/// no size awareness.
fn compute_repulsion_for_node(
    i: usize,
    positions: &[Vec2],
    grid: &SpatialGrid,
    cutoff_sq: f32,
    strength: f32,
    min_dist: f32,
) -> Vec2 {
    let pos_i = positions[i];
    let query_cell = grid.cell_of(pos_i);
    let mut force = Vec2::ZERO;

    grid.for_each_in_neighborhood(query_cell, |j| {
        if j == i {
            return;
        }
        let delta = pos_i - positions[j];
        let dist_sq = delta.length_squared();
        if dist_sq > cutoff_sq || dist_sq < 1e-10 {
            return;
        }
        let dist = dist_sq.sqrt().max(min_dist);
        force += delta.normalize_or_zero() * (strength / (dist * dist));
    });
    force
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forces::test_utils::TestCtx;

    #[test]
    fn disabled_strength_is_noop() {
        let tc = TestCtx::new(vec![Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)]);
        let mut forces = vec![Vec2::ZERO; 2];
        Repulsion::new(0.0, 500.0, 1.0, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn two_nodes_push_apart_symmetrically() {
        let tc = TestCtx::new(vec![Vec2::new(5.0, 0.0), Vec2::new(-5.0, 0.0)]);
        let mut forces = vec![Vec2::ZERO; 2];
        Repulsion::new(1000.0, 500.0, 1.0, true).apply(&tc.view(), &mut forces);
        // Node 0 at +x gets pushed in +x; node 1 at -x gets pushed in -x.
        assert!(forces[0].x > 0.0 && forces[0].y == 0.0);
        assert!(forces[1].x < 0.0 && forces[1].y == 0.0);
        // Newton's third law.
        assert!((forces[0] + forces[1]).length() < 1e-4);
    }

    #[test]
    fn hidden_nodes_do_not_interact() {
        let mut tc = TestCtx::new(vec![
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(-1.0, 0.0),
        ]);
        tc.active[1] = false;
        let mut forces = vec![Vec2::ZERO; 3];
        Repulsion::new(1000.0, 500.0, 1.0, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces[1], Vec2::ZERO, "hidden node received force");
        assert!(forces[0].x > 0.0);
        assert!(forces[2].x < 0.0);
    }

    #[test]
    fn cutoff_respected() {
        // Two nodes farther apart than the cutoff should feel no force.
        let tc = TestCtx::new(vec![Vec2::new(10000.0, 0.0), Vec2::new(-10000.0, 0.0)]);
        let mut forces = vec![Vec2::ZERO; 2];
        Repulsion::new(1000.0, 500.0, 1.0, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }
}
