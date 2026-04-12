//! Container-overlap resolution force.
//!
//! Sibling containers that currently overlap get pushed apart along the
//! axis of least penetration, with force proportional to the overlap
//! depth. The push is applied as a rigid-body translation of each
//! container plus all of its descendants, so the inside of a container
//! stays laid out relative to itself while the whole block slides.
//!
//! Containers are grouped by their shared parent (sibling groups) so
//! only direct siblings can repel each other. A [`SpatialGrid`] indexed
//! by the maximum container extent keeps per-group comparison to a 3×3
//! neighbourhood instead of `O(S²)` pairs.
//!
//! [`SpatialGrid`]: super::grid::SpatialGrid

use glam::Vec2;
use std::any::Any;

use super::grid::SpatialGrid;
use super::{Force, ForceContext};

/// Sibling-container AABB overlap resolution.
pub struct ContainerRepulsion {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Overlap-depth coefficient. The per-pair force scales linearly as
    /// `strength * overlap`.
    pub strength: f32,
    /// Sibling groups: containers grouped by their shared parent.
    /// Top-level containers form a single group.
    sibling_groups: Vec<Vec<usize>>,
}

impl ContainerRepulsion {
    /// Create a new container-repulsion force with precomputed sibling
    /// groups.
    pub fn new(strength: f32, sibling_groups: Vec<Vec<usize>>, enabled: bool) -> Self {
        Self {
            enabled,
            strength,
            sibling_groups,
        }
    }
}

impl Force for ContainerRepulsion {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        if self.strength <= 0.0 {
            return;
        }
        let cr = self.strength;

        for group in &self.sibling_groups {
            let live_siblings: Vec<usize> = group
                .iter()
                .copied()
                .filter(|&c| ctx.active[c] && ctx.expanded.contains(&c))
                .collect();
            if live_siblings.len() < 2 {
                continue;
            }

            // Cell size = largest container extent so overlapping pairs
            // are always in the same or adjacent cells.
            let max_extent = live_siblings
                .iter()
                .map(|&c| ctx.sizes[c].x.max(ctx.sizes[c].y))
                .fold(0.0f32, f32::max);
            let cell_size = max_extent.max(1.0);
            let grid = SpatialGrid::build(
                cell_size,
                live_siblings.iter().map(|&i| (i, ctx.positions[i])),
            );

            for &a in &live_siblings {
                let pos_a = ctx.positions[a];
                let half_a = ctx.sizes[a] * 0.5;
                let query_cell = grid.cell_of(pos_a);

                grid.for_each_in_neighborhood(query_cell, |b| {
                    // Avoid self-pairs and double-counting.
                    if b <= a {
                        return;
                    }
                    let pos_b = ctx.positions[b];
                    let half_b = ctx.sizes[b] * 0.5;

                    let overlap_x = (half_a.x + half_b.x) - (pos_a.x - pos_b.x).abs();
                    let overlap_y = (half_a.y + half_b.y) - (pos_a.y - pos_b.y).abs();

                    if overlap_x <= 0.0 || overlap_y <= 0.0 {
                        return;
                    }

                    // Push apart along the axis of least overlap
                    // (minimum penetration direction).
                    let delta = pos_a - pos_b;
                    let f = if overlap_x < overlap_y {
                        Vec2::new(delta.x.signum() * overlap_x * cr, 0.0)
                    } else {
                        Vec2::new(0.0, delta.y.signum() * overlap_y * cr)
                    };

                    apply_force_to_subtree(a, f, forces, ctx.children_of, ctx.active);
                    apply_force_to_subtree(b, -f, forces, ctx.children_of, ctx.active);
                });
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

/// Apply a force to a node and all its descendants (rigid-body
/// translation). Descendants whose `active` flag is `false` are skipped.
fn apply_force_to_subtree(
    root: usize,
    force: Vec2,
    forces: &mut [Vec2],
    children_of: &[Vec<usize>],
    active: &[bool],
) {
    forces[root] += force;
    let mut stack: Vec<usize> = children_of[root].clone();
    while let Some(node) = stack.pop() {
        if active[node] {
            forces[node] += force;
            stack.extend_from_slice(&children_of[node]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forces::test_utils::TestCtx;

    #[test]
    fn non_overlapping_siblings_unaffected() {
        // Two sibling containers 100 apart with half-size 10 — no overlap.
        let mut tc = TestCtx::new(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)]);
        tc.sizes = vec![Vec2::new(20.0, 20.0); 2];
        tc.containers.insert(0);
        tc.containers.insert(1);
        tc.expanded.insert(0);
        tc.expanded.insert(1);

        let mut forces = vec![Vec2::ZERO; 2];
        ContainerRepulsion::new(1.0, vec![vec![0, 1]], true).apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn overlapping_siblings_pushed_apart() {
        // Two expanded containers overlapping along the x axis.
        let mut tc = TestCtx::new(vec![Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0)]);
        tc.sizes = vec![Vec2::new(20.0, 20.0); 2];
        tc.containers.insert(0);
        tc.containers.insert(1);
        tc.expanded.insert(0);
        tc.expanded.insert(1);

        let mut forces = vec![Vec2::ZERO; 2];
        ContainerRepulsion::new(1.0, vec![vec![0, 1]], true).apply(&tc.view(), &mut forces);

        assert!(forces[0].x < 0.0);
        assert!(forces[1].x > 0.0);
        assert!((forces[0] + forces[1]).length() < 1e-4);
    }

    #[test]
    fn collapsed_siblings_do_not_participate() {
        // Same overlapping setup, but neither container is expanded.
        let mut tc = TestCtx::new(vec![Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0)]);
        tc.sizes = vec![Vec2::new(20.0, 20.0); 2];
        tc.containers.insert(0);
        tc.containers.insert(1);

        let mut forces = vec![Vec2::ZERO; 2];
        ContainerRepulsion::new(1.0, vec![vec![0, 1]], true).apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn force_applied_to_container_and_descendants() {
        // Container 0 has child 2; container 1 has child 3. Overlapping
        // siblings should have their subtrees translated rigidly.
        let mut tc = TestCtx::new(vec![
            Vec2::new(-5.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(-5.0, 0.0),
            Vec2::new(5.0, 0.0),
        ]);
        tc.sizes = vec![Vec2::new(20.0, 20.0); 4];
        tc.children_of[0] = vec![2];
        tc.children_of[1] = vec![3];
        tc.containers.insert(0);
        tc.containers.insert(1);
        tc.expanded.insert(0);
        tc.expanded.insert(1);

        let mut forces = vec![Vec2::ZERO; 4];
        ContainerRepulsion::new(1.0, vec![vec![0, 1]], true).apply(&tc.view(), &mut forces);

        // Container and its child get identical force vectors (rigid body).
        assert_eq!(forces[0], forces[2]);
        assert_eq!(forces[1], forces[3]);
        assert!(forces[0].x < 0.0);
        assert!(forces[1].x > 0.0);
    }
}
