//! Directory-affinity force: cluster nodes that share filesystem
//! directories.
//!
//! Nodes whose source locations share a directory prefix at some depth
//! are pulled toward the centroid of their group. Deeper (more specific)
//! prefixes get the full strength; shallower levels decay by
//! `falloff.powi(depth)` so same-directory attraction dominates over
//! same-grandparent.

use glam::Vec2;
use std::any::Any;

use super::{Force, ForceContext};

/// Directory-affinity clustering force.
pub struct LocationAffinity {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Base strength at the deepest directory level.
    pub strength: f32,
    /// Per-level decay. `0.5` means each level up halves the attraction.
    pub falloff: f32,
    /// `dir_groups[depth][group_idx]` is a list of node indices sharing
    /// the same directory prefix at `depth`. Groups with fewer than two
    /// members are not stored (a singleton group produces no force).
    dir_groups: Vec<Vec<Vec<usize>>>,
    /// Maximum directory depth across all nodes (cached for force scaling).
    max_dir_depth: usize,
}

impl LocationAffinity {
    /// Create a new location-affinity force from precomputed directory
    /// groups. `max_dir_depth` should equal
    /// `dir_groups.len().saturating_sub(1)` — the caller typically
    /// computes this alongside the groups themselves.
    pub fn new(
        strength: f32,
        falloff: f32,
        dir_groups: Vec<Vec<Vec<usize>>>,
        max_dir_depth: usize,
        enabled: bool,
    ) -> Self {
        Self {
            enabled,
            strength,
            falloff,
            dir_groups,
            max_dir_depth,
        }
    }
}

impl Force for LocationAffinity {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        if self.strength <= 0.0 || self.dir_groups.is_empty() {
            return;
        }
        let max_d = self.max_dir_depth;
        for (depth, groups_at_depth) in self.dir_groups.iter().enumerate() {
            // Deeper = more specific = stronger. Scale so the deepest
            // level gets full strength and shallower levels decay.
            let level_scale = if max_d > 0 {
                self.falloff.powi((max_d - depth) as i32)
            } else {
                1.0
            };
            let strength = self.strength * level_scale;
            if strength < 1e-6 {
                continue;
            }

            for group in groups_at_depth {
                let mut centroid = Vec2::ZERO;
                let mut count = 0u32;
                for &idx in group {
                    if ctx.active[idx] {
                        centroid += ctx.positions[idx];
                        count += 1;
                    }
                }
                if count < 2 {
                    continue;
                }
                centroid /= count as f32;

                for &idx in group {
                    if ctx.active[idx] {
                        forces[idx] += (centroid - ctx.positions[idx]) * strength;
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forces::test_utils::TestCtx;

    #[test]
    fn empty_groups_is_noop() {
        let tc = TestCtx::new(vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)]);
        let mut forces = vec![Vec2::ZERO; 2];
        LocationAffinity::new(1.0, 0.5, vec![], 0, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn nodes_in_same_group_pulled_to_centroid() {
        let tc = TestCtx::new(vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)]);
        let dir_groups = vec![vec![vec![0usize, 1usize]]];
        let mut forces = vec![Vec2::ZERO; 2];
        // max_dir_depth = 0, so level_scale = 1.0 and strength = 0.5
        // applies in full. Nodes pulled toward centroid (origin).
        LocationAffinity::new(0.5, 0.5, dir_groups, 0, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces[0], Vec2::new(-5.0, 0.0));
        assert_eq!(forces[1], Vec2::new(5.0, 0.0));
    }

    #[test]
    fn hidden_nodes_ignored_in_centroid_and_application() {
        // Three nodes, node 0 is hidden. Centroid comes from 1 and 2
        // only, and node 0 receives no force despite being in the group.
        let mut tc = TestCtx::new(vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(-10.0, 0.0),
        ]);
        tc.active[0] = false;

        let dir_groups = vec![vec![vec![0usize, 1usize, 2usize]]];
        let mut forces = vec![Vec2::ZERO; 3];
        LocationAffinity::new(1.0, 0.5, dir_groups, 0, true).apply(&tc.view(), &mut forces);

        assert_eq!(forces[0], Vec2::ZERO);
        assert_eq!(forces[1], Vec2::new(-10.0, 0.0));
        assert_eq!(forces[2], Vec2::new(10.0, 0.0));
    }

    #[test]
    fn falloff_weakens_shallow_levels() {
        // One group at depth 0 with max depth = 1. The shallow level
        // gets strength * falloff^1.
        let tc = TestCtx::new(vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)]);
        let dir_groups = vec![vec![vec![0usize, 1usize]], vec![]];
        let mut forces = vec![Vec2::ZERO; 2];
        LocationAffinity::new(1.0, 0.5, dir_groups, 1, true).apply(&tc.view(), &mut forces);

        // Level scale at depth 0 with max_d=1 is 0.5^1 = 0.5.
        // Expected force: 0.5 * (centroid - position) = 0.5 * (-10, 0).
        assert_eq!(forces[0], Vec2::new(-5.0, 0.0));
        assert_eq!(forces[1], Vec2::new(5.0, 0.0));
    }
}
